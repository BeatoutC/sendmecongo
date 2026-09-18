//! The macOS-style visual layer shared by both apps.
//!
//! One palette (light and dark) plus the drawing primitives every screen is built from:
//! cards, group lists with hairlines, the amber callout, segmented controls, and the
//! button trio (primary / plain / text). egui cannot do real vibrancy or real shadows,
//! so depth comes from fills and 0.5–1 px hairlines — the same approximation the design
//! doc (`docs/ui-redesign/设计方案.md`) specifies.

use eframe::egui;

/// The palette. Light follows macOS Sonoma/Tahoe; dark inverts surfaces and keeps the
/// same semantic hues at their dark-mode values.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub accent: egui::Color32,
    pub window_bg: egui::Color32,
    pub card_bg: egui::Color32,
    pub card_border: egui::Color32,
    pub hairline: egui::Color32,
    pub text_primary: egui::Color32,
    pub text_secondary: egui::Color32,
    pub text_tertiary: egui::Color32,
    pub warn_bg: egui::Color32,
    pub warn_border: egui::Color32,
    pub warn_text: egui::Color32,
    pub warn_sub: egui::Color32,
    pub success: egui::Color32,
    pub error: egui::Color32,
    /// In-progress amber (status dot, busy states). Same hue in both themes.
    pub pending: egui::Color32,
    pub seg_bg: egui::Color32,
    pub control_bg: egui::Color32,
    pub drop_bg: egui::Color32,
    pub drop_border: egui::Color32,
    pub code_bg: egui::Color32,
    pub code_fg: egui::Color32,
}

/// Light and dark palettes for the same token set. Kept next to each other so a change
/// to one is visible next to the other.
const LIGHT: Theme = Theme {
    dark: false,
    accent: egui::Color32::from_rgb(0x00, 0x7A, 0xFF),
    window_bg: egui::Color32::from_rgb(0xF5, 0xF5, 0xF7),
    card_bg: egui::Color32::WHITE,
    card_border: egui::Color32::from_black_alpha(18),
    hairline: egui::Color32::from_black_alpha(20),
    text_primary: egui::Color32::from_rgb(0x1D, 0x1D, 0x1F),
    text_secondary: egui::Color32::from_rgb(0x6E, 0x6E, 0x73),
    text_tertiary: egui::Color32::from_rgb(0xAE, 0xAE, 0xB2),
    warn_bg: egui::Color32::from_rgb(0xFF, 0xF4, 0xE0),
    warn_border: egui::Color32::from_rgb(0xF0, 0xD9, 0xA8),
    warn_text: egui::Color32::from_rgb(0x8A, 0x52, 0x00),
    warn_sub: egui::Color32::from_rgb(0x9A, 0x6A, 0x1E),
    success: egui::Color32::from_rgb(0x28, 0xA7, 0x45),
    error: egui::Color32::from_rgb(0xFF, 0x3B, 0x30),
    pending: egui::Color32::from_rgb(0xFF, 0x9F, 0x0A),
    seg_bg: egui::Color32::from_rgb(0xE9, 0xE9, 0xEB),
    control_bg: egui::Color32::from_black_alpha(13),
    drop_bg: egui::Color32::from_rgb(0xFB, 0xFB, 0xFC),
    drop_border: egui::Color32::from_rgb(0xC7, 0xC7, 0xCC),
    code_bg: egui::Color32::from_rgb(0x1E, 0x1E, 0x1E),
    code_fg: egui::Color32::from_rgb(0xD4, 0xD4, 0xD4),
};

const DARK: Theme = Theme {
    dark: true,
    accent: egui::Color32::from_rgb(0x0A, 0x84, 0xFF),
    window_bg: egui::Color32::from_rgb(0x1E, 0x1E, 0x20),
    card_bg: egui::Color32::from_rgb(0x2C, 0x2C, 0x2E),
    card_border: egui::Color32::from_rgba_premultiplied(26, 26, 26, 26),
    hairline: egui::Color32::from_rgba_premultiplied(26, 26, 26, 26),
    text_primary: egui::Color32::from_rgb(0xF5, 0xF5, 0xF7),
    text_secondary: egui::Color32::from_rgb(0x98, 0x98, 0x9D),
    text_tertiary: egui::Color32::from_rgb(0x63, 0x63, 0x66),
    warn_bg: egui::Color32::from_rgb(0x3A, 0x2E, 0x12),
    warn_border: egui::Color32::from_rgb(0x5C, 0x4A, 0x1F),
    warn_text: egui::Color32::from_rgb(0xFF, 0xD6, 0x0A),
    warn_sub: egui::Color32::from_rgb(0xC9, 0xA5, 0x4B),
    success: egui::Color32::from_rgb(0x30, 0xD1, 0x58),
    error: egui::Color32::from_rgb(0xFF, 0x45, 0x3A),
    pending: egui::Color32::from_rgb(0xFF, 0x9F, 0x0A),
    seg_bg: egui::Color32::from_rgb(0x38, 0x38, 0x3A),
    control_bg: egui::Color32::from_rgba_premultiplied(16, 16, 16, 16),
    drop_bg: egui::Color32::from_rgb(0x25, 0x25, 0x27),
    drop_border: egui::Color32::from_rgb(0x48, 0x48, 0x4A),
    code_bg: egui::Color32::from_rgb(0x14, 0x14, 0x14),
    code_fg: egui::Color32::from_rgb(0xD4, 0xD4, 0xD4),
};

