//! RaptorQ (RFC 6330) sender and receiver over SMQ frames.
//!
//! Sender: container -> source + repair symbols -> interleaved -> endlessly cycled frames.
//! Receiver: accepts frames in any order, dedupes by (sbn, esi), decodes when it has enough.

use crate::frame::{self, FrameHeader};
use crate::{compress, container, Error, Result};
use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation, PayloadId};
use std::collections::{BTreeMap, HashSet};

pub struct Sender {
    frames: Vec<Vec<u8>>,
    cursor: usize,
}

impl Sender {
    /// Compress, wrap in a container, and fountain-encode it into a frame list.
    pub fn new(name: &str, data: &[u8], symbol_size: u16, repair_pct: u32) -> Result<Self> {
        let (method, compressed) = compress::best(data);
        let object = container::encode(name, data, method, &compressed);
        Self::from_object(&object, symbol_size, repair_pct)
    }

    /// Fountain-encode an SMC1 container that was built elsewhere.
    ///
    /// The GUI compresses once, when the operator picks the file, and then hands the
    /// resulting container to the player process. Everything downstream of the
    /// container — symbol sizing, repair symbols, frame headers — is identical to
    /// [`Sender::new`], so the wire format does not care where the bytes came from.
    pub fn from_object(object: &[u8], symbol_size: u16, repair_pct: u32) -> Result<Self> {
        if object.is_empty() {
            return Err(Error::Truncated);
        }
        let oti = ObjectTransmissionInformation::with_defaults(object.len() as u64, symbol_size);
        let encoder = Encoder::new(object, oti);

        let source_symbols = object.len().div_ceil(symbol_size as usize).max(1);
        let repair = (source_symbols as u64 * repair_pct as u64 / 100).max(1) as u32;
        let packets = interleave(encoder.get_encoded_packets(repair));

        let session = crate::crc32(object);
        let mut frames = Vec::with_capacity(packets.len());
        for packet in &packets {
            let header = FrameHeader {
                session,
                object_len: object.len() as u32,
                symbol_size,
                sbn: packet.payload_id().source_block_number(),
                esi: packet.payload_id().encoding_symbol_id(),
            };
            frames.push(frame::build(&header, packet.data()));
        }
        Ok(Self { frames, cursor: 0 })
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Next frame in the endless cycle.
    pub fn next_frame(&mut self) -> &[u8] {
        let f = &self.frames[self.cursor];
        self.cursor = (self.cursor + 1) % self.frames.len().max(1);
        f
    }

    pub fn frames(&self) -> &[Vec<u8>] {
        &self.frames
    }
}

/// Round-robin across source blocks so a burst of consecutive loss (camera looking
/// away, a blink, a torn frame run) never wipes out one whole block.
fn interleave(packets: Vec<EncodingPacket>) -> Vec<EncodingPacket> {
    let total = packets.len();
    let mut by_block: BTreeMap<u8, Vec<EncodingPacket>> = BTreeMap::new();
    for packet in packets {
        by_block
            .entry(packet.payload_id().source_block_number())
            .or_default()
            .push(packet);
    }
    let mut out = Vec::with_capacity(total);
    let mut index = 0usize;
    loop {
        let mut added = false;
        for packets in by_block.values_mut() {
            if index < packets.len() {
                out.push(packets[index].clone());
                added = true;
            }
        }
        if !added {
            break;
        }
        index += 1;
    }
    out
}

pub struct Received {
    pub name: String,
    pub data: Vec<u8>,
    pub frames_used: usize,
    pub duplicates: usize,
}

pub struct Receiver {
    decoder: Option<Decoder>,
    session: Option<u32>,
    seen: HashSet<(u8, u32)>,
    frames_used: usize,
    duplicates: usize,
    done: bool,
}

impl Default for Receiver {
    fn default() -> Self {
        Self::new()
    }
}

impl Receiver {
    pub fn new() -> Self {
        Self {
            decoder: None,
            session: None,
            seen: HashSet::new(),
            frames_used: 0,
            duplicates: 0,
            done: false,
        }
    }

    pub fn frames_used(&self) -> usize {
        self.frames_used
    }

    pub fn duplicates(&self) -> usize {
        self.duplicates
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Feed one decoded frame payload. Returns the completed file when the object
    /// has been reconstructed; CRC failures return an error instead of poisoning state.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Option<Received>> {
        if self.done {
            return Ok(None);
        }
        let (header, payload) = frame::parse(bytes)?;

        match self.session {
            None => {
                let oti = ObjectTransmissionInformation::with_defaults(
                    header.object_len as u64,
                    header.symbol_size,
                );
                self.decoder = Some(Decoder::new(oti));
                self.session = Some(header.session);
            }
            Some(session) if session != header.session => return Ok(None),
            Some(_) => {}
        }

        if !self.seen.insert((header.sbn, header.esi)) {
            self.duplicates += 1;
            return Ok(None);
        }
        self.frames_used += 1;

        let packet = EncodingPacket::new(PayloadId::new(header.sbn, header.esi), payload.to_vec());
        let decoder = self.decoder.as_mut().ok_or(Error::Truncated)?;
        let object = match decoder.decode(packet) {
            Some(object) => object,
            None => return Ok(None),
        };

        // Layer 2: the session id is the CRC-32 of the object.
        if crate::crc32(&object) != header.session {
            return Err(Error::CrcMismatch);
        }
        let container = container::decode(&object)?;
        let data = container::into_file(&container)?;
        self.done = true;
        Ok(Some(Received {
            name: container.name,
            data,
            frames_used: self.frames_used,
            duplicates: self.duplicates,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset;

    /// The whole point of `from_object`: the player gets the container the GUI built,
    /// and the resulting stream must be byte-for-byte what the old path produced.
    #[test]
    fn a_container_encodes_to_the_same_frames_as_the_file_did() {
        let data = vec![3u8; 400_000];
        let p = preset::TURBO60;
        let (comp, payload) = compress::best(&data);
        let object = container::encode("x.bin", &data, comp, &payload);

        let direct = Sender::new("x.bin", &data, p.symbol_size(), p.repair_pct).unwrap();
        let handed_over = Sender::from_object(&object, p.symbol_size(), p.repair_pct).unwrap();
        assert_eq!(direct.frames(), handed_over.frames());
    }

    #[test]
    fn an_empty_object_is_refused_instead_of_panicking() {
        assert!(Sender::from_object(&[], 1000, 20).is_err());
    }
}
