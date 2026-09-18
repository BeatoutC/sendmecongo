//! Full-screen QR stream player.
//!
//! Shared by the CLI (`sendmecongo-bench play`) and the GUI (`sendmecongo-send --play`) so both
//! run exactly the code path that was measured on the optical link.
//!
//! minifb has no fullscreen API. A borderless window sized to the target monitor and
//! positioned at that monitor's origin is visually identical — that is what `origin` is
//! for, and it is also how multi-monitor selection works on both macOS and Windows.

use crate::preset::Preset;
use crate::{qr, Error, Result, Sender};
use minifb::{Key, Window, WindowOptions};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct PlayOptions {
    /// Total window width in px. Each QR gets `size / lanes` and stays square.
    pub size: usize,
    /// 0 = loop until the user stops it.
    pub cycles: usize,
    /// Overrides the preset's lane count.
    pub lanes: Option<usize>,
    /// Top-left corner of the target monitor, for multi-monitor placement.
    pub origin: Option<(isize, isize)>,
    pub borderless: bool,
    pub hide_cursor: bool,
    pub topmost: bool,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            size: 1600,
            cycles: 0,
            lanes: None,
            origin: None,
            borderless: false,
            hide_cursor: false,
            topmost: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayStats {
    pub emitted: usize,
    pub wall_secs: f64,
    pub sym_per_sec: f64,
    pub frames_per_cycle: usize,
    pub lanes: usize,
    /// Pixel side of one QR, after the quiet zone is taken out.
    pub tile: usize,
    /// Quiet-zone margin on each side of a QR.
    pub pad: usize,
    pub window: (usize, usize),
}

impl PlayStats {
    /// Did the renderer fall short of the preset's target symbol rate? Only meaningful
    /// once the sample is long enough that startup cost isn't distorting the average.
    pub fn kept_up(&self, target_fps: f64) -> bool {
        self.emitted > 120 && self.sym_per_sec < target_fps * 0.9
    }
}

/// Stream `data` as an endless QR loop until ESC or the window is closed.
pub fn play(data: &[u8], name: &str, preset: &Preset, opts: &PlayOptions) -> Result<PlayStats> {
    let (method, compressed) = crate::compress::best(data);
    let object = crate::container::encode(name, data, method, &compressed);
    play_object(&object, preset, opts)
}

/// Stream an SMC1 container that was built upstream.
///
/// The GUI compresses the file once, while the operator is still looking at the
/// settings, and then starts the player with those exact bytes (`--object-stdin`).
/// Nothing here re-compresses, and the window appears as soon as the fountain
/// coding is done — which is roughly 65 ms per megabyte.
pub fn play_object(object: &[u8], preset: &Preset, opts: &PlayOptions) -> Result<PlayStats> {
    let sender = Sender::from_object(object, preset.symbol_size(), preset.repair_pct)?;
    let frames = sender.frames();
    if frames.is_empty() {
        return Err(Error::Other("nothing to play: empty frame list".into()));
    }

    let lanes = opts.lanes.unwrap_or(preset.lanes as usize).max(1);
    let tile = (opts.size / lanes).max(1);
    let win_w = tile * lanes;
    let win_h = tile;
    // QR spec wants 4 modules of blank margin. Without it the detector has to infer the
    // symbol boundary from surrounding content, which collapses on the slightest drift.
    let pad = (tile / 20).max(4);
    let inner = (tile - 2 * pad).max(8);

    let mut buffer = vec![0xFFFF_FFFFu32; win_w * win_h];
    // The container carries the original file name, so the title costs one header
    // parse rather than a second copy of the payload.
    let name = crate::container::peek_name(object).unwrap_or_else(|_| "payload.bin".to_string());
    let title = format!("sendmecongo — {} — {}", preset.name, name);
    let mut window = Window::new(
        &title,
        win_w,
        win_h,
        WindowOptions {
            borderless: opts.borderless,
            topmost: opts.topmost,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| Error::Other(format!("window: {e}")))?;

    if let Some((x, y)) = opts.origin {
        window.set_position(x, y);
    }
    if opts.hide_cursor {
        window.set_cursor_visibility(false);
    }

    let interval = Duration::from_secs_f64(1.0 / preset.fps);
    let limit = if opts.cycles > 0 {
        opts.cycles * frames.len()
    } else {
        usize::MAX
    };

    let started = Instant::now();
    let mut next_at = Instant::now();
    let mut emitted = 0usize;

    while window.is_open() && !window.is_key_down(Key::Escape) && emitted < limit {
        let now = Instant::now();
        if now >= next_at {
            let lane = emitted % lanes;
            qr::blit(
                &frames[emitted % frames.len()],
                preset.version,
                preset.ec,
                inner,
                &mut buffer,
                win_w,
                tile * lane + pad,
                pad,
            )?;
            emitted += 1;
            next_at = (next_at + interval).max(now);
            // Push only when the frame changed: update_with_buffer re-uploads the whole
            // buffer, and calling it at polling rate starves the renderer.
            window
                .update_with_buffer(&buffer, win_w, win_h)
                .map_err(|e| Error::Other(format!("present: {e}")))?;
        } else {
            window.update();
        }
        std::thread::sleep(Duration::from_micros(500));
    }

    let wall_secs = started.elapsed().as_secs_f64();
    Ok(PlayStats {
        emitted,
        wall_secs,
        sym_per_sec: if wall_secs > 0.0 {
            emitted as f64 / wall_secs
        } else {
            0.0
        },
        frames_per_cycle: frames.len(),
        lanes,
        tile: inner,
        pad,
        window: (win_w, win_h),
    })
}
