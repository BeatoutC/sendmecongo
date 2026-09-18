//! The receive pipeline: video file in, original file out.
//!
//! The shape of the work dictated the shape of the code. A 46-second 4K60
//! recording is 2770 pictures, and in pure Rust those decode at ~13 fps
//! single-threaded while QR recognition runs at ~6 fps — eleven minutes of CPU
//! for something a person is standing there waiting on. Two facts make it
//! tractable:
//!
//! * **Phone streams are all-keyframe-ish.** The recordings here carry a CRA
//!   every ~55 pictures, and each one is a legal stream entry point once the
//!   parameter sets are re-emitted from `hvcC`. So the bitstream splits into
//!   independent groups of pictures that decode on separate cores.
//! * **The fountain code does not need the whole video.** Roughly `K'` distinct
//!   symbols finish the job, which on the 2 MB recording arrived a quarter of
//!   the way in. Everything past that point is waste, so completion stops the
//!   workers rather than draining the file.
//!
//! The collector is the shared [`Receiver`] from `sendmecongo-core`: the protocol
//! itself is untouched by any of this.

use crate::decode::VideoDecoder;
use crate::isobmff::{self, VideoTrack, START_CODE};
use crate::scan::Scanner;
use sendmecongo_core::{FrameHeader, Received, Receiver};
use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Live counters, shared between the workers, the progress line and the GUI.
///
/// `unique` and `target` are what make a *meaningful* progress bar possible: the
/// receiver needs roughly `K` distinct symbols, the first frame header carries
/// `K` (object length ÷ symbol size), and the collector counts how many distinct
/// ones have landed. Without them the only honest display is a spinner.
#[derive(Default)]
pub struct Counters {
    pub gops: AtomicUsize,
    pub pictures: AtomicUsize,
    pub total: AtomicUsize,
    pub codes: AtomicUsize,
    pub symbols: AtomicUsize,
    pub unique: AtomicUsize,
    pub target: AtomicUsize,
    pub rejected: AtomicUsize,
    pub decode_errors: AtomicUsize,
}

impl Counters {
    pub fn gops(&self) -> usize {
        self.gops.load(Ordering::Relaxed)
    }
    pub fn pictures(&self) -> usize {
        self.pictures.load(Ordering::Relaxed)
    }
    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }
    pub fn codes(&self) -> usize {
        self.codes.load(Ordering::Relaxed)
    }
    pub fn symbols(&self) -> usize {
        self.symbols.load(Ordering::Relaxed)
    }
    pub fn unique(&self) -> usize {
        self.unique.load(Ordering::Relaxed)
    }
    pub fn target(&self) -> usize {
        self.target.load(Ordering::Relaxed)
    }
    pub fn rejected(&self) -> usize {
        self.rejected.load(Ordering::Relaxed)
    }
    pub fn decode_errors(&self) -> usize {
        self.decode_errors.load(Ordering::Relaxed)
    }

    /// How far along the recovery is, in `0.0..=1.0`.
    ///
    /// Prefers "distinct symbols collected ÷ symbols needed" and falls back to
    /// "pictures decoded ÷ pictures in the recording" for the handful of frames
    /// before the first header has been read.
    pub fn fraction(&self) -> f32 {
        let target = self.target();
        if target > 0 {
            return (self.unique() as f32 / target as f32).clamp(0.0, 1.0);
        }
        let total = self.total();
        if total > 0 {
            return (self.pictures() as f32 / total as f32).clamp(0.0, 1.0);
        }
        0.0
    }
}

pub struct Config {
    pub threads: usize,
    /// Keep the distinct SMQ frames around so they can be dumped for the
    /// independent `sendmecongo-bench decode` cross-check.
    pub keep_symbols: bool,
    /// Shared with whoever is watching. A GUI reads these every frame; the CLI
    /// prints from them inside `run`'s progress callback.
    pub counters: Arc<Counters>,
    /// Set to make `run` give up. The collector polls this, so a job that is
    /// nowhere near finishing still stops within a tenth of a second.
    pub cancel: Arc<AtomicBool>,
}

