//! The window icon.
//!
//! egui installs *no* icon by default, and on Windows there is nothing behind that to
//! fall back on: a bare `.exe` whose window sets none gets the generic application glyph
//! in the title bar and on the taskbar, even though Explorer draws the real one for the
//! file itself. macOS is the opposite — the `.app` carries its own `.icns`, and an icon
//! passed here would *replace* it — so the decode only happens on Windows.
//!
//! The bytes are the same `app.ico` the build script embeds as a Windows resource, which
//! keeps that file the single source of truth for both the file icon and the window icon.

use eframe::egui;

/// The icon for [`egui::ViewportBuilder::with_icon`], decoded from an `.ico`.
///
/// Returns egui's empty icon if the container cannot be read, which leaves whatever the
/// platform would draw on its own rather than refusing to open a window. The tests below
/// pin the decode, so that path is not reached with the icons this workspace ships.
pub fn window_icon(ico: &[u8]) -> egui::IconData {
    #[cfg(target_os = "windows")]
    {
        from_ico(ico).unwrap_or_default()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = ico;
        egui::IconData::default()
    }
}

/// The largest frame of an `.ico` container, as the RGBA bitmap egui wants.
///
/// `image` is already in the tree — `sendmecongo-core` depends on this exact version with
/// its default features — so the ICO decoder costs no extra download and no extra build.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn from_ico(ico: &[u8]) -> Option<egui::IconData> {
    let rgba = image::load_from_memory_with_format(ico, image::ImageFormat::Ico)
        .ok()?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::from_ico;

    /// The two apps ship different icons; both have to decode.
    const SEND: &[u8] = include_bytes!("../../sendmecongo-send/app.ico");
    const RECV: &[u8] = include_bytes!("../../sendmecongo-recv/app.ico");

    #[test]
    fn both_shipped_icons_decode_to_the_largest_frame() {
        for (name, bytes) in [("send", SEND as &[u8]), ("recv", RECV as &[u8])] {
            let icon = from_ico(bytes).unwrap_or_else(|| panic!("{name}: app.ico did not decode"));
            // 256 is the largest frame either container carries.
            assert_eq!((icon.width, icon.height), (256, 256), "{name}");
            assert_eq!(icon.rgba.len(), 256 * 256 * 4, "{name}");
        }
    }

    #[test]
    fn the_decoded_frame_is_visible_rather_than_blank() {
        // A decode to transparent black would satisfy the size assertions above and still
        // leave an invisible icon, which is the failure worth guarding against.
        let icon = from_ico(SEND).expect("send app.ico");
        assert!(
            icon.rgba.chunks_exact(4).any(|px| px[3] > 0),
            "every pixel is transparent"
        );
    }
}
