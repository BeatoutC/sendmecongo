//! Minimal ISO-BMFF (MP4 / MOV) demuxer.
//!
//! The receiver has to open whatever a phone camera produced with no external
//! tooling, so this walks the box tree by hand instead of pulling in a demuxer
//! that may or may not understand `hvc1`. Only what the optical link needs is
//! implemented: locate the video track, read its codec configuration, and turn
//! the sample tables into a flat list of `(file offset, length)` records.
//!
//! Deliberately *not* supported: fragmented movies (`moof`/`traf`/`trun`) and
//! `stz2` compact sample sizes. Both are refused with a named error rather than
//! silently producing a truncated stream — a camera recording is never either
//! of those, so a clear message beats a wrong answer.

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// VisualSampleEntry fixed prefix: 6 reserved + 2 data_reference_index +
/// 16 pre_defined/reserved + 2 width + 2 height + 4 hres + 4 vres + 4 reserved +
/// 2 frame_count + 32 compressorname + 2 depth + 2 pre_defined.
const VISUAL_SAMPLE_ENTRY_PREFIX: u64 = 78;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Hevc,
    Avc,
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Codec::Hevc => "HEVC",
            Codec::Avc => "H.264",
        })
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotIsobmff,
    /// A movie whose samples live in `moof` fragments. Phones do not write
    /// these for camera captures, but screen recorders and some editors do.
    Fragmented,
    NoVideoTrack,
    MissingTable(&'static str),
    BadStructure(&'static str),
    UnsupportedCodec(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Read at the moment of the failure, not at the moment of construction: this text
        // ends up on the receiver's screen and in the JSON report, in their language.
        let t = sendmecongo_ui::i18n::t();
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NotIsobmff => f.write_str(t.err_iso_not_isobmff),
            Error::Fragmented => f.write_str(t.err_iso_fragmented),
            Error::NoVideoTrack => f.write_str(t.err_iso_no_video_track),
            Error::MissingTable(table) => {
                f.write_str(&sendmecongo_ui::i18n::fill(t.err_iso_missing_table, &[table]))
            }
            Error::BadStructure(detail) => {
                f.write_str(&sendmecongo_ui::i18n::fill(t.err_iso_bad_structure, &[detail]))
            }
            Error::UnsupportedCodec(codec) => f.write_str(&sendmecongo_ui::i18n::fill(
                t.err_iso_unsupported_codec,
                &[codec],
            )),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy)]
struct BoxHeader {
    kind: [u8; 4],
    /// Offset of the first payload byte.
    body: u64,
    /// Offset one past the last payload byte.
    end: u64,
}

impl BoxHeader {
    fn is(&self, kind: &[u8; 4]) -> bool {
        &self.kind == kind
    }
}

fn read_header(f: &mut File, pos: u64, limit: u64) -> Result<BoxHeader> {
    if pos + 8 > limit {
        return Err(Error::BadStructure("box header past end of file"));
    }
    f.seek(SeekFrom::Start(pos))?;
    let mut head = [0u8; 8];
    f.read_exact(&mut head)?;
    let mut size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as u64;
    let kind = [head[4], head[5], head[6], head[7]];
    let mut body = pos + 8;
    if size == 1 {
        let mut ext = [0u8; 8];
        f.read_exact(&mut ext)?;
        size = u64::from_be_bytes(ext);
        body = pos + 16;
    } else if size == 0 {
        size = limit - pos;
    }
    if size < body - pos || pos + size > limit {
        return Err(Error::BadStructure("box size out of range"));
    }
    Ok(BoxHeader {
        kind,
        body,
        end: pos + size,
    })
}

/// Direct children of the box whose payload spans `[start, end)`.
fn children(f: &mut File, start: u64, end: u64) -> Result<Vec<BoxHeader>> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let h = read_header(f, pos, end)?;
        if h.end <= pos {
            break;
        }
        pos = h.end;
        out.push(h);
    }
    Ok(out)
}

/// Walk a chain of four-character box types from a payload range.
fn find_path(f: &mut File, start: u64, end: u64, path: &[&[u8; 4]]) -> Result<Option<BoxHeader>> {
    let mut range = (start, end);
    let mut found = None;
    for want in path {
        match children(f, range.0, range.1)?
            .into_iter()
            .find(|k| k.is(want))
        {
            Some(h) => {
                range = (h.body, h.end);
                found = Some(h);
            }
            None => return Ok(None),
        }
    }
    Ok(found)
}

