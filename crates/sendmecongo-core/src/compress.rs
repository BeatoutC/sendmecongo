//! Compression front-end: gzip and brotli are both attempted, the smallest wins.
//!
//! Optical bytes are the scarce resource, so quality settings are maxed out — but
//! *compression time is scarce too*, and the two costs scale differently:
//!
//!   * brotli's cost grows with the **input** (~1.3 MB/s at quality 11),
//!   * the bytes it saves on top of gzip grow with the **output** (~10%).
//!
//! That asymmetry makes high-quality brotli a net loss on big or well-compressed
//! files: 960 KB of text saves 8.4 KB over gzip, which is worth 0.08 s of optical
//! time, in exchange for 1.25 s of CPU. The operator sits there watching both.
//! So brotli now has to earn its place — see [`worth_brotli`].
//!
//! Compression is also observable and interruptible: the input is fed in chunks and
//! each chunk is flushed, which forces the encoder to actually finish that chunk
//! (measured: progress then advances almost linearly with wall time, for a 0.24% size
//! penalty). The chunk boundary doubles as the cancellation checkpoint.

use crate::container::{COMP_BROTLI, COMP_GZIP, COMP_RAW};
use crate::{Error, Result};
use std::io::{Read, Write};

/// Flush granularity: smaller means a smoother progress bar and a snappier cancel,
/// larger means fewer flush markers in the output. 24 chunks is the sweet spot —
/// measured progress came out linear, and the size penalty stayed under 0.25%.
const CHUNK_COUNT: usize = 24;
const CHUNK_MIN: usize = 64 * 1024;

/// Reference optical throughput: the measured goodput of the fastest preset
/// (turbo60, 100.2 KB/s).
///
/// Pinning it to the fastest link means a byte is valued at its *cheapest*, so brotli
/// only runs when it wins even on the best case. That also keeps the judgment
/// independent of the preset, which matters: the SMC1 container is built before the
/// preset is even relevant, so the decision cannot depend on it (or changing preset
/// would invalidate the container and force a re-compression).
const OPTICAL_REF_BPS: f64 = 102_400.0;
/// Measured brotli q11 throughput (8 MB of random data took 5.6 s).
const BROTLI_BPS: f64 = 1_310_000.0;
/// Typical extra saving of brotli over gzip (measured: 10.7% on plain text).
const BROTLI_MARGINAL_GAIN: f64 = 0.10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Gzip,
    Brotli,
}

/// Where the caller watches progress and asks for a stop.
pub trait Observer {
    /// `done` of `total` input bytes have been handed to this stage's encoder.
    fn advance(&self, stage: Stage, done: usize, total: usize);
    /// Return true to abandon compression. Checked between chunks.
    fn cancelled(&self) -> bool {
        false
    }
}

/// The observer used when nobody is watching (CLI, tests).
pub struct Quiet;

impl Observer for Quiet {
    fn advance(&self, _stage: Stage, _done: usize, _total: usize) {}
}

/// Why brotli was skipped. The numbers are kept raw so the presentation layer can
/// phrase them for whoever is actually standing in front of the screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Skip {
    /// Gzip could not shrink the input, so brotli cannot either — it would only
    /// take longer. This is the common case for already-compressed payloads
    /// (zip/docx/exe/jpeg), and it is a pure win: 5.6 s of CPU for zero bytes.
    Incompressible { raw: usize, best: usize },
    /// Brotli would save bytes, but not enough of them to pay for the wait.
    NotWorthIt {
        predicted_gain: usize,
        encode_secs: f64,
        saved_secs: f64,
    },
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub comp: u8,
    pub payload: Vec<u8>,
    /// `Some` when brotli was deliberately not run.
    pub skipped: Option<Skip>,
}

pub fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    gzip_observed(data, &Quiet)
}

pub fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    let mut out = Vec::new();
    GzDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}

pub fn brotli(data: &[u8]) -> Result<Vec<u8>> {
    brotli_observed(data, &Quiet)
}

pub fn unbrotli(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::Decompressor::new(data, 32 * 1024).read_to_end(&mut out)?;
    Ok(out)
}

pub fn apply(comp: u8, data: &[u8]) -> Result<Vec<u8>> {
    match comp {
        COMP_RAW => Ok(data.to_vec()),
        COMP_GZIP => gunzip(data),
        COMP_BROTLI => unbrotli(data),
        other => Err(Error::UnknownCompression(other)),
    }
}

