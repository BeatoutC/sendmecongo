//! Resumable-transfer progress: which encoding symbols the receiver already holds.
//!
//! The fountain code only needs *any* K' distinct symbols, so "resume" is nothing
//! more than carrying the set of already-seen (sbn, esi) pairs across recordings.
//! This module is the on-disk form of that set (`.smr.json`) plus the in-memory
//! bookkeeping the receiver and the (M2.2+) GUI share.
//!
//! Merge/resume key is the triple (session, object_len, symbol_size). The session
//! id alone is *not* enough: it is the CRC-32 of the container, and re-encoding
//! the same file with a different symbol size produces the same session over an
//! incompatible symbol stream. See docs/M2-DESIGN.md §1.2.

use crate::{Error, Result};
use raptorq::{ObjectTransmissionInformation, partition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// `format` field of every manifest we write; unknown values are refused on load.
pub const FORMAT: &str = "smr1";

/// One source block's received ESIs, range-compressed for the disk form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockManifest {
    pub sbn: u8,
    pub received: u32,
    /// Inclusive [start, end] ESI ranges, sorted and non-overlapping.
    pub ranges: Vec<[u32; 2]>,
}

/// The `.smr.json` file. Human-readable, agent-parseable, cheap to carry back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    /// Container CRC-32 as 8 lowercase hex digits (matches how logs print it).
    pub session: String,
    pub object_len: u64,
    pub symbol_size: u16,
    pub blocks: Vec<BlockManifest>,
    /// Recordings that contributed symbols, in the order they were scanned.
    pub recordings: Vec<String>,
    pub updated_unix: u64,
}

/// In-memory progress: the received-ESI sets plus the merge key.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub session: u32,
    pub object_len: u64,
    pub symbol_size: u16,
    blocks: BTreeMap<u8, BTreeSet<u32>>,
}

impl Progress {
    pub fn new(session: u32, object_len: u64, symbol_size: u16) -> Self {
        Self {
            session,
            object_len,
            symbol_size,
            blocks: BTreeMap::new(),
        }
    }

    /// Record one symbol. Returns false if it was already present (duplicate).
    pub fn record(&mut self, sbn: u8, esi: u32) -> bool {
        self.blocks.entry(sbn).or_default().insert(esi)
    }

    /// Distinct received symbols across all blocks.
    pub fn received(&self) -> usize {
        self.blocks.values().map(BTreeSet::len).sum()
    }

    /// Received symbols in one block.
    pub fn received_in_block(&self, sbn: u8) -> usize {
        self.blocks.get(&sbn).map_or(0, BTreeSet::len)
    }

    /// Per-block (sbn, source_k) derived from the OTI, same partition as RFC 6330.
    pub fn source_blocks(&self) -> Vec<(u8, u32)> {
        let oti = ObjectTransmissionInformation::with_defaults(self.object_len, self.symbol_size);
        let k_total = self.object_len.div_ceil(self.symbol_size as u64) as u32;
        let z = oti.source_blocks() as u32;
        let (kl, ks, zl, _zs) = partition(k_total, z);
        (0..z)
            .map(|sbn| {
                let k = if sbn < zl { kl } else { ks };
                (sbn as u8, k)
            })
            .collect()
    }

    /// How many more distinct symbols are needed, before any safety margin.
    /// RaptorQ finishes at K' ≈ K plus a handful of overhead symbols; callers
    /// that emit repair symbols should add their own margin (see M2-DESIGN §1.4).
    pub fn needed_estimate(&self) -> usize {
        self.source_blocks()
            .iter()
            .map(|(sbn, k)| (*k as usize).saturating_sub(self.received_in_block(*sbn)))
            .sum()
    }

    /// Absorb another manifest's symbols. The merge key must match exactly;
    /// returns how many new symbols were added.
    pub fn merge(&mut self, other: &Progress) -> Result<usize> {
        if (self.session, self.object_len, self.symbol_size)
            != (other.session, other.object_len, other.symbol_size)
        {
            return Err(Error::ResumeMismatch(format!(
                "session/object_len/symbol_size differ: {:08x}/{}/{} vs {:08x}/{}/{}",
                self.session,
                self.object_len,
                self.symbol_size,
                other.session,
                other.object_len,
                other.symbol_size
            )));
        }
        let mut added = 0;
        for (sbn, esis) in &other.blocks {
            let target = self.blocks.entry(*sbn).or_default();
            for esi in esis {
                if target.insert(*esi) {
                    added += 1;
                }
            }
        }
        Ok(added)
    }