impl Config {
    pub fn new(threads: usize, keep_symbols: bool) -> Self {
        Self {
            threads,
            keep_symbols,
            counters: Arc::new(Counters::default()),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Cancelled by the user rather than failed. Kept distinct from the other error
/// strings so a GUI can tell "you stopped it" from "it did not work".
/// Matched by the GUI to tell "you stopped it" from "it did not work", so the string is
/// the identity — it is *not* translated on the way out.
pub const CANCELLED: &str = "__cancelled__";

pub struct Outcome {
    pub received: Received,
    pub counters: Arc<Counters>,
    pub elapsed: Duration,
    pub first_header: Option<FrameHeader>,
    /// Distinct frames in arrival order; only populated when `keep_symbols`.
    pub symbols: Vec<Vec<u8>>,
}

/// One group of pictures: complete access units, each already Annex-B and
/// already carrying the parameter sets it needs.
struct Gop {
    access_units: Vec<Vec<u8>>,
}

struct Queue {
    state: Mutex<QueueState>,
    item_ready: Condvar,
    space_ready: Condvar,
    stop: AtomicBool,
    capacity: usize,
}

struct QueueState {
    items: VecDeque<Gop>,
    closed: bool,
}

impl Queue {
    fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(QueueState {
                items: VecDeque::new(),
                closed: false,
            }),
            item_ready: Condvar::new(),
            space_ready: Condvar::new(),
            stop: AtomicBool::new(false),
            capacity,
        }
    }

    fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Returns false when the consumer side has gone away or been told to stop.
    fn push(&self, gop: Gop) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.items.len() >= self.capacity && !state.closed && !self.is_stopped() {
            state = self.space_ready.wait(state).unwrap();
        }
        if state.closed || self.is_stopped() {
            return false;
        }
        state.items.push_back(gop);
        self.item_ready.notify_one();
        true
    }

    fn pop(&self) -> Option<Gop> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(gop) = state.items.pop_front() {
                self.space_ready.notify_one();
                return Some(gop);
            }
            if state.closed || self.is_stopped() {
                return None;
            }
            state = self.item_ready.wait(state).unwrap();
        }
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.item_ready.notify_all();
        self.space_ready.notify_all();
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.item_ready.notify_all();
        self.space_ready.notify_all();
    }
}

/// Turn the sample table into self-contained groups of pictures.
///
/// Each access unit is rebuilt as Annex-B, and the codec configuration out of
/// `hvcC`/`avcC` is re-emitted in front of every random access point — `hvc1`
/// keeps VPS/SPS/PPS out of band, so a worker starting at a mid-file keyframe
/// would otherwise have no way to make sense of the stream.
fn spawn_producer(
    path: &Path,
    track: &VideoTrack,
    queue: Arc<Queue>,
    counters: Arc<Counters>,
) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let codec = track.codec;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut buffer = Vec::new();

    for sample in &track.samples {
        if queue.is_stopped() {
            break;
        }
        buffer.resize(sample.size as usize, 0);
        file.seek(SeekFrom::Start(sample.offset))?;
        file.read_exact(&mut buffer)?;

        let nals = isobmff::split_sample(&buffer, track.nal_length_size);
        let starts_group = nals.iter().any(|nal| {
            isobmff::nal_type(codec, nal).is_some_and(|t| isobmff::is_random_access(codec, t))
        });

        if starts_group && !pending.is_empty() {
            let units = std::mem::take(&mut pending);
            counters.gops.fetch_add(1, Ordering::Relaxed);
            if !queue.push(Gop {
                access_units: units,
            }) {
                return Ok(());
            }
        }

        let mut access_unit = Vec::with_capacity(buffer.len() + 128);
        if starts_group {
            for set in &track.parameter_sets {
                access_unit.extend_from_slice(&START_CODE);
                access_unit.extend_from_slice(set);
            }
        }
        for nal in nals {
            access_unit.extend_from_slice(&START_CODE);
            access_unit.extend_from_slice(nal);
        }
        pending.push(access_unit);
    }

    if !pending.is_empty() && !queue.is_stopped() {
        counters.gops.fetch_add(1, Ordering::Relaxed);
        queue.push(Gop {
            access_units: pending,
        });
    }
    queue.close();
    Ok(())
}

