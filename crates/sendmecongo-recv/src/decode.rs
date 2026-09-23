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
///
/// The count is read through [`VideoDecoder::take_errors`] and never as a
/// field: one decoder outlives the groups of pictures it is fed, so a caller
/// that reports the *running total* each time a group ends bills every early
/// failure once per group. That is a reported number, not a decoding one — the
/// bytes recovered were always right.
pub struct VideoDecoder {
    inner: Inner,
    errors: usize,
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

    /// Failures since the previous call, leaving the counter at zero.
    ///
    /// A delta, not a total — see the type-level note. Each worker reports once
    /// per group of pictures, and those reports are summed by the caller.
    pub fn take_errors(&mut self) -> usize {
        std::mem::take(&mut self.errors)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归用的小码流：8 帧灰度阶梯、320×240、avc1。由
    /// `tools/make-h264-sample.swift` 生成，随仓库跟踪。
    ///
    /// 为什么要有这么一段：`recordings/` 里的相机素材全是 iPhone 的 HEVC
    /// （`hvc1`），所以 H.264 那一半长期只有「编译过」这一个保障 —— 真跑一段
    /// 安卓录像是人工验收，挡不住以后改坏。相机素材单条 56~312 MB 进不了 git，
    /// 这段 1.5 KB 的合成码流把同一条链路钉在 `cargo test` 里。
    fn sample_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/h264-sample.mp4")
    }

    /// 一帧的平均亮度。灰度阶梯相邻帧差 1/7 个满量程，足够区分。
    fn mean_luma(frame: &GrayFrame) -> u32 {
        let sum: u64 = frame.luma.iter().map(|&s| s as u64).sum();
        (sum / frame.luma.len() as u64) as u32
    }

    #[test]
    fn both_codec_paths_construct() {
        assert!(
            VideoDecoder::new(Codec::Hevc).is_ok(),
            "HEVC 解码器必须一直可构造"
        );
        #[cfg(feature = "h264")]
        assert!(
            VideoDecoder::new(Codec::Avc).is_ok(),
            "openh264 必须能初始化"
        );
    }

    /// 关掉 `h264` feature 时，一段 avc1 录像必须得到一句明确的话，而不是被
    /// 静默拆给 HEVC 解码器去啃 —— 那样对外表现为「一个码也读不出来」，看起来
    /// 像相机的问题。
    #[cfg(not(feature = "h264"))]
    #[test]
    fn avc_without_the_h264_feature_is_refused_out_loud() {
        assert!(VideoDecoder::new(Codec::Avc).is_err());
    }

    /// 最小闭环：容器解析 → openh264 解码 → luma 抽取。
    #[cfg(feature = "h264")]
    #[test]
    fn a_real_h264_recording_decodes_into_its_pictures() {
        let path = sample_path();
        let track = crate::isobmff::open(&path).expect("打开合成样本");
        assert_eq!(
            track.codec,
            Codec::Avc,
            "样本必须是 avc1，否则测的不是 H.264 那一半"
        );
        assert_eq!((track.width, track.height), (320, 240));
        assert_eq!(track.samples.len(), 8, "八个视频样本");
        assert!(
            track.parameter_sets.len() >= 2,
            "avcC 里该有 SPS 和 PPS，参数集走带外"
        );

        let bytes = std::fs::read(&path).unwrap();
        let mut decoder = VideoDecoder::new(Codec::Avc).unwrap();
        let mut frames = Vec::new();
        for sample in &track.samples {
            let start = sample.offset as usize;
            let raw = &bytes[start..start + sample.size as usize];
            // 每个访问单元都自带参数集：这里只测解码，不复刻 producer 那层
            // 「参数集只在随机访问点前重发」。
            let mut access_unit = Vec::new();
            for set in &track.parameter_sets {
                access_unit.extend_from_slice(&crate::isobmff::START_CODE);
                access_unit.extend_from_slice(set);
            }
            for nal in crate::isobmff::split_sample(raw, track.nal_length_size) {
                access_unit.extend_from_slice(&crate::isobmff::START_CODE);
                access_unit.extend_from_slice(nal);
            }
            decoder.push(&access_unit, &mut frames);
        }
        decoder.flush(&mut frames);
        assert_eq!(decoder.take_errors(), 0, "自产的码流不该有解码错误");

        assert_eq!(frames.len(), 8, "八帧要一帧不少地回来");
        for frame in &frames {
            assert_eq!((frame.width, frame.height), (320, 240));
            assert_eq!(
                frame.luma.len(),
                320 * 240,
                "luma 必须是紧密排布的整帧，多一段 stride 或 chroma 都算错"
            );
        }

        // 阶梯单调，所以逐帧递增就同时证明了顺序没错、帧没丢也没重复。比断言
        // 绝对亮度稳：limited-range 转换与有损压缩都会挪动亮度，但不会打乱次序。
        let brightness: Vec<u32> = frames.iter().map(mean_luma).collect();
        assert!(
            brightness.windows(2).all(|w| w[0] < w[1]),
            "灰度阶梯必须逐帧递增，实测 {brightness:?}"
        );
        assert!(
            brightness[7] - brightness[0] > 150,
            "首尾要拉开一个满量程，实测 {brightness:?}"
        );
    }
}
