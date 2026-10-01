#[cfg(feature = "decoders")]
pub mod decoder;
#[cfg(feature = "decoders")]
pub mod detector;

#[cfg(feature = "decoders")]
mod maxi_code_reader;

#[cfg(feature = "decoders")]
pub use maxi_code_reader::*;
