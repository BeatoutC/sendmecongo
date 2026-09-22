//! SMR1 repair codes — the typeable answer to "what are you still missing?".
//!
//! The receiver shows one of these on a partial receive; the operator types it
//! into the sender on the isolated machine, and the sender replays only fresh
//! repair symbols instead of the whole broadcast (M2-DESIGN §1.4).
//!
//! Wire format, before base32:
//!
//! ```text
//! offset  size  field
//! 0       1     version = 1
//! 1       4     session     = CRC-32(container), LE
//! 5       4     object_len  = container length F, LE
//! 9       2     symbol_size = T, LE
//! 11      var   deficit per source block, LEB128 varint × Z
//! last    1     crc-8 (poly 0x07) over everything before it
//! ```
//!
//! Z is derived from (F, T) exactly like the codec does, so it costs no bits.
//! A single-block transfer — anything under ~63 MB at the turbo presets — lands
//! at 13 bytes, which is 21 base32 characters: short enough to copy by hand.
//! The alphabet is Crockford's (no I/L/O/U) and decoding maps the lookalikes
//! back, because this string will be read off a phone screen.

use crate::progress::source_block_sizes;
use crate::{Error, Result};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// What the sender needs to replay exactly the missing remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairRequest {
    pub session: u32,
    pub object_len: u32,
    pub symbol_size: u16,
    /// Fresh repair symbols wanted per source block. The receiver's safety
    /// margin is already baked into these numbers.
    pub deficits: Vec<u32>,
}

impl RepairRequest {
    pub fn new(session: u32, object_len: u32, symbol_size: u16, deficits: Vec<u32>) -> Self {
        Self {
            session,
            object_len,
            symbol_size,
            deficits,
        }
    }

    /// Total fresh repair symbols across all blocks.
    pub fn total(&self) -> u32 {
        self.deficits.iter().sum()
    }

    /// `SMR1-XXXX-XXXX-…` — the form meant to be read and typed by a person.
    pub fn encode(&self) -> String {
        let mut payload = Vec::with_capacity(16);
        payload.push(1u8);
        payload.extend_from_slice(&self.session.to_le_bytes());
        payload.extend_from_slice(&self.object_len.to_le_bytes());
        payload.extend_from_slice(&self.symbol_size.to_le_bytes());
        for deficit in &self.deficits {
            write_varint(&mut payload, *deficit);
        }
        let crc = crc8(&payload);
        payload.push(crc);

        let chars = base32_encode(&payload);
        let mut out = String::from("SMR1");
        for (index, c) in chars.iter().enumerate() {
            if index % 4 == 0 {
                out.push('-');
            }
            out.push(*c as char);
        }
        out
    }

    /// Tolerant parser: case, spaces and dashes are ignored, the `SMR1` prefix
    /// is optional, and O/I/L are mapped to the lookalike digits.
    pub fn decode(text: &str) -> Result<Self> {
        let mut cleaned: Vec<u8> = text
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase() as u8)
            .collect();
        if cleaned.starts_with(b"SMR1") {
            cleaned.drain(..4);
        }
        if cleaned.len() < 12 {
            return Err(Error::Other("repair code too short".into()));
        }
        let payload = base32_decode(&cleaned)?;
        if payload.len() < 12 {
            return Err(Error::Other("repair code too short".into()));
        }
        let (body, crc) = payload.split_at(payload.len() - 1);
        if crc8(body) != crc[0] {
            return Err(Error::Other("repair code crc mismatch".into()));
        }
        if body[0] != 1 {
            return Err(Error::Other(format!(
                "unsupported repair code version {}",
                body[0]
            )));
        }
        let session = u32::from_le_bytes(body[1..5].try_into().unwrap());
        let object_len = u32::from_le_bytes(body[5..9].try_into().unwrap());
        let symbol_size = u16::from_le_bytes(body[9..11].try_into().unwrap());
        let blocks = source_block_sizes(object_len as u64, symbol_size);
        let mut cursor = 11usize;
        let mut deficits = Vec::with_capacity(blocks.len());
        for _ in 0..blocks.len() {
            let (value, next) = read_varint(body, cursor)
                .ok_or_else(|| Error::Other("repair code truncated".into()))?;
            deficits.push(value);
            cursor = next;
        }
        if cursor != body.len() {
            return Err(Error::Other("repair code length mismatch".into()));
        }
        Ok(Self {
            session,
            object_len,
            symbol_size,
            deficits,
        })
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(data: &[u8], mut cursor: usize) -> Option<(u32, usize)> {
    let mut value = 0u32;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(cursor)?;
        value |= ((byte & 0x7f) as u32) << shift;
        cursor += 1;
        if byte & 0x80 == 0 {
            return Some((value, cursor));
        }
        shift += 7;
        if shift >= 32 {
            return None;
        }
    }
}