    /// Iterate (sbn, esi) of every recorded symbol — the receiver rebuilds its
    /// dedup set from this.
    pub fn symbols(&self) -> impl Iterator<Item = (u8, u32)> + '_ {
        self.blocks
            .iter()
            .flat_map(|(sbn, esis)| esis.iter().map(move |esi| (*sbn, *esi)))
    }

    pub fn to_manifest(&self, recordings: Vec<String>, updated_unix: u64) -> Manifest {
        Manifest {
            format: FORMAT.to_string(),
            session: format!("{:08x}", self.session),
            object_len: self.object_len,
            symbol_size: self.symbol_size,
            blocks: self
                .blocks
                .iter()
                .map(|(sbn, esis)| BlockManifest {
                    sbn: *sbn,
                    received: esis.len() as u32,
                    ranges: compress_ranges(esis),
                })
                .collect(),
            recordings,
            updated_unix,
        }
    }

    pub fn from_manifest(manifest: &Manifest) -> Result<Self> {
        if manifest.format != FORMAT {
            return Err(Error::ResumeMismatch(format!(
                "unsupported manifest format {:?} (want {FORMAT:?})",
                manifest.format
            )));
        }
        let session = u32::from_str_radix(&manifest.session, 16).map_err(|_| {
            Error::ResumeMismatch(format!("bad session hex {:?}", manifest.session))
        })?;
        let mut progress = Progress::new(session, manifest.object_len, manifest.symbol_size);
        for block in &manifest.blocks {
            let esis = progress.blocks.entry(block.sbn).or_default();
            for [start, end] in &block.ranges {
                if start > end {
                    return Err(Error::ResumeMismatch(format!(
                        "inverted range {start}-{end} in block {}",
                        block.sbn
                    )));
                }
                for esi in *start..=*end {
                    esis.insert(esi);
                }
            }
        }
        Ok(progress)
    }

    pub fn manifest_to_json(manifest: &Manifest) -> Result<String> {
        serde_json::to_string_pretty(manifest).map_err(|e| Error::Other(e.to_string()))
    }

    pub fn manifest_from_json(json: &str) -> Result<Manifest> {
        serde_json::from_str(json).map_err(|e| Error::ResumeMismatch(format!("bad manifest: {e}")))
    }
}

/// Sorted set -> inclusive ranges. Reception is largely sequential, so this
/// collapses tens of thousands of ESIs into a handful of pairs.
fn compress_ranges(esis: &BTreeSet<u32>) -> Vec<[u32; 2]> {
    let mut ranges: Vec<[u32; 2]> = Vec::new();
    for &esi in esis {
        match ranges.last_mut() {
            Some(last) if esi == last[1] + 1 => last[1] = esi,
            _ => ranges.push([esi, esi]),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_collapse_and_expand_losslessly() {
        let mut p = Progress::new(0xdeadbeef, 100_000, 1000);
        for esi in [0, 1, 2, 5, 6, 9] {
            assert!(p.record(0, esi));
        }
        assert!(!p.record(0, 2));
        let manifest = p.to_manifest(vec![], 0);
        assert_eq!(manifest.blocks[0].ranges, vec![[0, 2], [5, 6], [9, 9]]);
        let back = Progress::from_manifest(&manifest).unwrap();
        assert_eq!(back.received(), 6);
        assert!(back.symbols().eq(p.symbols()));
    }

    #[test]
    fn needed_estimate_counts_down_to_zero() {
        // 10 symbols of source data, single block.
        let mut p = Progress::new(1, 10_000, 1000);
        assert_eq!(p.source_blocks(), vec![(0, 10)]);
        assert_eq!(p.needed_estimate(), 10);
        for esi in 0..10 {
            p.record(0, esi);
        }
        assert_eq!(p.needed_estimate(), 0);
    }

    #[test]
    fn merge_refuses_a_mismatched_key() {
        let a = Progress::new(1, 10_000, 1000);
        let b = Progress::new(1, 10_000, 2000); // same file, different preset
        assert!(a.clone().merge(&b).is_err());
    }

    #[test]
    fn json_roundtrip() {
        let mut p = Progress::new(0x0badf00d, 52_428_800, 1168);
        for esi in 0..100 {
            p.record(0, esi * 3);
        }
        let json = Progress::manifest_to_json(&p.to_manifest(vec!["a.mov".into()], 42)).unwrap();
        let back = Progress::from_manifest(&Progress::manifest_from_json(&json).unwrap()).unwrap();
        assert_eq!(back.received(), 100);
        assert!(back.symbols().eq(p.symbols()));
    }
}
