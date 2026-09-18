//! Chinese-capable font loading.
//!
//! egui ships Latin-only fonts, so every CJK glyph in this UI renders as a tofu box until
//! a font that covers it is registered. Rather than embedding a 10 MB font in the binary
//! we borrow the system's, probing the usual locations per platform and falling back to
//! the default (Latin-only) set if none is found — the UI stays usable either way, it just
//! loses the Chinese labels.

use eframe::egui;
use std::sync::Arc;

#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/Library/Fonts/Arial Unicode.ttf",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\msyhl.ttc",
    "C:\\Windows\\Fonts\\simhei.ttf",
    "C:\\Windows\\Fonts\\simsun.ttc",
];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
];

/// Register the first available CJK font. Returns the path used, if any.
///
/// The proportional family gets it *prepended*, so Latin renders in the same face as
/// the Chinese — egui's bundled Latin face next to PingFang/YaHei is what made mixed
/// lines look off. Monospace only gets it appended, keeping egui's code font for Latin
/// code and falling through for CJK.
pub fn install(ctx: &egui::Context) -> Option<String> {
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        let key = "cjk".to_owned();
        fonts
            .font_data
            .insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, key.clone());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push(key.clone());
        ctx.set_fonts(fonts);
        return Some((*path).to_string());
    }
    None
}

/// Roomier spacing than egui's default; this UI is read at a glance, not pored over.
/// Kept as a thin wrapper so existing call sites keep working — the real styling now
/// lives in [`crate::theme::apply`].
pub fn apply_style(ctx: &egui::Context) {
    crate::theme::apply(ctx);
}