/// CRC-8/SMBUS: poly 0x07, init 0x00, no reflection. One byte is plenty for a
/// string whose entire job is catching transcription typos.
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { crc << 1 ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

fn base32_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 8 / 5 + 1);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in data {
        acc = (acc << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[(acc >> bits) as usize & 31]);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[(acc << (5 - bits)) as usize & 31]);
    }
    out
}

fn base32_decode(chars: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(chars.len() * 5 / 8);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in chars {
        let value = match c {
            b'0' | b'O' => 0,
            b'1' | b'I' | b'L' => 1,
            b'2'..=b'9' => c - b'0',
            b'A'..=b'H' => c - b'A' + 10,
            b'J' | b'K' => c - b'J' + 18,
            b'M' | b'N' => c - b'M' + 20,
            b'P'..=b'T' => c - b'P' + 22,
            b'V'..=b'Z' => c - b'V' + 27,
            _ => {
                return Err(Error::Other(format!(
                    "bad character {:?} in repair code",
                    *c as char
                )))
            }
        } as u32;
        acc = (acc << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RepairRequest {
        RepairRequest::new(0x0badf00d, 52_428_800, 1168, vec![13_907])
    }

    #[test]
    fn a_code_roundtrips_and_stays_typeable() {
        let code = sample().encode();
        assert!(code.starts_with("SMR1-"), "{code}");
        // 14 payload bytes (2-byte deficit varint) -> 23 chars: one short line,
        // copyable off a phone.
        let chars = code.chars().filter(|c| c.is_ascii_alphanumeric()).count() - 4;
        assert_eq!(chars, 23, "{code}");
        assert_eq!(RepairRequest::decode(&code).unwrap(), sample());
    }

    #[test]
    fn decoding_tolerates_people() {
        let code = sample().encode();
        // lowercase, no dashes, no prefix, and the classic O/0 I/1 mixups.
        let mangled = code
            .trim_start_matches("SMR1-")
            .replace('-', "")
            .to_lowercase()
            .replace('0', "o")
            .replace('1', "i");
        assert_eq!(RepairRequest::decode(&mangled).unwrap(), sample());
    }

    #[test]
    fn a_typo_is_caught_by_the_crc() {
        let code = sample().encode();
        let mut typo = code.clone().into_bytes();
        let at = 6;
        typo[at] = if typo[at] == b'Q' { b'R' } else { b'Q' };
        let typo = String::from_utf8(typo).unwrap();
        assert!(RepairRequest::decode(&typo).is_err(), "{typo}");
    }

    #[test]
    fn multi_block_deficits_roundtrip() {
        // 64 MB at T=1168 is two source blocks.
        let request = RepairRequest::new(7, 64 * 1024 * 1024, 1168, vec![100, 3]);
        assert_eq!(request.deficits.len(), 2);
        assert_eq!(RepairRequest::decode(&request.encode()).unwrap(), request);
    }

    #[test]
    fn large_deficits_use_multi_byte_varints() {
        let request = RepairRequest::new(7, 52_428_800, 1168, vec![50_000]);
        assert_eq!(RepairRequest::decode(&request.encode()).unwrap(), request);
    }
}