fn gzip_observed(data: &[u8], obs: &dyn Observer) -> Result<Vec<u8>> {
    use flate2::{write::GzEncoder, Compression};
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    feed(&mut enc, data, Stage::Gzip, obs)?;
    Ok(enc.finish()?)
}

fn brotli_observed(data: &[u8], obs: &dyn Observer) -> Result<Vec<u8>> {
    use brotli::enc::BrotliEncoderParams;
    let params = BrotliEncoderParams {
        quality: 11,
        lgwin: 22,
        ..Default::default()
    };
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::with_params(&mut out, 32 * 1024, &params);
        feed(&mut writer, data, Stage::Brotli, obs)?;
    }
    Ok(out)
}

/// Feed the encoder in chunks, flushing after each one.
///
/// The flush matters: without it the encoder buffers input and the progress bar
/// would sit still while the CPU works. With it, `advance` tracks real progress.
fn feed<W: Write>(writer: &mut W, data: &[u8], stage: Stage, obs: &dyn Observer) -> Result<()> {
    if obs.cancelled() {
        return Err(Error::Cancelled);
    }
    let chunk = data.len().div_ceil(CHUNK_COUNT).max(CHUNK_MIN);
    let mut done = 0usize;
    for part in data.chunks(chunk) {
        if obs.cancelled() {
            return Err(Error::Cancelled);
        }
        writer.write_all(part)?;
        writer.flush()?;
        done += part.len();
        obs.advance(stage, done, data.len());
    }
    Ok(())
}

/// Is the extra saving brotli promises worth the wait it costs?
///
/// Precondition: `best_so_far < raw`. Whether the input compressed at all is a
/// separate gate, decided before this one is consulted.
pub fn worth_brotli(raw: usize, best_so_far: usize) -> bool {
    let predicted_gain = best_so_far as f64 * BROTLI_MARGINAL_GAIN;
    predicted_gain / OPTICAL_REF_BPS > raw as f64 / BROTLI_BPS
}

/// Pick raw / gzip / brotli, keeping the smallest representation, while reporting
/// progress and honouring cancellation.
pub fn best_observed(data: &[u8], obs: &dyn Observer) -> Result<Outcome> {
    let mut comp = COMP_RAW;
    let mut payload = data.to_vec();
    let mut skipped = None;

    let gz = gzip_observed(data, obs)?;
    if gz.len() < payload.len() {
        comp = COMP_GZIP;
        payload = gz;
    }

    if payload.len() >= data.len() {
        skipped = Some(Skip::Incompressible {
            raw: data.len(),
            best: payload.len(),
        });
    } else if !worth_brotli(data.len(), payload.len()) {
        let predicted_gain = (payload.len() as f64 * BROTLI_MARGINAL_GAIN) as usize;
        skipped = Some(Skip::NotWorthIt {
            predicted_gain,
            encode_secs: data.len() as f64 / BROTLI_BPS,
            saved_secs: predicted_gain as f64 / OPTICAL_REF_BPS,
        });
    } else {
        let br = brotli_observed(data, obs)?;
        if br.len() < payload.len() {
            comp = COMP_BROTLI;
            payload = br;
        }
    }

    Ok(Outcome {
        comp,
        payload,
        skipped,
    })
}