/// Pick the palette that matches the current egui visuals (which follow the system
/// appearance under eframe's default theme preference).
pub fn theme(ctx: &egui::Context) -> Theme {
    if ctx.style().visuals.dark_mode {
        DARK
    } else {
        LIGHT
    }
}

/// Global tweaks that make egui's native widgets (checkbox, combo box, slider, progress
/// bar) sit comfortably inside the palette. Called once at startup by each app.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.visuals.selection.bg_fill = theme(ctx).accent;
    // Native widgets draw their own fills; tint them towards the palette instead of
    // egui's default blue-grey.
    style.visuals.widgets.inactive.weak_bg_fill = theme(ctx).control_bg;
    style.visuals.widgets.hovered.weak_bg_fill = theme(ctx).control_bg;
    ctx.set_style(style);
}

/// The central panel's frame: flat window colour, generous margins.
pub fn window_frame(t: &Theme) -> egui::Frame {
    egui::Frame::default()
        .fill(t.window_bg)
        .inner_margin(egui::Margin::symmetric(22, 16))
}

/// A card with 14 px padding all around (file cards, callouts, stat cards).
pub fn card(t: &Theme) -> egui::Frame {
    egui::Frame::default()
        .fill(t.card_bg)
        .stroke(egui::Stroke::new(1.0_f32, t.card_border))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(14))
}

/// A card with no inner padding, for group lists whose rows manage their own margins
/// and whose hairlines must span the full width.
pub fn card_edge(t: &Theme) -> egui::Frame {
    egui::Frame::default()
        .fill(t.card_bg)
        .stroke(egui::Stroke::new(1.0_f32, t.card_border))
        .corner_radius(12.0)
}

/// The amber callout — reserved for the single most important number on a screen
/// (the recommended recording time) and other advice that must not blend in.
pub fn callout(t: &Theme) -> egui::Frame {
    egui::Frame::default()
        .fill(t.warn_bg)
        .stroke(egui::Stroke::new(1.0_f32, t.warn_border))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(16, 14))
}

/// A full-width hairline, used between rows inside a `card_edge` group list.
pub fn hairline(ui: &mut egui::Ui, t: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.hairline);
}

/// The small caps-ish section label above a group list ("播放设置", "附近的录像").
pub fn section_title(ui: &mut egui::Ui, t: &Theme, text: impl Into<String>) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(text.into()).size(13.0).strong().color(t.text_secondary));
    });
    ui.add_space(-4.0);
}

/// The primary capsule button. `active = false` renders it at reduced opacity but keeps
/// it clickable — a click that explains itself beats a button that looks dead.
pub fn primary_button(
    ui: &mut egui::Ui,
    t: &Theme,
    text: &str,
    big: bool,
    active: bool,
) -> egui::Response {
    let fill = if active {
        t.accent
    } else {
        t.accent.gamma_multiply(0.45)
    };
    let size = if big { 15.0 } else { 13.0 };
    let mut rich = egui::RichText::new(text).size(size).color(egui::Color32::WHITE);
    if big {
        rich = rich.strong();
    }
    let button = egui::Button::new(rich)
        .fill(fill)
        .corner_radius(if big { 10.0 } else { 8.0 })
        .min_size(egui::vec2(0.0, if big { 34.0 } else { 0.0 }));
    ui.add(button)
}

/// Same shape, error colour — the playing state's "stop" button.
pub fn danger_button(ui: &mut egui::Ui, t: &Theme, text: &str, big: bool) -> egui::Response {
    let size = if big { 15.0 } else { 13.0 };
    let mut rich = egui::RichText::new(text).size(size).color(egui::Color32::WHITE);
    if big {
        rich = rich.strong();
    }
    ui.add(
        egui::Button::new(rich)
            .fill(t.error)
            .corner_radius(if big { 10.0 } else { 8.0 })
            .min_size(egui::vec2(0.0, if big { 34.0 } else { 0.0 })),
    )
}

/// The quiet secondary button (grey fill, primary text).
pub fn plain_button(ui: &mut egui::Ui, t: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(13.0).color(t.text_primary))
            .fill(t.control_bg)
            .corner_radius(8.0),
    )
}

/// A borderless text button, macOS style. Default colour is the accent.
pub fn text_button(
    ui: &mut egui::Ui,
    t: &Theme,
    text: &str,
    color: Option<egui::Color32>,
) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(13.0)
                .color(color.unwrap_or(t.accent)),
        )
        .frame(false),
    )
}

