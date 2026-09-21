//! Monitor enumeration, so the player can be placed on a chosen display.
//!
//! minifb has no fullscreen and no monitor API, which means "fullscreen on monitor N" has
//! to be built out of "borderless window sized to that monitor, positioned at its origin".
//! That needs the monitor's logical bounds, which is the entire job of this module.
//!
//! Coordinate spaces per platform — the enumeration and the window placement must
//! speak the same one, or a scaled monitor shifts the window off-screen:
//! - macOS: CoreGraphics bounds are in logical points; minifb's `set_position` is
//!   points too. Consistent by default.
//! - Windows: minifb's `set_position` is a bare `SetWindowPos`, i.e. raw pixels in
//!   whatever virtualisation the process DPI awareness dictates. Both the GUI and
//!   the player child therefore call `ensure_dpi_awareness()` (the GUI would get
//!   per-monitor awareness from winit anyway; the player never runs winit and
//!   without this is DPI-*unaware* — its coordinates would be virtualised to the
//!   96-DPI space while the GUI enumerates physical pixels, and on any scaled
//!   monitor the window lands in the wrong place and renders blurry). With
//!   awareness set on both sides, everything is physical pixels.

#[derive(Debug, Clone)]
pub struct Display {
    pub label: String,
    /// Logical (not physical) origin — that is what window placement consumes.
    /// (On Windows this is physical pixels; the name is inherited from the macOS
    /// path where points ≠ pixels.)
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

/// One enumerated monitor as `(x, y, w, h, primary)` — the shape both platform
/// backends reduce their APIs to, so the ordering/dedupe rules below are shared
/// and testable off-platform.
type RawMonitor = (isize, isize, usize, usize, bool);

/// Ordering and de-duplication shared by every backend. Enumeration APIs give no
/// useful order, so sort primary-first then left-to-right / top-to-bottom: index
/// 0 is the sensible default output, and labels stay stable across runs. Mirrored
/// ("duplicate these displays") setups report the same rect once per physical
/// monitor; they are one output, so keep the first copy only.
fn normalize(mut raw: Vec<RawMonitor>) -> Vec<RawMonitor> {
    raw.retain(|&(_, _, w, h, _)| w >= 64 && h >= 64);
    raw.sort_by_key(|&(x, y, _, _, primary)| (!primary, y, x));
    raw.dedup_by_key(|r| (r.0, r.1, r.2, r.3));
    raw
}

fn entry(i: usize, &(x, y, w, h, primary): &RawMonitor) -> Display {
    Display {
        label: sendmecongo_ui::i18n::fill(
            sendmecongo_ui::i18n::t().snd_display_label,
            &[&(i + 1), &w, &h],
        ),
        x,
        y,
        width: w,
        height: h,
        primary,
    }
}

#[cfg(target_os = "macos")]
pub fn list() -> Vec<Display> {
    use core_graphics::display::CGDisplay;

    let mut raw: Vec<RawMonitor> = Vec::new();
    if let Ok(ids) = CGDisplay::active_displays() {
        let main_id = CGDisplay::main().id;
        for &id in ids.iter() {
            let bounds = CGDisplay::new(id).bounds();
            raw.push((
                bounds.origin.x as isize,
                bounds.origin.y as isize,
                bounds.size.width as usize,
                bounds.size.height as usize,
                id == main_id,
            ));
        }
    }
    let mut out: Vec<Display> = normalize(raw)
        .into_iter()
        .enumerate()
        .map(|(i, r)| entry(i, &r))
        .collect();
    if out.is_empty() {
        out.push(fallback());
    }
    out
}

/// Windows: `EnumDisplayMonitors` + `GetMonitorInfoW`. Requires per-monitor DPI
/// awareness (see the module docs) — otherwise the rects come back virtualised
/// to the 96-DPI space and don't match the player's `SetWindowPos` either.
#[cfg(target_os = "windows")]
pub fn list() -> Vec<Display> {
    use std::mem;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

    // The enum callback receives an hdc and rect we don't need; the interesting
    // bits (bounds, primary flag) come from GetMonitorInfoW per monitor.
    unsafe extern "system" fn collect(
        _hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: isize,
    ) -> i32 {
        let out = &mut *(lparam as *mut Vec<RawMonitor>);
        let mut info: MONITORINFO = mem::zeroed();
        info.cbSize = mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(_hmon, &mut info) != 0 {
            out.push((
                info.rcMonitor.left as isize,
                info.rcMonitor.top as isize,
                (info.rcMonitor.right - info.rcMonitor.left) as usize,
                (info.rcMonitor.bottom - info.rcMonitor.top) as usize,
                info.dwFlags & MONITORINFOF_PRIMARY != 0,
            ));
        }
        1 // keep enumerating
    }

    let mut raw: Vec<RawMonitor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut raw as *mut _ as isize,
        );
    }
    let mut out: Vec<Display> = normalize(raw)
        .into_iter()
        .enumerate()
        .map(|(i, r)| entry(i, &r))
        .collect();
    if out.is_empty() {
        out.push(fallback());
    }
    out
}

/// Opt the process into per-monitor-v2 DPI awareness. Must run before any window
/// or monitor query in that process. On the GUI path winit would do this anyway;
/// the player child never runs winit, and the two processes must agree on the
/// coordinate space (module docs). Idempotent: fails harmlessly if already set.
#[cfg(target_os = "windows")]
pub fn ensure_dpi_awareness() {
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitors_sort_primary_first_then_left_to_right() {
        let raw = vec![
            (1920, 0, 1920usize, 1080usize, false), // right of primary
            (0, 0, 3840, 2160, true),               // primary
            (960, -1080, 1920, 1080, false),        // above, straddling
        ];
        let out = normalize(raw);
        assert_eq!(out[0], (0, 0, 3840, 2160, true));
        assert_eq!(out[1], (960, -1080, 1920, 1080, false));
        assert_eq!(out[2], (1920, 0, 1920, 1080, false));
    }

    /// "Duplicate these displays" reports one rect per adapter; the stream has a
    /// single output, so the mirror twin must not survive as a phantom monitor 2.
    #[test]
    fn mirrored_duplicates_collapse_into_one_entry() {
        let raw = vec![
            (0, 0, 1920, 1080, true),
            (0, 0, 1920, 1080, false), // same rect via the second adapter
        ];
        assert_eq!(normalize(raw).len(), 1);
    }

    /// Tiny rects (headless adapters, render-only devices) are unusable as a QR
    /// canvas and must not shift the numbering of the real ones.
    #[test]
    fn degenerate_monitors_are_dropped() {
        let raw = vec![(0, 0, 1920, 1080, true), (1920, 0, 0, 0, false)];
        let out = normalize(raw);
        assert_eq!(out.len(), 1);
        assert!(out[0].4);
    }

    /// `fit` is pure math and shared by every platform: the QR tile must stay
    /// square, so on 1920×1080 two lanes fill 1920×960 centred vertically.
    #[test]
    fn fit_keeps_qr_tiles_square() {
        let d = Display {
            label: String::new(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            primary: true,
        };
        let (w, origin) = d.fit(2);
        assert_eq!(w, 1920);
        assert_eq!(origin, (0, 60));
    }
}