/// Recognise the codes in one decoded picture and forward whatever it carried.
/// Returns false when the collecting side has hung up.
fn consume(
    frame: crate::decode::GrayFrame,
    scanner: &mut Scanner,
    counters: &Counters,
    symbols: &mpsc::Sender<Vec<u8>>,
    payloads: &mut Vec<Vec<u8>>,
) -> bool {
    let index = counters.pictures.fetch_add(1, Ordering::Relaxed) + 1;
    let (width, height) = (frame.width as u32, frame.height as u32);
    if width == 0 || height == 0 {
        return true;
    }
    debug_dump(&frame, index);
    payloads.clear();
    let found = scanner.scan(frame.luma, width, height, payloads);
    if found > 0 {
        counters.codes.fetch_add(found, Ordering::Relaxed);
    }
    for payload in payloads.drain(..) {
        if symbols.send(payload).is_err() {
            return false;
        }
    }
    true
}

fn spawn_worker(
    codec: isobmff::Codec,
    queue: Arc<Queue>,
    symbols: mpsc::Sender<Vec<u8>>,
    counters: Arc<Counters>,
) {
    std::thread::spawn(move || {
        let mut decoder = match VideoDecoder::new(codec) {
            Ok(d) => d,
            Err(_) => {
                counters.decode_errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        let mut scanner = Scanner::new();
        let mut frames = Vec::new();
        let mut payloads = Vec::new();

        while let Some(gop) = queue.pop() {
            for access_unit in &gop.access_units {
                if queue.is_stopped() {
                    return;
                }
                frames.clear();
                decoder.push(access_unit, &mut frames);
                for frame in frames.drain(..) {
                    if !consume(frame, &mut scanner, &counters, &symbols, &mut payloads) {
                        return;
                    }
                }
            }
            // Release pictures the reorder buffer was still holding: the next
            // group starts from scratch anyway.
            frames.clear();
            decoder.flush(&mut frames);
            for frame in frames.drain(..) {
                if !consume(frame, &mut scanner, &counters, &symbols, &mut payloads) {
                    return;
                }
            }
            counters
                .decode_errors
                .fetch_add(decoder.errors, Ordering::Relaxed);
        }
    });
}

/// Run the whole thing. `progress` is called from the collecting thread while
/// the receiver is still short of enough symbols.
pub fn run(
    path: &Path,
    track: &VideoTrack,
    config: &Config,
    mut progress: impl FnMut(&Counters),
) -> Result<Outcome, String> {
    let started = Instant::now();
    let counters = Arc::clone(&config.counters);
    counters.total.store(track.samples.len(), Ordering::Relaxed);
    let queue = Arc::new(Queue::new(config.threads.max(1) * 2));
    let (symbol_tx, symbol_rx) = mpsc::channel::<Vec<u8>>();

    for _ in 0..config.threads.max(1) {
        spawn_worker(
            track.codec,
            Arc::clone(&queue),
            symbol_tx.clone(),
            Arc::clone(&counters),
        );
    }
    drop(symbol_tx);

    let producer_queue = Arc::clone(&queue);
    let producer_counters = Arc::clone(&counters);
    let producer_path = path.to_path_buf();
    let producer_codec = track.codec;
    let parameter_sets = track.parameter_sets.clone();
    let nal_length_size = track.nal_length_size;
    let producer_samples = track.samples.clone();

    let producer = std::thread::spawn(move || {
        // A private view of the track so the thread owns everything it touches.
        let track = VideoTrack {
            codec: producer_codec,
            parameter_sets,
            nal_length_size,
            width: 0,
            height: 0,
            timescale: 0,
            duration: 0,
            samples: producer_samples,
        };
        if let Err(e) = spawn_producer(&producer_path, &track, producer_queue, producer_counters) {
            eprintln!(
                "{}",
                sendmecongo_ui::i18n::fill(sendmecongo_ui::i18n::t().cli_producer_failed, &[&e])
            );
        }
    });

    let mut receiver = Receiver::new();
    let mut received = None;
    let mut first_header: Option<FrameHeader> = None;
    let mut kept: Vec<Vec<u8>> = Vec::new();
    let mut seen: HashSet<(u8, u32)> = HashSet::new();
    let mut last_tick = Instant::now();

    loop {
        let payload = match symbol_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(payload) => payload,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Polling rather than blocking is what makes "停止" possible: the
                // collector is the only thread that knows the job is over, and it
                // must be able to notice a cancel that arrives while no symbols
                // are coming through at all.
                if config.cancel.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        counters.symbols.fetch_add(1, Ordering::Relaxed);

        if let Ok((header, _)) = sendmecongo_core::frame::parse(&payload) {
            if first_header.is_none() {
                first_header = Some(header);
                // K: exactly how many distinct symbols the object is made of, so
                // the progress bar has a real denominator from the first frame on.
                let symbols = header.object_len.div_ceil(header.symbol_size as u32) as usize;
                counters.target.store(symbols, Ordering::Relaxed);
            }
            if config.keep_symbols && seen.insert((header.sbn, header.esi)) {
                kept.push(payload.clone());
            }
        }

        if received.is_some() {
            continue; // keep draining so nobody blocks on a full channel
        }
        match receiver.push(&payload) {
            Ok(Some(done)) => {
                received = Some(done);
                queue.stop();
            }
            Ok(None) => {}
            Err(_) => {
                counters.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
        counters
            .unique
            .store(receiver.frames_used(), Ordering::Relaxed);

        if last_tick.elapsed() >= Duration::from_millis(250) {
            last_tick = Instant::now();
            progress(&counters);
        }
    }

    queue.stop();
    let _ = producer.join();

    let Some(received) = received else {
        let unique = receiver.frames_used();
        if config.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let t = sendmecongo_ui::i18n::t();
        let source = first_header
            .map(|h| h.object_len.div_ceil(h.symbol_size as u32) as usize)
            .unwrap_or(0);
        return Err(sendmecongo_ui::i18n::fill(
            t.err_symbols_short,
            &[
                &unique,
                &source,
                &counters.pictures(),
                &counters.codes(),
                &counters.rejected(),
                &counters.decode_errors(),
            ],
        ));
    };

    Ok(Outcome {
        received,
        counters,
        elapsed: started.elapsed(),
        first_header,
        symbols: kept,
    })
}

/// Suggested worker count: leave a core for the collector, and stay well clear
/// of the memory a 4K decoder's reference picture buffer needs per thread.
pub fn default_threads() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    cpus.saturating_sub(1).clamp(1, 6)
}

/// Diagnostic escape hatch for a receiver-side failure: `SENDMECONGO_DUMP_FRAME=<n>:<file.pgm>`
/// writes the n-th decoded picture as a binary PGM so the luma the scanner sees can be
/// looked at directly instead of guessed at.
fn debug_dump(frame: &crate::decode::GrayFrame, index: usize) {
    let Ok(spec) = std::env::var("SENDMECONGO_DUMP_FRAME") else {
        return;
    };
    let Some((want, path)) = spec.split_once(':') else {
        return;
    };
    if want.parse::<usize>().ok() != Some(index) {
        return;
    }
    let mut out = format!("P5\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    out.extend_from_slice(&frame.luma);
    let _ = std::fs::write(path, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_prefers_collected_symbols_over_decoded_pictures() {
        let counters = Counters::default();
        counters.total.store(1000, Ordering::Relaxed);
        counters.pictures.store(100, Ordering::Relaxed);
        assert!((counters.fraction() - 0.1).abs() < 1e-6);

        // Once K is known it wins, because that is what actually finishes the job.
        counters.target.store(200, Ordering::Relaxed);
        counters.unique.store(150, Ordering::Relaxed);
        assert!((counters.fraction() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn progress_stays_inside_zero_and_one() {
        let counters = Counters::default();
        assert_eq!(counters.fraction(), 0.0);

        counters.target.store(10, Ordering::Relaxed);
        counters.unique.store(40, Ordering::Relaxed);
        assert_eq!(counters.fraction(), 1.0);
    }
}