/// A macOS segmented control ("发送 / 接收说明", lane counts). Returns true when the
/// selection changed. Labels are owned so dynamic ones ("默认 (2)") work too.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    t: &Theme,
    value: &mut T,
    options: &[(T, String)],
) -> bool {
    let mut changed = false;
    egui::Frame::default()
        .fill(t.seg_bg)
        .corner_radius(9.0)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                for (opt, label) in options {
                    let selected = *value == *opt;
                    let text = egui::RichText::new(label.as_str()).size(13.0).color(if selected {
                        t.text_primary
                    } else {
                        t.text_secondary
                    });
                    let mut frame = egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(14, 4))
                        .corner_radius(7.0);
                    if selected {
                        frame = frame
                            .fill(t.card_bg)
                            .stroke(egui::Stroke::new(0.5_f32, t.card_border));
                    }
                    let response = frame
                        .show(ui, |ui| ui.label(text))
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() && !selected {
                        *value = *opt;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// A small filled circle — the status bar's traffic-light dot.
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// The result badge: a 56 px filled circle with a glyph, for the done/failed heroes.
pub fn badge(ui: &mut egui::Ui, color: egui::Color32, glyph: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 28.0, color);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(24.0),
        egui::Color32::WHITE,
    );
}

/// A small capsule tag (the "影响成败" chip on the tips disclosure).
pub fn chip(ui: &mut egui::Ui, t: &Theme, text: &str) {
    egui::Frame::default()
        .fill(t.warn_bg)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(11.0).color(t.warn_text));
        });
}

/// The dashed-border drop zone. egui strokes are solid, so the dashes are painted by
/// hand along the four edges after the frame has been laid out.
pub fn drop_zone(
    ui: &mut egui::Ui,
    t: &Theme,
    min_height: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let response = egui::Frame::default()
        .fill(t.drop_bg)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(20, 26))
        .show(ui, |ui| {
            ui.set_min_height(min_height);
            ui.vertical_centered(|ui| add_contents(ui));
        });
    paint_dashed_border(ui.painter(), response.response.rect, t.drop_border);
}

fn paint_dashed_border(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    const DASH: f32 = 8.0;
    const GAP: f32 = 5.0;
    const W: f32 = 1.5;
    let stroke = egui::Stroke::new(W, color);
    let edges = [
        (rect.left_top(), rect.right_top()),
        (rect.right_top(), rect.right_bottom()),
        (rect.right_bottom(), rect.left_bottom()),
        (rect.left_bottom(), rect.left_top()),
    ];
    for (from, to) in edges {
        let delta = to - from;
        let len = delta.length();
        let dir = delta / len;
        // Inset a corner-radius's worth at both ends so dashes don't pile into corners.
        let mut d = 12.0;
        while d < len - 12.0 {
            let end = (d + DASH).min(len - 12.0);
            painter.line_segment([from + dir * d, from + dir * end], stroke);
            d = end + GAP;
        }
    }
}

/// A grid of stat cards (the receiver's running screen). Each cell is a card with a
/// big tabular number (plus a small inline denominator) and a caption.
pub fn stat_card(
    ui: &mut egui::Ui,
    t: &Theme,
    number: &str,
    suffix: &str,
    label: &str,
) {
    card(t).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(number)
                    .size(17.0)
                    .strong()
                    .color(t.text_primary),
            );
            if !suffix.is_empty() {
                ui.label(egui::RichText::new(suffix).size(11.0).color(t.text_tertiary));
            }
        });
        ui.label(egui::RichText::new(label).size(11.0).color(t.text_secondary));
    });
}

/// One key/value row inside a result group list.
pub fn kv_row(ui: &mut egui::Ui, t: &Theme, key: &str, value: egui::RichText) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(egui::RichText::new(key).size(13.0).color(t.text_secondary));
        ui.add_space(14.0);
        ui.label(value.size(13.0));
        ui.add_space(14.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both palettes must populate every token — a half-filled table is how one theme
    /// silently falls back to black text on a black card.
    #[test]
    fn palettes_are_distinct_and_complete() {
        assert_ne!(LIGHT.window_bg, DARK.window_bg);
        assert_ne!(LIGHT.card_bg, DARK.card_bg);
        assert_ne!(LIGHT.text_primary, DARK.text_primary);
        // Semantic hues stay recognisable across themes.
        for t in [LIGHT, DARK] {
            assert!(t.accent.b() > t.accent.r(), "accent is a blue");
            assert!(t.error.r() > t.error.g(), "error is a red");
            assert!(t.success.g() >= t.success.r(), "success is a green");
        }
    }

    /// The dark error token is #FF453A — guard against a typo drifting it into orange.
    #[test]
    fn dark_error_is_the_macos_dark_red() {
        assert_eq!(DARK.error, egui::Color32::from_rgb(0xFF, 0x45, 0x3A));
    }
}
