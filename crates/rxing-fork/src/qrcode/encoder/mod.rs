mod block_pair;
pub mod mask_util;
pub mod matrix_util;
mod minimal_encoder;
mod qr_code;
pub mod qrcode_encoder;

pub use block_pair::*;
pub use minimal_encoder::*;
pub use qr_code::*;

#[cfg(test)]
#[cfg(feature = "decoders")]
mod EncoderTestCase;
#[cfg(test)]
mod MaskUtilTestCase;
#[cfg(test)]
mod QRCodeTestCase;
#[cfg(test)]
mod bit_vector_testcase;
#[cfg(test)]
mod matrix_util_testcase;