fn read_bytes(f: &mut File, start: u64, end: u64) -> Result<Vec<u8>> {
    let n = (end - start) as usize;
    let mut buf = vec![0u8; n];
    f.seek(SeekFrom::Start(start))?;
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_u32(f: &mut File, at: u64) -> Result<u32> {
    let b = read_bytes(f, at, at + 4)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub offset: u64,
    pub size: u32,
}

#[derive(Debug)]
pub struct VideoTrack {
    pub codec: Codec,
    /// VPS/SPS/PPS (HEVC) or SPS/PPS (H.264), in the order they must be emitted.
    pub parameter_sets: Vec<Vec<u8>>,
    /// Length-prefix width of the length-prefixed NAL units inside a sample.
    pub nal_length_size: usize,
    pub width: u32,
    pub height: u32,
    pub timescale: u32,
    pub duration: u64,
    pub samples: Vec<Sample>,
}

impl VideoTrack {
    pub fn fps(&self) -> f64 {
        let secs = self.duration_secs();
        if secs <= 0.0 {
            return 0.0;
        }
        self.samples.len() as f64 / secs
    }

    pub fn duration_secs(&self) -> f64 {
        if self.timescale == 0 {
            0.0
        } else {
            self.duration as f64 / self.timescale as f64
        }
    }
}

pub fn open(path: &Path) -> Result<VideoTrack> {
    let mut f = File::open(path)?;
    let file_len = f.metadata()?.len();
    let top = children(&mut f, 0, file_len)?;

    if top.iter().any(|b| b.is(b"moof")) {
        return Err(Error::Fragmented);
    }
    let moov = top
        .iter()
        .find(|b| b.is(b"moov"))
        .ok_or(Error::NotIsobmff)?;
    let (moov_body, moov_end) = (moov.body, moov.end);

    for trak in children(&mut f, moov_body, moov_end)?
        .into_iter()
        .filter(|b| b.is(b"trak"))
    {
        // hdlr payload: version+flags(4) pre_defined(4) handler_type(4)
        let Some(hdlr) = find_path(&mut f, trak.body, trak.end, &[b"mdia", b"hdlr"])? else {
            continue;
        };
        if read_u32(&mut f, hdlr.body + 8)? == u32::from_be_bytes(*b"vide") {
            return read_track(&mut f, trak.body, trak.end);
        }
    }
    Err(Error::NoVideoTrack)
}

fn read_track(f: &mut File, trak_body: u64, trak_end: u64) -> Result<VideoTrack> {
    let stbl = find_path(f, trak_body, trak_end, &[b"mdia", b"minf", b"stbl"])?
        .ok_or(Error::MissingTable("stbl"))?;
    let (stbl_body, stbl_end) = (stbl.body, stbl.end);

    // --- timing -----------------------------------------------------------
    let (timescale, duration) = match find_path(f, trak_body, trak_end, &[b"mdia", b"mdhd"])? {
        Some(mdhd) => {
            let version = read_bytes(f, mdhd.body, mdhd.body + 1)?[0];
            if version == 1 {
                let ts = read_u32(f, mdhd.body + 20)?;
                let b = read_bytes(f, mdhd.body + 24, mdhd.body + 32)?;
                (ts, u64::from_be_bytes(b.try_into().unwrap()))
            } else {
                (
                    read_u32(f, mdhd.body + 12)?,
                    read_u32(f, mdhd.body + 16)? as u64,
                )
            }
        }
        None => (0, 0),
    };

    // --- codec + configuration -------------------------------------------
    let stsd = children(f, stbl_body, stbl_end)?
        .into_iter()
        .find(|b| b.is(b"stsd"))
        .ok_or(Error::MissingTable("stsd"))?;
    // stsd payload: version+flags(4) entry_count(4) then sample entries
    let entry = children(f, stsd.body + 8, stsd.end)?
        .into_iter()
        .find(|b| b.is(b"hvc1") || b.is(b"hev1") || b.is(b"avc1") || b.is(b"avc3"))
        .ok_or_else(|| Error::UnsupportedCodec(describe_entries(f, stsd.body + 8, stsd.end)))?;

    let is_hevc = entry.is(b"hvc1") || entry.is(b"hev1");
    let codec = if is_hevc { Codec::Hevc } else { Codec::Avc };
    let width = read_bytes(f, entry.body + 24, entry.body + 26)
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
        .unwrap_or(0);
    let height = read_bytes(f, entry.body + 26, entry.body + 28)
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
        .unwrap_or(0);

    let config_kind: &[u8; 4] = if is_hevc { b"hvcC" } else { b"avcC" };
    let cfg = children(f, entry.body + VISUAL_SAMPLE_ENTRY_PREFIX, entry.end)?
        .into_iter()
        .find(|b| b.is(config_kind))
        .ok_or(Error::BadStructure(
            "sample entry has no codec configuration box",
        ))?;
    let conf = read_bytes(f, cfg.body, cfg.end)?;
    let (parameter_sets, nal_length_size) = if is_hevc {
        parse_hvcc(&conf)?
    } else {
        parse_avcc(&conf)?
    };

    // --- sample tables ----------------------------------------------------
    let stsz = children(f, stbl_body, stbl_end)?
        .into_iter()
        .find(|b| b.is(b"stsz"))
        .ok_or(Error::MissingTable("stsz"))?;
    let default_sample_size = read_u32(f, stsz.body + 4)?;
    let sample_count = read_u32(f, stsz.body + 8)? as usize;
    let sizes: Vec<u32> = if default_sample_size != 0 {
        vec![default_sample_size; sample_count]
    } else {
        let raw = read_bytes(f, stsz.body + 12, stsz.end)?;
        if raw.len() < sample_count * 4 {
            return Err(Error::BadStructure("stsz is shorter than its sample count"));
        }
        raw.chunks_exact(4)
            .take(sample_count)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    };

    let stsc = children(f, stbl_body, stbl_end)?
        .into_iter()
        .find(|b| b.is(b"stsc"))
        .ok_or(Error::MissingTable("stsc"))?;
    // stsc payload: version+flags(4) entry_count(4) then 12-byte run entries.
    let raw = read_bytes(f, stsc.body + 8, stsc.end)?;
    if raw.len() % 12 != 0 {
        return Err(Error::BadStructure(
            "stsc entry size is not a multiple of 12",
        ));
    }
    let runs: Vec<(u32, u32)> = raw
        .chunks_exact(12)
        .map(|c| {
            (
                u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                u32::from_be_bytes([c[4], c[5], c[6], c[7]]),
            )
        })
        .collect();
    if runs.is_empty() {
        return Err(Error::BadStructure("stsc is empty"));
    }

    let stco = children(f, stbl_body, stbl_end)?
        .into_iter()
        .find(|b| b.is(b"stco") || b.is(b"co64"))
        .ok_or(Error::MissingTable("stco"))?;
    let wide = stco.is(b"co64");
    let chunk_count = read_u32(f, stco.body + 4)? as usize;
    let raw = read_bytes(f, stco.body + 8, stco.end)?;
    let chunk_offsets: Vec<u64> = if wide {
        raw.chunks_exact(8)
            .take(chunk_count)
            .map(|c| u64::from_be_bytes(c.try_into().unwrap()))
            .collect()
    } else {
        raw.chunks_exact(4)
            .take(chunk_count)
            .map(|c| u32::from_be_bytes(c.try_into().unwrap()) as u64)
            .collect()
    };

    let mut samples = Vec::with_capacity(sample_count);
    let mut next = 0usize;
    for (index, &chunk_offset) in chunk_offsets.iter().enumerate() {
        if next >= sample_count {
            break;
        }
        // The applicable run is the last one whose first_chunk is <= this chunk.
        let chunk_number = index as u32 + 1;
        let per_chunk = runs
            .iter()
            .rev()
            .find(|(first, _)| *first <= chunk_number)
            .map(|(_, n)| *n)
            .unwrap_or(1) as usize;
        let mut offset = chunk_offset;
        for _ in 0..per_chunk {
            if next >= sample_count {
                break;
            }
            let size = sizes[next];
            samples.push(Sample { offset, size });
            offset += size as u64;
            next += 1;
        }
    }
    if next != sample_count {
        return Err(Error::BadStructure(
            "sample tables disagree on how many samples there are",
        ));
    }

    Ok(VideoTrack {
        codec,
        parameter_sets,
        nal_length_size,
        width,
        height,
        timescale,
        duration,
        samples,
    })
}

/// Best-effort label for the "unsupported codec" message.
fn describe_entries(f: &mut File, start: u64, end: u64) -> String {
    children(f, start, end)
        .map(|kids| {
            kids.iter()
                .map(|k| String::from_utf8_lossy(&k.kind).trim().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// hvcC: ... constantFrameRate/numTemporalLayers/temporalIdNested/lengthSizeMinusOne
/// lives at byte 21, then numOfArrays at 22, then one array per NAL type.
fn parse_hvcc(conf: &[u8]) -> Result<(Vec<Vec<u8>>, usize)> {
    if conf.len() < 23 {
        return Err(Error::BadStructure("hvcC is truncated"));
    }
    let nal_length_size = (conf[21] & 3) as usize + 1;
    let num_arrays = conf[22] as usize;
    let mut pos = 23usize;
    let mut sets = Vec::new();
    for _ in 0..num_arrays {
        if pos + 3 > conf.len() {
            return Err(Error::BadStructure("hvcC array header is truncated"));
        }
        let count = u16::from_be_bytes([conf[pos + 1], conf[pos + 2]]) as usize;
        pos += 3;
        for _ in 0..count {
            if pos + 2 > conf.len() {
                return Err(Error::BadStructure("hvcC NAL length is truncated"));
            }
            let len = u16::from_be_bytes([conf[pos], conf[pos + 1]]) as usize;
            pos += 2;
            if pos + len > conf.len() {
                return Err(Error::BadStructure("hvcC NAL overruns the box"));
            }
            sets.push(conf[pos..pos + len].to_vec());
            pos += len;
        }
    }
    Ok((sets, nal_length_size))
}

/// avcC: configurationVersion/profiles(4) then lengthSizeMinusOne at byte 4,
/// numOfSequenceParameterSets at byte 5, then the SPS and PPS lists.
fn parse_avcc(conf: &[u8]) -> Result<(Vec<Vec<u8>>, usize)> {
    if conf.len() < 6 {
        return Err(Error::BadStructure("avcC is truncated"));
    }
    let nal_length_size = (conf[4] & 3) as usize + 1;
    let mut pos = 5usize;
    let mut sets = Vec::new();
    let sps_count = (conf[pos] & 0x1F) as usize;
    pos += 1;
    for _ in 0..sps_count {
        if pos + 2 > conf.len() {
            return Err(Error::BadStructure("avcC SPS length is truncated"));
        }
        let len = u16::from_be_bytes([conf[pos], conf[pos + 1]]) as usize;
        pos += 2;
        if pos + len > conf.len() {
            return Err(Error::BadStructure("avcC SPS overruns the box"));
        }
        sets.push(conf[pos..pos + len].to_vec());
        pos += len;
    }
    if pos >= conf.len() {
        return Err(Error::BadStructure("avcC has no PPS count"));
    }
    let pps_count = conf[pos] as usize;
    pos += 1;
    for _ in 0..pps_count {
        if pos + 2 > conf.len() {
            return Err(Error::BadStructure("avcC PPS length is truncated"));
        }
        let len = u16::from_be_bytes([conf[pos], conf[pos + 1]]) as usize;
        pos += 2;
        if pos + len > conf.len() {
            return Err(Error::BadStructure("avcC PPS overruns the box"));
        }
        sets.push(conf[pos..pos + len].to_vec());
        pos += len;
    }
    Ok((sets, nal_length_size))
}

pub const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Split one length-prefixed sample into its raw NAL units.
pub fn split_sample<'a>(sample: &'a [u8], nal_length_size: usize) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + nal_length_size <= sample.len() {
        let mut len = 0usize;
        for byte in &sample[pos..pos + nal_length_size] {
            len = (len << 8) | *byte as usize;
        }
        pos += nal_length_size;
        if len == 0 || pos + len > sample.len() {
            break;
        }
        out.push(&sample[pos..pos + len]);
        pos += len;
    }
    out
}

/// NAL type of a raw NAL unit, or `None` when it is too short to have a header.
pub fn nal_type(codec: Codec, nal: &[u8]) -> Option<u8> {
    match codec {
        Codec::Hevc => {
            if nal.len() < 2 {
                None
            } else {
                Some((nal[0] >> 1) & 0x3F)
            }
        }
        Codec::Avc => nal.first().map(|b| b & 0x1F),
    }
}

/// Does this NAL start a group of pictures that can be decoded from scratch?
///
/// For HEVC that is BLA/IDR/CRA (16..=21) and for H.264 an IDR (5). Every one
/// of the 50 keyframes a phone writes per minute is a CRA, so recognising them
/// is what makes parallel decoding possible at all.
pub fn is_random_access(codec: Codec, nal_type: u8) -> bool {
    match codec {
        Codec::Hevc => (16..=21).contains(&nal_type),
        // IDR only. SPS/PPS travel in-band in `avc3` streams and would otherwise
        // look like a keyframe every time they are repeated.
        Codec::Avc => nal_type == 5,
    }
}
