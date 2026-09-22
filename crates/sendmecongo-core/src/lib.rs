//! sendmecongo-core — SMQ1 optical transfer codec.
//!
//! Pipeline (sender):
//!   file -> [compression] -> container (SMC1) -> RaptorQ (RFC 6330) -> SMQ frames -> QR bitmaps
//!
//! The optical channel is one-way and lossy: there is no ACK and no retransmission.
//! Every frame is self-describing, frames may arrive out of order, and the receiver
//! only needs enough distinct symbols to rebuild the whole object.

pub mod codec;
pub mod compress;
pub mod container;
pub mod frame;
pub mod play;
pub mod preset;
pub mod progress;
pub mod qr;

pub use codec::{Received, Receiver, Sender};
pub use container::Container;
pub use frame::FrameHeader;
pub use play::{play, play_object, PlayOptions, PlayStats};
pub use progress::{Manifest, Progress};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("qr: {0}")]
    Qr(#[from] qrcode::types::QrError),
    #[error("bad magic bytes")]
    BadMagic,
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("unknown compression method: {0}")]
    UnknownCompression(u8),
    #[error("truncated input")]
    Truncated,
    #[error("crc mismatch")]
    CrcMismatch,
    #[error("invalid utf-8 in filename")]
    BadName,
    #[error("cancelled")]
    Cancelled,
    #[error("resume manifest rejected: {0}")]
    ResumeMismatch(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// CRC-32 (IEEE) helper used at every integrity layer.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(bytes);
    h.finalize()
}
