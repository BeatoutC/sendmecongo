//! Whatever has to finish before the QR window can open.
//!
//! Reading, compressing and packaging a multi-megabyte file takes seconds, and
//! brotli at quality 11 is the whole of it. Doing that on the UI thread — which is
//! what used to happen — freezes the window hard enough that the operator assumes
//! it has died. So it runs here instead, on one worker thread, reporting progress
//! back through atomics and honouring a cancel flag that is checked between
//! compression chunks.
//!
//! The output is the SMC1 container itself. The GUI hands those exact bytes to the
//! player process over stdin, so the file is compressed once per session rather
//! than once per click.

use sendmecongo_core::{compress, container, Error};
use sendmecongo_ui::i18n::{fill, t, Text};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

/// Refuse anything larger than this.
///
/// Two reasons, both measured. The optical link tops out around 100 KB/s, so 64 MB
/// is already ten minutes of filming; and the prepare stage holds the original plus
/// the compressed copies plus the container at the same time, which is roughly four
/// to five times the file size in peak memory.
pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Reported by a worker that was told to stop.
pub const CANCELLED: &str = "__cancelled__";

/// Coarse phase of the job. The slice of the bar each one owns is proportional to
/// how long it actually takes: brotli is ~1.3 MB/s while gzip is ~50 MB/s and the
/// CRC over the container costs almost nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Read,
    Gzip,
    Brotli,
    Package,
}

impl Phase {
    fn from_byte(n: u8) -> Self {
        match n {
            0 => Phase::Read,
            1 => Phase::Gzip,
            2 => Phase::Brotli,
            _ => Phase::Package,
        }
    }

    /// Where this phase starts and ends on the bar, as a fraction of the whole job —
    /// the same scale [`Progress::read`] reports. The phases tile `0.0..=1.0` in order,
    /// with widths proportional to how long each one really takes.
    fn span(self) -> (f32, f32) {
        match self {
            Phase::Read => (0.0, 0.03),
            Phase::Gzip => (0.03, 0.18),
            Phase::Brotli => (0.18, 0.96),
            Phase::Package => (0.96, 1.0),
        }
    }

    /// Named for the operator, in the progress line. Takes the table rather than reading
    /// the global one so it stays usable from a test in any language, and so the progress
    /// line and its phases can never disagree about which language they are in.
    pub fn label(self, t: &Text) -> &'static str {
        match self {
            Phase::Read => t.prep_phase_read,
            Phase::Gzip => t.prep_phase_gzip,
            Phase::Brotli => t.prep_phase_brotli,
            Phase::Package => t.prep_phase_package,
        }
    }
}

/// Shared, lock-free progress. The UI reads it once per frame; the worker writes it
/// from inside the compression callbacks.
pub struct Progress {
    phase: AtomicU8,
    permille: AtomicU32,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        Self {
            phase: AtomicU8::new(0),
            permille: AtomicU32::new(0),
        }
    }

    pub fn begin(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
        let (start, _) = phase.span();
        self.bump(start * 1000.0);
    }

    pub fn set(&self, phase: Phase, fraction: f32) {
        self.phase.store(phase as u8, Ordering::Relaxed);
        let (start, end) = phase.span();
        let fraction = fraction.clamp(0.0, 1.0);
        self.bump((start + (end - start) * fraction) * 1000.0);
    }

    /// The bar never goes backwards, even if a phase is skipped and the next one
    /// jumps ahead.
    fn bump(&self, permille: f32) {
        let value = permille.clamp(0.0, 1000.0) as u32;
        self.permille.fetch_max(value, Ordering::Relaxed);
    }

    pub fn read(&self) -> (Phase, f32) {
        let phase = Phase::from_byte(self.phase.load(Ordering::Relaxed));
        (phase, self.permille.load(Ordering::Relaxed) as f32 / 1000.0)
    }
}

/// A file that is ready to play.
#[derive(Debug)]
pub struct Prepared {
    pub raw: usize,
    /// The SMC1 container. Shared rather than cloned: the stdin writer thread needs
    /// it too, and it can be tens of megabytes.
    pub object: Arc<Vec<u8>>,
    pub comp: u8,
    /// Set when brotli was deliberately skipped, phrased for the operator.
    pub note: Option<String>,
    pub secs: f64,
}

impl Prepared {
    pub fn wire(&self) -> usize {
        self.object.len()
    }

    pub fn method(&self) -> &'static str {
        match self.comp {
            container::COMP_GZIP => "gzip",
            container::COMP_BROTLI => "brotli",
            _ => t().prep_method_raw,
        }
    }

    /// K — source symbols for this container at the given symbol size. The receiver
    /// needs K' >= K of them, so this is the floor.
    pub fn symbols(&self, symbol_size: u16) -> usize {
        self.object.len().div_ceil(symbol_size as usize).max(1)
    }
}

/// A prepare job running on its own thread.
pub struct Job {
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<std::result::Result<Prepared, String>>,
    finished: bool,
}

