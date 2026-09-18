//! QR recognition.
//!
//! `rxing` is a pure-Rust port of ZXing, and on this project's footage it returns
//! byte-identical payloads to `zxing-cpp`. The alternative, `rqrr`, decoded the
//! cleanly rendered PNG frames just as well but collapsed to **zero** usable
//! payloads on an actual camera recording, so it is not used.
//!
//! Two measured facts shape this module:
//!
//! * `TryHarder` is off. Over 47 real 4K frames it costs 0.488 s/frame against
//!   0.173 s/frame and returns the *same* 94 symbols.
//! * `detect_multiple_in_luma` can miss a code. On frame 300 of `06.MOV` it
//!   reports **one** of the two codes on screen where `zxing-cpp` reports both,
//!   which is the whole difference between 0.83 and 1.53 codes per frame over
//!   that recording. A frame that comes up short is therefore re-read in two
//!   overlapping vertical bands, each of which then holds a single code.
//!
//! Two things that look obvious and were measured to be *wrong*:
//!
//! * Locking onto the boxes a code was last seen in and reading only those is
//!   slower, not faster. The multi-code reader finds both codes in **one**
//!   binarise-and-search pass; two independent reads of two sub-images cost more
//!   than that single pass, and on `08.MOV` they took the run from 65 s to 88 s.
//! * Masking a found code out with white and re-reading the frame is unreliable
//!   here because the two codes sit a few pixels apart: a mask big enough to work
//!   swallows its neighbour. The bands avoid the problem by never modifying the
//!   image.

use rxing::{DecodeHints, RXingResult};

/// The link never puts more than two codes on screen at once (turbo60/megabit).
const MAX_CODES: usize = 2;
/// Width of each fallback band, as a fraction of the frame. The two overlap so a
/// code can never end up straddling a band edge.
const BAND: f32 = 0.60;
/// Frames spent deciding how many codes to expect before the extra band reads
/// can be switched off. See [`Scanner::expected_codes`].
const LEARN_FRAMES: usize = 45;

pub struct Scanner {
    hints: DecodeHints,
    /// Most codes seen on one frame so far.
    lanes_seen: usize,
    frames_seen: usize,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        let mut hints = DecodeHints::default();
        hints.TryHarder = Some(false);
        Self {
            hints,
            lanes_seen: 0,
            frames_seen: 0,
        }
    }

    /// How many codes a frame should carry.
    ///
    /// The receiver is never told which preset was used — that is the point of a
    /// one-way channel — so it is learned instead: a dual-lane stream shows two
    /// codes within the first frames, a single-lane one never does. While the
    /// answer is still unknown, err on the side of looking twice.
    fn expected_codes(&self) -> usize {
        if self.lanes_seen >= MAX_CODES || self.frames_seen < LEARN_FRAMES {
            MAX_CODES
        } else {
            1
        }
    }

    /// Decode every QR code in a luma plane, appending the SMQ frame payloads to
    /// `out`. Returns the number of SMQ frames found.
    ///
    /// The `SMQ` magic is checked here so that a stray QR code on screen — a
    /// poster, a web page in the corner — is dropped before it can disturb the
    /// statistics. Everything past that (CRC-32 per frame, session checksum,
    /// container checksum) is verified by the shared decoder.
    pub fn scan(
        &mut self,
        luma: Vec<u8>,
        width: u32,
        height: u32,
        out: &mut Vec<Vec<u8>>,
    ) -> usize {
        if width == 0 || height == 0 {
            return 0;
        }
        let expected = self.expected_codes();
        let mut payloads = Vec::with_capacity(MAX_CODES);

        self.read_all(luma.clone(), width, height, &mut payloads);

        if payloads.len() < expected {
            let band_width = (width as f32 * BAND) as u32;
            for start in [0u32, width.saturating_sub(band_width)] {
                let band = Roi {
                    x: start,
                    y: 0,
                    w: band_width.min(width - start),
                    h: height,
                };
                let Some(region) = crop(&luma, width, height, band) else {
                    continue;
                };
                self.read_all(region, band.w, band.h, &mut payloads);
                if payloads.len() >= MAX_CODES {
                    break;
                }
            }
        }

        self.frames_seen += 1;
        self.lanes_seen = self.lanes_seen.max(payloads.len());

        let found = payloads.len();
        for payload in payloads {
            out.push(payload);
        }
        found
    }

    /// One multi-code read, merging into `payloads` without duplicates.
    fn read_all(&mut self, luma: Vec<u8>, width: u32, height: u32, payloads: &mut Vec<Vec<u8>>) {
        let Ok(results) = rxing::helpers::detect_multiple_in_luma_with_hints(
            luma,
            width,
            height,
            &mut self.hints,
        ) else {
            return;
        };
        for result in &results {
            let Some(payload) = agq_payload(result) else {
                continue;
            };
            if payloads.iter().any(|seen| *seen == payload) {
                continue;
            }
            payloads.push(payload);
        }
    }
}

fn agq_payload(result: &RXingResult) -> Option<Vec<u8>> {
    let bytes = result.getRawBytes();
    if bytes.len() > sendmecongo_core::frame::HEADER_LEN
        && bytes.starts_with(&sendmecongo_core::frame::MAGIC)
    {
        Some(bytes.to_vec())
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Roi {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

fn crop(luma: &[u8], width: u32, height: u32, roi: Roi) -> Option<Vec<u8>> {
    if roi.w == 0 || roi.h == 0 || roi.x + roi.w > width || roi.y + roi.h > height {
        return None;
    }
    let mut out = Vec::with_capacity((roi.w * roi.h) as usize);
    for row in roi.y..roi.y + roi.h {
        let start = (row * width + roi.x) as usize;
        out.extend_from_slice(&luma[start..start + roi.w as usize]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_believed_to_hold_two_codes_until_proven_otherwise() {
        let scanner = Scanner::new();
        assert_eq!(scanner.expected_codes(), MAX_CODES);
    }

    #[test]
    fn a_single_lane_stream_stops_paying_for_the_second_look() {
        let mut scanner = Scanner::new();
        scanner.frames_seen = LEARN_FRAMES;
        assert_eq!(scanner.expected_codes(), 1);
        scanner.lanes_seen = MAX_CODES;
        assert_eq!(scanner.expected_codes(), MAX_CODES);
    }

    #[test]
    fn crop_copies_the_requested_rectangle_row_by_row() {
        let (w, h) = (4u32, 3u32);
        let luma: Vec<u8> = (0..12).collect();
        let patch = crop(
            &luma,
            w,
            h,
            Roi {
                x: 1,
                y: 1,
                w: 2,
                h: 2,
            },
        )
        .unwrap();
        assert_eq!(patch, vec![5, 6, 9, 10]);
    }

    #[test]
    fn an_out_of_frame_region_is_refused_rather_than_clamped() {
        let luma = vec![0u8; 16];
        assert!(crop(
            &luma,
            4,
            4,
            Roi {
                x: 3,
                y: 3,
                w: 4,
                h: 4
            }
        )
        .is_none());
    }

    #[test]
    fn the_two_bands_cover_the_whole_width() {
        let width = 3840u32;
        let band_width = (width as f32 * BAND) as u32;
        let starts = [0u32, width.saturating_sub(band_width)];
        assert_eq!(starts[1] + band_width, width);
        assert!(starts[1] < starts[0] + band_width, "bands must overlap");
    }
}