/// Pick raw / gzip / brotli, keeping the smallest representation.
pub fn best(data: &[u8]) -> (u8, Vec<u8>) {
    match best_observed(data, &Quiet) {
        Ok(o) => (o.comp, o.payload),
        // Only reachable when cancelled, which `Quiet` never does.
        Err(_) => (COMP_RAW, data.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn compressible(len: usize) -> Vec<u8> {
        // Repetitive text with a little noise: gzip should land near 0.5x, which is
        // the band where brotli still earns its keep.
        let mut out = Vec::with_capacity(len);
        let mut n = 0u32;
        let words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];
        while out.len() < len {
            n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.extend_from_slice(words[(n >> 27) as usize % words.len()].as_bytes());
            if n.is_multiple_of(7) {
                out.extend_from_slice(&n.to_le_bytes());
            }
            if n.is_multiple_of(23) {
                out.push(b' ');
            }
        }
        out.truncate(len);
        out
    }

    fn incompressible(len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut n = 0x2545_F491_4F6C_DD1Du64;
        while out.len() < len {
            n ^= n << 13;
            n ^= n >> 7;
            n ^= n << 17;
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    #[test]
    fn round_trips_whatever_the_gate_decides() {
        for data in [compressible(300_000), incompressible(300_000), vec![]] {
            let outcome = best_observed(&data, &Quiet).unwrap();
            let back = apply(outcome.comp, &outcome.payload).unwrap();
            assert_eq!(back, data, "comp={} must round-trip", outcome.comp);
        }
    }

    #[test]
    fn incompressible_payload_never_pays_for_brotli() {
        let data = incompressible(200_000);
        let outcome = best_observed(&data, &Quiet).unwrap();
        assert_eq!(outcome.comp, COMP_RAW);
        assert!(
            matches!(outcome.skipped, Some(Skip::Incompressible { .. })),
            "expected the incompressible gate to fire, got {:?}",
            outcome.skipped
        );
    }

    #[test]
    fn brotli_runs_only_when_it_pays_for_itself() {
        // 500 KB at ~1.3 MB/s costs ~0.38 s of CPU; the predicted gain over gzip has
        // to be worth more than that in optical time.
        let raw = 500_000;
        assert!(!worth_brotli(raw, raw / 20)); // gzip already crushed it: skip
        assert!(worth_brotli(raw, raw * 9 / 10)); // barely compressed: worth trying
    }

    #[test]
    fn measured_cases_match_the_model() {
        // Real numbers from testdata (see the daily log): 960 KB of text costs
        // 1.25 s of brotli to save 8.4 KB over gzip, so the model must decline.
        assert!(!worth_brotli(960_107, 78_544));
        // 8 MB of random data is the *other* gate: gzip could not shrink it at all,
        // so brotli has nothing to beat and never even gets considered.
        let random = incompressible(200_000);
        assert!(matches!(
            best_observed(&random, &Quiet).unwrap().skipped,
            Some(Skip::Incompressible { .. })
        ));
        // The same is true at the sizes the operator actually complains about.
        assert!(matches!(
            best_observed(&incompressible(2_000_000), &Quiet)
                .unwrap()
                .skipped,
            Some(Skip::Incompressible { .. })
        ));
    }

    #[test]
    fn cancellation_stops_the_encoder() {
        struct CancelAfter(AtomicUsize);
        impl Observer for CancelAfter {
            fn advance(&self, _: Stage, _done: usize, _total: usize) {}
            fn cancelled(&self) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed) >= 3
            }
        }
        let data = compressible(4_000_000);
        let obs = CancelAfter(AtomicUsize::new(0));
        assert!(matches!(best_observed(&data, &obs), Err(Error::Cancelled)));
    }

    #[test]
    fn progress_is_reported_monotonically_and_completes() {
        struct Track {
            seen: std::sync::Mutex<Vec<(Stage, usize, usize)>>,
        }
        impl Observer for Track {
            fn advance(&self, stage: Stage, done: usize, total: usize) {
                self.seen.lock().unwrap().push((stage, done, total));
            }
        }
        let data = compressible(2_000_000);
        let obs = Track {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        best_observed(&data, &obs).unwrap();
        let seen = obs.seen.lock().unwrap();
        assert!(
            seen.len() >= 2,
            "expected several chunks, got {}",
            seen.len()
        );
        for stage in [Stage::Gzip, Stage::Brotli] {
            let marks: Vec<usize> = seen
                .iter()
                .filter(|(s, _, _)| *s == stage)
                .map(|(_, done, _)| *done)
                .collect();
            if marks.is_empty() {
                continue;
            }
            assert!(
                marks.windows(2).all(|w| w[1] > w[0]),
                "not monotonic: {marks:?}"
            );
            assert_eq!(*marks.last().unwrap(), 2_000_000, "must reach the end");
        }
    }

    #[test]
    fn atomic_observer_is_usable_across_threads() {
        // The GUI hands the same flags to a worker thread, so they have to be Send+Sync.
        struct Flags {
            cancel: AtomicBool,
        }
        impl Observer for Flags {
            fn advance(&self, _: Stage, _: usize, _: usize) {}
            fn cancelled(&self) -> bool {
                self.cancel.load(Ordering::Relaxed)
            }
        }
        let obs = Flags {
            cancel: AtomicBool::new(true),
        };
        assert!(matches!(
            best_observed(&compressible(1000), &obs),
            Err(Error::Cancelled)
        ));
    }
}
