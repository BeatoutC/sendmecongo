//! Window geometry shared by both apps.
//!
//! The redesigned screens are a fixed content column, not a fluid one: the settings
//! list, the drop card and the recording-time callout all want their own height, and
//! whatever the window cannot show ends up below the fold. A single hard-coded default
//! cannot serve both a 27" display and a 13" laptop — too small and the bottom of the
//! page is invisible (which is exactly how this module came to exist), too large and
//! the window hangs off the screen, where it cannot even be dragged back into view.
//!
//! So the window states what it wants and the monitor gets the final say.

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};

/// One process, one window: the flag only has to survive repeated `update` calls.
static SIZED: AtomicBool = AtomicBool::new(false);

/// Apply the ideal window size, clamped to the monitor it lands on.
///
/// Call once per frame from `update`; it acts on the first frame where egui knows the
/// monitor geometry and then never again, so a window the user has resized stays put.
pub fn autosize(ctx: &egui::Context, ideal: [f32; 2]) {
    if SIZED.load(Ordering::Relaxed) {
        return;
    }
    let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
        return;
    };

    // The point size here, not the pixel size: egui reports the monitor in the same
    // units `ViewportBuilder::with_inner_size` takes, so no scaling arithmetic is needed.
    // The margins leave room for the menu bar and the Dock — a window taller than the
    // screen hides its own bottom edge, and the bottom edge is where the content is.
    let width = ideal[0].clamp(560.0, (monitor.x - 40.0).max(560.0));
    let height = ideal[1].clamp(420.0, (monitor.y - 140.0).max(420.0));
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
        width, height,
    )));
    SIZED.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    /// The clamp is the whole point of the module, so it is worth pinning down
    /// independently of egui: a 1024×640 monitor must not get a 1000×800 window.
    fn clamp(ideal: [f32; 2], monitor: [f32; 2]) -> [f32; 2] {
        [
            ideal[0].clamp(560.0, (monitor[0] - 40.0).max(560.0)),
            ideal[1].clamp(420.0, (monitor[1] - 140.0).max(420.0)),
        ]
    }

    #[test]
    fn a_large_monitor_gets_the_ideal_size() {
        assert_eq!(clamp([1000.0, 800.0], [2048.0, 2304.0]), [1000.0, 800.0]);
    }

    #[test]
    fn a_short_monitor_gets_a_shorter_window() {
        assert_eq!(clamp([1000.0, 800.0], [1440.0, 900.0]), [1000.0, 760.0]);
    }

    #[test]
    fn a_tiny_monitor_still_gets_a_usable_window() {
        // 800×600 is the floor: below the minimum size the window would be
        // unusable, at which point hanging off the screen is the lesser evil.
        assert_eq!(clamp([1000.0, 800.0], [800.0, 600.0]), [760.0, 460.0]);
    }
}
