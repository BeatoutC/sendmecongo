//! Video decoding, reduced to the one thing the optical link needs: luma planes.
//!
//! Two pure-Rust decoders cover both codecs a phone camera produces:
//! `rusty_h265` for HEVC (the iPhone default, and what every recording in
//! `recordings/` turned out to be) and `openh264` for H.264 (most Android
//! captures). Neither needs FFmpeg, and both take Annex-B, so the wrapping is
//! just "feed one access unit, collect whatever pictures came out".
//!
//! Only the luma plane is materialised. Chroma is 2/3 of the decode output and
//! QR recognition never looks at it.

use crate::isobmff::Codec;

/// One decoded picture, luma only, tightly packed at `width * height` bytes.
pub struct GrayFrame {
    pub width: usize,
    pub height: usize,
    pub luma: Vec<u8>,
}

/// A decoder plus the running count of access units it could not make sense of.
///
/// A decode error is deliberately not fatal. Splitting the stream at open-GOP
/// keyframes means each worker starts mid-stream, where the first one or two
/// leading pictures are expected to fail — and those frames are exactly the
/// ones the fountain code has repair symbols for.
pub struct VideoDecoder {
    inner: Inner,
    pub errors: usize,
}

enum Inner {
    Hevc(Box<rusty_h265::Decoder>),
    #[cfg(feature = "h264")]
    Avc(Box<openh264::decoder::Decoder>),
}

impl VideoDecoder {
    pub fn new(codec: Codec) -> Result<Self, String> {
        let inner = match codec {
            Codec::Hevc => Inner::Hevc(Box::new(rusty_h265::Decoder::new())),
            #[cfg(feature = "h264")]
            Codec::Avc => Inner::Avc(Box::new(openh264::decoder::Decoder::new().map_err(
                |e| sendmecongo_ui::i18n::fill(sendmecongo_ui::i18n::t().err_h264_init, &[&e]),
            )?)),
            #[cfg(not(feature = "h264"))]
            Codec::Avc => return Err(sendmecongo_ui::i18n::t().err_h264_missing.into()),
        };
        Ok(Self { inner, errors: 0 })
    }

    /// Feed one access unit (Annex-B); completed pictures are appended to `out`.
    /// Returns how many pictures this access unit produced.
    pub fn push(&mut self, access_unit: &[u8], out: &mut Vec<GrayFrame>) -> usize {
        let before = out.len();
        match &mut self.inner {
            Inner::Hevc(dec) => {
                if dec.push_annexb(access_unit, None).is_err() {
                    self.errors += 1;
                }
                drain_hevc(dec, out);
            }
            #[cfg(feature = "h264")]
            Inner::Avc(dec) => match dec.decode(access_unit) {
                Ok(Some(frame)) => push_yuv(&frame, out),
                Ok(None) => {}
                Err(_) => self.errors += 1,
            },
        }
        out.len() - before
    }

    /// End of stream: release pictures still held for reordering.
    pub fn flush(&mut self, out: &mut Vec<GrayFrame>) {
        match &mut self.inner {
            Inner::Hevc(dec) => {
                dec.flush();
                drain_hevc(dec, out);
            }
            #[cfg(feature = "h264")]
            Inner::Avc(dec) => {
                if let Ok(frames) = dec.flush_remaining() {
                    for frame in &frames {
                        push_yuv(frame, out);
                    }
                }
            }
        }
    }
}

fn drain_hevc(dec: &mut rusty_h265::Decoder, out: &mut Vec<GrayFrame>) {
    let mut scratch = Vec::new();
    while let Ok(frame) = dec.next_frame() {
        let (w, h) = (frame.width, frame.height);
        if w == 0 || h == 0 {
            continue;
        }
        scratch.clear();
        frame.write_yuv(&mut scratch);
        let luma_len = w * h;
        // write_yuv emits Y then U then V, cropped, as `u8` samples for 8-bit
        // content and little-endian `u16` beyond that. Phone HEVC is *usually*
        // Main 10, so assuming one byte per sample silently interleaves two
        // pixels per value and every frame comes out as noise — which looks
        // exactly like a camera problem and is not one.
        if frame.bit_depth() > 8 {
            if scratch.len() < luma_len * 2 {
                continue;
            }
            let shift = frame.bit_depth() - 8;
            let mut luma = vec![0u8; luma_len];
            for (index, slot) in luma.iter_mut().enumerate() {
                let sample = u16::from_le_bytes([scratch[index * 2], scratch[index * 2 + 1]]);
                *slot = (sample >> shift) as u8;
            }
            out.push(GrayFrame {
                width: w,
                height: h,
                luma,
            });
        } else {
            if scratch.len() < luma_len {
                continue;
            }
            scratch.truncate(luma_len);
            out.push(GrayFrame {
                width: w,
                height: h,
                luma: std::mem::take(&mut scratch),
            });
        }
    }
}

#[cfg(feature = "h264")]
fn push_yuv(frame: &openh264::decoder::DecodedYUV<'_>, out: &mut Vec<GrayFrame>) {
    use openh264::formats::YUVSource;
    let (w, h) = frame.dimensions();
    let (y_stride, _, _) = frame.strides();
    if w == 0 || h == 0 {
        return;
    }
    let src = frame.y();
    let mut luma = vec![0u8; w * h];
    for row in 0..h {
        let start = row * y_stride;
        if start + w > src.len() {
            break;
        }
        luma[row * w..(row + 1) * w].copy_from_slice(&src[start..start + w]);
    }
    out.push(GrayFrame {
        width: w,
        height: h,
        luma,
    });
}
