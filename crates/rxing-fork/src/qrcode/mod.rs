#[cfg(feature = "decoders")]
pub mod decoder;
#[cfg(feature = "decoders")]
pub mod detector;
#[cfg(feature = "encoders")]
pub mod encoder;

#[cfg(feature = "decoders")]
mod qr_code_reader;
#[cfg(feature = "decoders")]
pub use qr_code_reader::*;

#[cfg(feature = "encoders")]
mod qr_code_writer;
#[cfg(feature = "encoders")]
pub use qr_code_writer::*;

pub mod cpp_port;

#[cfg(test)]
#[cfg(all(feature = "image", feature = "encoders", feature = "decoders"))]
mod QRCodeWriterTestCase;

pub mod common;
