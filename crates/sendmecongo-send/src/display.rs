//! Monitor enumeration, so the player can be placed on a chosen display.
//!
//! minifb has no fullscreen and no monitor API, which means "fullscreen on monitor N" has
//! to be built out of "borderless window sized to that monitor, positioned at its origin".
//! That needs the monitor's logical bounds, which is the entire job of this module.
//!
//! The macOS path is implemented and exercised; other platforms fall back to a single
//! assumed display and the GUI lets the operator override the origin by hand.

#[derive(Debug, Clone)]
pub struct Display {
    pub label: String,
    /// Logical (not physical) origin — that is what window placement consumes.
    pub x: isize,
    pub y: isize,
    pub width: usize,
    pub height: usize,
    pub primary: bool,
}

impl Display {
    /// Window geometry that fills this display while keeping each QR square.
    ///
    /// Returns `(total_width, origin)`: the width to hand to the player, and where the
    /// window should sit. Height is implied (`width / lanes`), because a QR must stay
    /// square — on a 1920×1080 screen a dual-lane stream therefore fills 1920×960 and the
    /// rest is centred off.
    pub fn fit(&self, lanes: usize) -> (usize, (isize, isize)) {
        let lanes = lanes.max(1);
        let tile = self.height.min(self.width / lanes).max(64);
        let win_w = tile * lanes;
        let win_h = tile;
        let x = self.x + ((self.width as isize - win_w as isize) / 2).max(0);
        let y = self.y + ((self.height as isize - win_h as isize) / 2).max(0);
        (win_w, (x, y))
    }
}

#[cfg(target_os = "macos")]
pub fn list() -> Vec<Display> {
    use core_graphics::display::CGDisplay;

    let mut out = Vec::new();
    if let Ok(ids) = CGDisplay::active_displays() {
        let main_id = CGDisplay::main().id;
        for (i, id) in ids.iter().enumerate() {
            let display = CGDisplay::new(*id);
            let bounds = display.bounds();
            let (w, h) = (bounds.size.width as usize, bounds.size.height as usize);
            if w < 64 || h < 64 {
                continue;
            }
            out.push(Display {
                label: sendmecongo_ui::i18n::fill(
                    sendmecongo_ui::i18n::t().snd_display_label,
                    &[&(i + 1), &w, &h],
                ),
                x: bounds.origin.x as isize,
                y: bounds.origin.y as isize,
                width: w,
                height: h,
                primary: *id == main_id,
            });
        }
    }
    if out.is_empty() {
        out.push(fallback());
    }
    out
}

#[cfg(not(target_os = "macos"))]
pub fn list() -> Vec<Display> {
    vec![fallback()]
}

/// Used when enumeration is unavailable: one assumed display. The GUI exposes a manual
/// origin override so a second monitor is still reachable by typing its coordinates.
fn fallback() -> Display {
    Display {
        label: sendmecongo_ui::i18n::t().snd_display_fallback.to_string(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        primary: true,
    }
}
