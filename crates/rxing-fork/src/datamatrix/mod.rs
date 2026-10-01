#[cfg(feature = "decoders")]
pub mod decoder;
#[cfg(feature = "decoders")]
pub mod detector;
#[cfg(feature = "encoders")]
pub mod encoder;

#[cfg(feature = "decoders")]
mod data_matrix_reader;
#[cfg(feature = "encoders")]
mod data_matrix_writer;
#[cfg(feature = "decoders")]
pub use data_matrix_reader::*;
#[cfg(feature = "encoders")]
pub use data_matrix_writer::*;
