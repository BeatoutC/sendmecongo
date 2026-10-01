mod multiple_barcode_reader;

#[cfg(feature = "qrcode")]
pub mod qrcode;
pub use multiple_barcode_reader::*;

mod by_quadrant_reader;
pub use by_quadrant_reader::*;

mod generic_multiple_barcode_reader;
pub use generic_multiple_barcode_reader::*;

#[cfg(test)]
#[cfg(all(
    feature = "image",
    feature = "multi_barcode_readers",
    feature = "qrcode",
    feature = "oned"
))]
mod multi_test_case;
