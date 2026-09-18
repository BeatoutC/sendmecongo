//! SMC1 container — the byte stream protected by RaptorQ.
//!
//! The container is rebuilt in full before any of it can be used, so file metadata
//! lives inside the fountain-coded object rather than in a separate beacon frame.
//!
//!   magic      4B  "SMC1"
//!   version    1B  1
//!   comp       1B  0 = raw, 1 = gzip, 2 = brotli
//!   name_len   2B
//!   name       name_len bytes, UTF-8
//!   orig_len   8B  size of the original file
//!   orig_crc   4B  CRC-32 of the original file
//!   payload    rest (compressed or raw file bytes)

use crate::{Error, Result};

pub const MAGIC: [u8; 4] = *b"SMC1";
pub const VERSION: u8 = 1;
pub const COMP_RAW: u8 = 0;
pub const COMP_GZIP: u8 = 1;
pub const COMP_BROTLI: u8 = 2;

const FIXED: usize = 4 + 1 + 1 + 2 + 8 + 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub name: String,
    pub orig_len: u64,
    pub orig_crc: u32,
    pub comp: u8,
    pub payload: Vec<u8>,
}

pub fn encode(name: &str, original: &[u8], comp: u8, payload: &[u8]) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut out = Vec::with_capacity(FIXED + name_bytes.len() + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(comp);
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(&(original.len() as u64).to_le_bytes());
    out.extend_from_slice(&crate::crc32(original).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Validate the fixed header and locate the name and payload.
/// Returns (name range, payload offset). `comp` is left in the caller's hands.
fn header(bytes: &[u8]) -> Result<(std::ops::Range<usize>, usize)> {
    if bytes.len() < FIXED {
        return Err(Error::Truncated);
    }
    if bytes[0..4] != MAGIC {
        return Err(Error::BadMagic);
    }
    if bytes[4] != VERSION {
        return Err(Error::UnsupportedVersion(bytes[4]));
    }
    if bytes[5] > COMP_BROTLI {
        return Err(Error::UnknownCompression(bytes[5]));
    }
    let name_len = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    let name_end = 8 + name_len;
    if name_end + 12 > bytes.len() {
        return Err(Error::Truncated);
    }
    Ok((8..name_end, name_end + 12))
}

/// Read the file name without copying the payload.
///
/// The player needs a window title and nothing else. `decode` clones the payload,
/// which on a multi-megabyte object would be a second full-size allocation spent
/// on a string.
pub fn peek_name(bytes: &[u8]) -> Result<String> {
    let (name_range, _) = header(bytes)?;
    Ok(std::str::from_utf8(&bytes[name_range])
        .map_err(|_| Error::BadName)?
        .to_string())
}

pub fn decode(bytes: &[u8]) -> Result<Container> {
    let (name_range, payload_at) = header(bytes)?;
    let name = std::str::from_utf8(&bytes[name_range])
        .map_err(|_| Error::BadName)?
        .to_string();
    let meta = payload_at - 12;
    let orig_len = u64::from_le_bytes(bytes[meta..meta + 8].try_into().unwrap());
    let orig_crc = u32::from_le_bytes(bytes[meta + 8..meta + 12].try_into().unwrap());
    Ok(Container {
        name,
        orig_len,
        orig_crc,
        comp: bytes[5],
        payload: bytes[payload_at..].to_vec(),
    })
}

/// Decompress the container payload and verify it against the stored original CRC.
pub fn into_file(container: &Container) -> Result<Vec<u8>> {
    let data = crate::compress::apply(container.comp, &container.payload)?;
    if data.len() as u64 != container.orig_len {
        return Err(Error::Other(format!(
            "size mismatch: expected {}, got {}",
            container.orig_len,
            data.len()
        )));
    }
    if crate::crc32(&data) != container.orig_crc {
        return Err(Error::CrcMismatch);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_name_agrees_with_decode() {
        let data = vec![7u8; 100_000];
        let (comp, payload) = crate::compress::best(&data);
        let object = encode("报告 v2.docx", &data, comp, &payload);
        assert_eq!(peek_name(&object).unwrap(), "报告 v2.docx");
        let parsed = decode(&object).unwrap();
        assert_eq!(parsed.name, "报告 v2.docx");
        assert_eq!(into_file(&parsed).unwrap(), data);
    }

    #[test]
    fn malformed_objects_are_rejected_not_panicked() {
        assert!(matches!(peek_name(b"AGC"), Err(Error::Truncated)));
        assert!(matches!(peek_name(&[b'X'; 32]), Err(Error::BadMagic)));
        let mut bad = encode("a.txt", b"hi", COMP_RAW, b"hi");
        bad[5] = 9;
        assert!(matches!(peek_name(&bad), Err(Error::UnknownCompression(9))));
    }
}
