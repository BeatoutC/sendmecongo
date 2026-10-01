#[cfg(feature = "decoders")]
mod aztec_detector_result;
#[cfg(feature = "decoders")]
mod aztec_reader;
#[cfg(feature = "encoders")]
mod aztec_writer;
#[cfg(feature = "decoders")]
pub mod decoder;
#[cfg(feature = "decoders")]
pub mod detector;
#[cfg(feature = "encoders")]
pub mod encoder;

#[cfg(feature = "decoders")]
pub use aztec_reader::*;
#[cfg(feature = "encoders")]
pub use aztec_writer::*;

#[cfg(test)]
#[cfg(feature = "decoders")]
mod DecoderTest;
#[cfg(test)]
#[cfg(all(feature = "decoders", feature = "encoders"))]
mod DetectorTest;
#[cfg(test)]
#[cfg(all(feature = "encoders", feature = "decoders"))]
mod EncoderTest;
