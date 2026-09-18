//! SMQ1 optical frame.
//!
//! Layout (little-endian):
//!   magic       3B  "SMQ"
//!   version     1B  1
//!   session     4B  CRC-32 of the container (also doubles as the object checksum)
//!   object_len  4B  container length in bytes (F for the RaptorQ OTI)
//!   symbol_size 2B  symbol size T in bytes
//!   sbn         1B  source block number
//!   esi         3B  encoding symbol id (24 bits)
//!   payload     T   one RaptorQ encoding symbol
//!   crc32       4B  CRC-32 over everything before it
//!
//! Every frame is self-describing: a receiver that joins mid-stream can build the
//! RaptorQ decoder from the first frame it manages to decode.

use crate::{Error, Result};

pub const MAGIC: [u8; 3] = *b"SMQ";
pub const PROTO_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 18;
pub const CRC_LEN: usize = 4;
/// Per-frame overhead: header + trailing CRC.
pub const OVERHEAD: usize = HEADER_LEN + CRC_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub session: u32,
    pub object_len: u32,
    pub symbol_size: u16,
    pub sbn: u8,
    pub esi: u32,
}

impl FrameHeader {
    pub fn write_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.push(PROTO_VERSION);
        out.extend_from_slice(&self.session.to_le_bytes());
        out.extend_from_slice(&self.object_len.to_le_bytes());
        out.extend_from_slice(&self.symbol_size.to_le_bytes());
        out.push(self.sbn);
        let esi = self.esi.to_le_bytes();
        out.extend_from_slice(&esi[..3]);
    }

    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < HEADER_LEN {
            return Err(Error::Truncated);
        }
        if body[0..3] != MAGIC {
            return Err(Error::BadMagic);
        }
        if body[3] != PROTO_VERSION {
            return Err(Error::UnsupportedVersion(body[3]));
        }
        Ok(Self {
            session: u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
            object_len: u32::from_le_bytes([body[8], body[9], body[10], body[11]]),
            symbol_size: u16::from_le_bytes([body[12], body[13]]),
            sbn: body[14],
            esi: u32::from_le_bytes([body[15], body[16], body[17], 0]),
        })
    }
}

/// Build a complete frame (header + payload + CRC-32).
pub fn build(header: &FrameHeader, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len() + CRC_LEN);
    header.write_into(&mut out);
    out.extend_from_slice(payload);
    let crc = crate::crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Parse a frame, verifying its CRC-32. Returns the header and the symbol payload.
pub fn parse(bytes: &[u8]) -> Result<(FrameHeader, &[u8])> {
    if bytes.len() < HEADER_LEN + CRC_LEN {
        return Err(Error::Truncated);
    }
    let (body, crc_bytes) = bytes.split_at(bytes.len() - CRC_LEN);
    let expected = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
    if crate::crc32(body) != expected {
        return Err(Error::CrcMismatch);
    }
    let header = FrameHeader::parse(body)?;
    Ok((header, &body[HEADER_LEN..]))
}
