//! QR rendering and capacity calibration (byte mode, single segment, no ECI).

use crate::Result;
use image::{GrayImage, Luma};
use qrcode::types::Color;
use qrcode::{EcLevel, QrCode, Version};

/// Largest byte-mode payload that fits a given version / ECC level.
/// Measured against the encoder itself, so it can never drift from the truth.
///
/// The measurement is expensive — a binary search that runs the encoder ~13 times,
/// each pass evaluating all 8 mask patterns — so the result is cached per
/// `(version, ec)`. It is a pure function of those two arguments; the cache cannot
/// change what it returns. This is not a micro-optimisation: the GUI reads
/// `symbol_size()` several times per frame (every preset string on screen), and
/// without the cache the whole window ran at ~19 fps, which showed up as the preset
/// dropdown's highlight lagging behind the mouse.
pub fn capacity(version: i16, ec: EcLevel) -> usize {
    use std::sync::atomic::{AtomicU16, Ordering};
    use std::sync::OnceLock;

    // Zero means "not measured yet". Legal capacities never reach 65 535, let alone 0.
    static CACHE: OnceLock<[AtomicU16; 4 * 41]> = OnceLock::new();

    let ec_index = match ec {
        EcLevel::L => 0,
        EcLevel::M => 1,
        EcLevel::Q => 2,
        EcLevel::H => 3,
    };
    let Some(cache) = (1..=40)
        .contains(&version)
        .then(|| CACHE.get_or_init(|| std::array::from_fn(|_| AtomicU16::new(0))))
    else {
        // Out of range: mirror the encoder, which rejects these outright.
        return 0;
    };

    let slot = &cache[version as usize * 4 + ec_index];
    let cached = slot.load(Ordering::Relaxed);
    if cached != 0 {
        return cached as usize;
    }
    let measured = measure_capacity(version, ec);
    slot.store(measured as u16, Ordering::Relaxed);
    measured
}

/// The uncached measurement: binary search over the encoder itself.
fn measure_capacity(version: i16, ec: EcLevel) -> usize {
    let mut lo = 0usize;
    let mut hi = 8192usize;
    while lo < hi {
        let mid = (lo + hi + 1).div_ceil(2);
        if QrCode::with_version(vec![0u8; mid], Version::Normal(version), ec).is_ok() {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// Render a payload to a grayscale module matrix, 1 pixel per module, 4-module quiet zone.
pub fn matrix(payload: &[u8], version: i16, ec: EcLevel) -> Result<GrayImage> {
    let code = QrCode::with_version(payload, Version::Normal(version), ec)?;
    let width = code.width();
    let colors = code.to_colors();
    let quiet = 4usize;
    let dim = width + quiet * 2;
    let mut img = GrayImage::from_pixel(dim as u32, dim as u32, Luma([255u8]));
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == Color::Dark {
                img.put_pixel((x + quiet) as u32, (y + quiet) as u32, Luma([0u8]));
            }
        }
    }
    Ok(img)
}

/// Blit a symbol as a square RGBA tile (0xAARRGGBB) into a larger frame buffer.
/// `tile` is the side length in pixels, `(ox, oy)` the top-left corner, `out_w` the
/// stride of the destination. Nearest-neighbour scaling, white background.
pub fn blit(
    payload: &[u8],
    version: i16,
    ec: EcLevel,
    tile: usize,
    out: &mut [u32],
    out_w: usize,
    ox: usize,
    oy: usize,
) -> Result<()> {
    let img = matrix(payload, version, ec)?;
    let modules = img.width() as usize;
    let scale = tile as f32 / modules as f32;
    let map: Vec<usize> = (0..tile)
        .map(|i| ((i as f32 / scale) as usize).min(modules - 1))
        .collect();

    for y in 0..tile {
        let my = map[y] as u32;
        let row = &mut out[(oy + y) * out_w + ox..(oy + y) * out_w + ox + tile];
        for (x, pixel) in row.iter_mut().enumerate() {
            *pixel = if img.get_pixel(map[x] as u32, my)[0] < 128 {
                0xFF00_0000
            } else {
                0xFFFF_FFFF
            };
        }
    }
    Ok(())
}

/// Render to a PNG (nearest-neighbour upscale by `scale`), ready to be shown or saved.
pub fn png(payload: &[u8], version: i16, ec: EcLevel, scale: u32) -> Result<Vec<u8>> {
    let img = matrix(payload, version, ec)?;
    let scaled = image::imageops::resize(
        &img,
        img.width() * scale,
        img.height() * scale,
        image::imageops::FilterType::Nearest,
    );
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    image::DynamicImage::ImageLuma8(scaled).write_to(&mut cursor, image::ImageFormat::Png)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset;

    #[test]
    fn cached_capacity_agrees_with_a_fresh_measurement() {
        for p in preset::ALL {
            for ec in [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H] {
                assert_eq!(
                    capacity(p.version, ec),
                    measure_capacity(p.version, ec),
                    "cache drifted for v{}-{ec:?}",
                    p.version
                );
            }
        }
    }

    /// Values measured on the optical bench (see the daily log): raw byte-mode
    /// capacity for the two versions the presets actually use. The preset's
    /// `symbol_size()` subtracts the 22-byte frame overhead from these.
    #[test]
    fn known_capacities_are_stable() {
        assert_eq!(capacity(30, EcLevel::L), 1732);
        assert_eq!(capacity(40, EcLevel::L), 2953);
    }

    #[test]
    fn out_of_range_versions_reject_instead_of_panicking() {
        assert_eq!(capacity(0, EcLevel::L), 0);
        assert_eq!(capacity(41, EcLevel::L), 0);
        assert_eq!(capacity(-3, EcLevel::L), 0);
    }

    /// The reason the cache exists: reading the preset table must not encode a QR
    /// code. One cold call is allowed; everything after must be a table lookup.
    #[test]
    fn repeated_calls_are_effectively_free() {
        let cold = std::time::Instant::now();
        let expected = capacity(30, EcLevel::L);
        let _ = cold.elapsed();

        let warm = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(capacity(30, EcLevel::L), expected);
        }
        let warm_secs = warm.elapsed().as_secs_f64();
        assert!(
            warm_secs < 1.0,
            "1000 cached calls took {warm_secs:.3}s — the cache is not caching"
        );
    }
}