impl Job {
    pub fn spawn(path: PathBuf) -> Self {
        let progress = Arc::new(Progress::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        let worker_progress = Arc::clone(&progress);
        let worker_cancel = Arc::clone(&cancel);
        let failure_tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("sendmecongo-prepare".to_string())
            .spawn(move || {
                let result = prepare(&path, &worker_progress, &worker_cancel);
                let _ = tx.send(result);
            });

        if let Err(e) = spawned {
            let _ = failure_tx.send(Err(fill(t().prep_thread_failed, &[&e])));
        }

        Self {
            progress,
            cancel,
            rx,
            finished: false,
        }
    }

    pub fn progress(&self) -> (Phase, f32) {
        self.progress.read()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Non-blocking. `Some` exactly once, when the job reaches a verdict.
    pub fn poll(&mut self) -> Option<std::result::Result<Prepared, String>> {
        if self.finished {
            return None;
        }
        match self.rx.try_recv() {
            Ok(result) => {
                self.finished = true;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.finished = true;
                Some(Err(t().prep_thread_died.to_string()))
            }
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // The worker checks this between chunks, so an abandoned job stops within a
        // chunk rather than burning a core until the process exits.
        self.cancel();
    }
}

/// Size gate, split out so it can be tested without creating a 64 MB file.
pub fn check_size(name: &str, bytes: u64) -> std::result::Result<(), String> {
    if bytes <= MAX_INPUT_BYTES {
        return Ok(());
    }
    Err(fill(t().prep_too_large, &[&name, &human(bytes as usize)]))
}

/// Synchronously prepare a file on the current thread without an asynchronous worker.
pub fn prepare_sync(path: &Path) -> std::result::Result<Prepared, String> {
    let progress = Progress::new();
    let cancel = AtomicBool::new(false);
    prepare(path, &progress, &cancel)
}

fn prepare(
    path: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> std::result::Result<Prepared, String> {
    let started = Instant::now();
    let name = file_name(path);

    let meta = std::fs::metadata(path).map_err(|e| fill(t().prep_meta_failed, &[&e]))?;
    check_size(&name, meta.len())?;

    progress.begin(Phase::Read);
    let data = std::fs::read(path).map_err(|e| fill(t().prep_read_failed, &[&e]))?;
    progress.set(Phase::Read, 1.0);
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.to_string());
    }

    let observer = Bridge { progress, cancel };
    let outcome = compress::best_observed(&data, &observer).map_err(|e| match e {
        Error::Cancelled => CANCELLED.to_string(),
        other => fill(t().prep_compress_failed, &[&other]),
    })?;

    progress.begin(Phase::Package);
    let object = container::encode(&name, &data, outcome.comp, &outcome.payload);
    progress.set(Phase::Package, 1.0);

    Ok(Prepared {
        raw: data.len(),
        object: Arc::new(object),
        comp: outcome.comp,
        note: outcome.skipped.as_ref().map(describe_skip),
        secs: started.elapsed().as_secs_f64(),
    })
}

/// Maps the compression layer's byte counters onto the bar, and carries the cancel
/// flag in the direction the worker can see.
struct Bridge<'a> {
    progress: &'a Progress,
    cancel: &'a AtomicBool,
}

impl compress::Observer for Bridge<'_> {
    fn advance(&self, stage: compress::Stage, done: usize, total: usize) {
        let phase = match stage {
            compress::Stage::Gzip => Phase::Gzip,
            compress::Stage::Brotli => Phase::Brotli,
        };
        self.progress.set(phase, done as f32 / total.max(1) as f32);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Explain a skipped brotli to whoever is standing in front of the screen. The
/// numbers matter: "saves 0.1 s of filming but costs 1.2 s of waiting" is the kind
/// of sentence that stops an operator from wondering why it felt fast today.
fn describe_skip(skip: &compress::Skip) -> String {
    match skip {
        compress::Skip::Incompressible { raw, best } => {
            fill(t().prep_skip_incompressible, &[&human(*raw), &human(*best)])
        }
        compress::Skip::NotWorthIt {
            predicted_gain,
            encode_secs,
            saved_secs,
        } => fill(
            t().prep_skip_not_worth,
            &[
                &human(*predicted_gain),
                &format!("{saved_secs:.1}"),
                &format!("{encode_secs:.1}"),
            ],
        ),
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("payload.bin")
        .to_string()
}

pub fn human(n: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sendmecongo_core::compress::Observer;
    use std::time::Duration;

    #[test]
    fn the_size_gate_is_inclusive_at_the_limit() {
        assert!(check_size("a.bin", 1024).is_ok());
        assert!(check_size("a.bin", MAX_INPUT_BYTES).is_ok());
        let err = check_size("大文件.zip", MAX_INPUT_BYTES + 1).unwrap_err();
        assert!(err.contains("64 MB"), "reason must state the limit: {err}");
        assert!(
            err.contains("大文件.zip"),
            "reason must name the file: {err}"
        );
        assert!(err.contains("100 KB/s"), "reason must say why: {err}");
    }

    fn small_bin() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/small.bin")
            .canonicalize()
            .expect("testdata/small.bin")
    }

    #[test]
    fn a_small_file_becomes_a_playable_container() {
        let progress = Progress::new();
        let prepared = prepare(&small_bin(), &progress, &AtomicBool::new(false)).expect("prepare");

        assert_eq!(prepared.raw, 32768);
        assert_eq!(container::peek_name(&prepared.object).unwrap(), "small.bin");
        let parsed = container::decode(&prepared.object).unwrap();
        assert_eq!(container::into_file(&parsed).unwrap().len(), 32768);
        assert_eq!(progress.read().1, 1.0, "the bar must land at 100%");
        assert!(prepared.secs >= 0.0);
    }

    /// The GUI only ever talks to a `Job`, so the thread → channel → poll seam has to
    /// work on its own, not just when `prepare` is called directly.
    #[test]
    fn a_spawned_job_reports_back_through_poll() {
        let mut job = Job::spawn(small_bin());
        for _ in 0..2000 {
            if let Some(result) = job.poll() {
                let prepared = result.expect("small file must prepare");
                assert_eq!(prepared.raw, 32768);
                assert_eq!(prepared.wire(), prepared.object.len());
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("job never reported back");
    }

    #[test]
    fn cancelling_a_job_yields_cancellation_not_a_result() {
        let mut job = Job::spawn(small_bin());
        job.cancel();
        for _ in 0..2000 {
            if let Some(result) = job.poll() {
                assert_eq!(result.unwrap_err(), CANCELLED);
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("cancelled job never reported back");
    }

    #[test]
    fn progress_never_runs_backwards() {
        let progress = Progress::new();
        progress.set(Phase::Gzip, 0.5);
        let (_, peak) = progress.read();
        progress.set(Phase::Gzip, 0.1);
        assert_eq!(progress.read().1, peak);
        progress.begin(Phase::Read);
        assert_eq!(progress.read().1, peak);
        assert_eq!(progress.read().0, Phase::Read);
    }

    /// The bar is one continuous scale, so the phases must tile it without gaps or
    /// overlaps, in the order they happen.
    #[test]
    fn the_phases_tile_the_whole_bar() {
        let order = [Phase::Read, Phase::Gzip, Phase::Brotli, Phase::Package];
        let mut cursor = 0.0;
        for phase in order {
            let (start, end) = phase.span();
            assert_eq!(
                start, cursor,
                "{phase:?} does not start where the last ended"
            );
            assert!(end > start, "{phase:?} has no width");
            cursor = end;
        }
        assert_eq!(cursor, 1.0, "the bar must end at 100%");
    }

    /// The compression layer reports per-stage byte counts; the bridge has to turn
    /// those into bar positions, or the progress bar would sit frozen while the
    /// encoder works — the exact symptom this module exists to remove.
    #[test]
    fn compression_progress_moves_the_bar() {
        let progress = Progress::new();
        let cancel = AtomicBool::new(false);
        let bridge = Bridge {
            progress: &progress,
            cancel: &cancel,
        };

        bridge.advance(compress::Stage::Gzip, 0, 100);
        let (phase, at_start) = progress.read();
        assert_eq!(phase, Phase::Gzip);
        bridge.advance(compress::Stage::Gzip, 50, 100);
        let (_, halfway) = progress.read();
        bridge.advance(compress::Stage::Gzip, 100, 100);
        let (_, at_end) = progress.read();

        assert!(
            at_start < halfway && halfway < at_end,
            "{at_start} {halfway} {at_end}"
        );
        assert!(
            at_start >= 0.03 && at_end <= 0.18,
            "must stay inside the gzip span"
        );
        assert_eq!(
            progress.read().0,
            Phase::Gzip,
            "the phase label follows along"
        );
    }

    #[test]
    fn the_bridge_carries_the_cancel_flag() {
        let progress = Progress::new();
        let cancel = AtomicBool::new(false);
        let bridge = Bridge {
            progress: &progress,
            cancel: &cancel,
        };
        assert!(!bridge.cancelled());
        cancel.store(true, Ordering::Relaxed);
        assert!(bridge.cancelled());
    }

    #[test]
    fn a_cancelled_job_reports_cancellation_rather_than_a_broken_file() {
        let progress = Progress::new();
        let cancel = AtomicBool::new(true);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/small.bin")
            .canonicalize()
            .expect("testdata/small.bin");
        assert_eq!(
            prepare(&path, &progress, &cancel).unwrap_err(),
            CANCELLED,
            "cancelling must be distinguishable from a real failure"
        );
    }

    #[test]
    fn a_missing_file_fails_with_a_readable_reason() {
        let progress = Progress::new();
        let cancel = AtomicBool::new(false);
        let err = prepare(
            &PathBuf::from("/definitely/not/here.bin"),
            &progress,
            &cancel,
        )
        .unwrap_err();
        assert!(err.contains("读不到文件"), "got: {err}");
    }
}
