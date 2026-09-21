//! sendmecongo GUI sender — one binary, two modes.
//!
//!     sendmecongo-send                    configuration GUI
//!     sendmecongo-send --play <args>      the QR player
//!
//! The GUI runs the player as a child process of itself. Two reasons: minifb and the GUI
//! toolkit both insist on owning the main-thread event loop, and a freshly spawned window
//! is created with exactly the geometry and monitor placement we asked for instead of
//! fighting whatever state an existing window is in.
//!
//! On Windows the binary is a *windows-subsystem* application: double-clicking it must
//! not park a console window behind the settings screen, and starting the player must
//! not paint a black box onto the monitor we are about to fill with QR codes. The
//! `--play` mode re-attaches to the console it was launched from, so scripting still
//! gets its output, and all printing here is failure-tolerant because a detached
//! process has nowhere to write.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod display;
mod prepare;

use sendmecongo_core::preset;
use eframe::egui;
use std::io::Read;

fn main() -> eframe::Result<()> {
    attach_parent_console();
    // Before any window or monitor query, in BOTH modes: the GUI enumerates
    // monitors and the player child places its window from those coordinates, so
    // the two processes must share one coordinate space. The player never runs
    // winit (which would set this itself on the GUI path) — without it a scaled
    // monitor virtualises the child's coordinates and the QR window lands wrong
    // and renders blurry. See display.rs module docs.
    #[cfg(windows)]
    display::ensure_dpi_awareness();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let playing = args.first().map(String::as_str) == Some("--play");
    // Before anything can print or draw: the player is a short-lived child of the GUI and
    // has no preference of its own, so only the GUI remembers a choice between runs.
    sendmecongo_ui::i18n::init(lang_arg(&args).as_deref(), !playing);
    if playing {
        std::process::exit(match run_player(&args[1..]) {
            Ok(()) => 0,
            Err(e) => {
                say_err(format!("sendmecongo: {e}"));
                2
            }
        });
    }
    // `--open <file>` preloads a file into the GUI, so it can be launched from a shell
    // (or wired to a file association) without a trip through the file picker.
    let initial = args
        .windows(2)
        .find(|w| w[0] == "--open")
        .map(|w| std::path::PathBuf::from(&w[1]));
    run_gui(initial)
}

/// The tag given to `--lang`, if any. Read before anything else so that even an argument
/// error comes out in the language that was asked for.
fn lang_arg(args: &[String]) -> Option<String> {
    let at = args.iter().position(|arg| arg == "--lang")?;
    args.get(at + 1).cloned()
}

/// Write a line to stdout, tolerating the case where there is no console at all.
/// A bare `println!` panics on a detached handle, which is exactly the situation a
/// windows-subsystem process finds itself in.
fn say(line: String) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{line}");
}

/// Same contract as `say`, but for errors: stderr, not stdout. The GUI pipes the
/// player child's stdout to null (it only reads stderr into the status line), so
/// a failure reported on stdout vanished without a trace — the Windows QR window
/// "disappearing" bug was invisible for exactly that reason (2026-09-21).
fn say_err(line: String) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// Windows only, best-effort: a windows-subsystem process has no console of its own,
/// so borrow the one it was launched from. Fails silently when started from Explorer.
fn attach_parent_console() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn run_gui(initial: Option<std::path::PathBuf>) -> eframe::Result<()> {
    // The one-liner for the case that produces a stray console window: the bare
    // executable, which Finder opens in Terminal. Silence from inside a `.app`.
    sendmecongo_ui::launch::note_unbundled("sendmecongo-send");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(app::IDEAL_WINDOW)
            .with_min_inner_size([620.0, 480.0])
            .with_title(sendmecongo_ui::i18n::t().snd_title)
            // eframe installs egui's own logo as the application icon when the app does not
            // provide one — a black tile with a white "e", which is what the Dock showed
            // instead of the app's icon. An empty icon means "no icon": leave the bundle's
            // AppIcon.icns alone. (Windows has no bundle icon to fall back on, so a bare
            // .exe there gets the generic icon; embedding one would need a resource step.)
            .with_icon(egui::IconData::default()),
        ..Default::default()
    };
    eframe::run_native(
        sendmecongo_ui::i18n::t().snd_title,
        options,
        Box::new(move |cc| Ok(Box::new(app::SenderApp::new(cc, initial)))),
    )
}

fn run_player(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    let mut from_stdin = false;
    let mut preset_name = String::from("turbo60");
    let mut size = 1600usize;
    let mut cycles = 0usize;
    let mut lanes: Option<usize> = None;
    let mut origin: Option<(isize, isize)> = None;
    let mut borderless = false;

    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--in" => {
                input = value();
                i += 2;
            }
            "--object-stdin" => {
                from_stdin = true;
                i += 1;
            }
            "--preset" => {
                preset_name = value();
                i += 2;
            }
            "--size" => {
                size = value().parse()?;
                i += 2;
            }
            "--cycles" => {
                cycles = value().parse()?;
                i += 2;
            }
            "--lanes" => {
                lanes = Some(value().parse()?);
                i += 2;
            }
            "--at-x" => {
                let x = value().parse()?;
                origin = Some((x, origin.map(|o| o.1).unwrap_or(0)));
                i += 2;
            }
            "--at-y" => {
                let y = value().parse()?;
                origin = Some((origin.map(|o| o.0).unwrap_or(0), y));
                i += 2;
            }
            "--borderless" => {
                borderless = true;
                i += 1;
            }
            // Accepted and already applied in `main`; the player's own stderr lines
            // follow the same language as the window that spawned it.
            "--lang" => i += 2,
            _ => i += 1,
        }
    }

    let preset =
        preset::by_name(&preset_name).ok_or_else(|| format!("unknown preset: {preset_name}"))?;

    let opts = sendmecongo_core::PlayOptions {
        size,
        cycles,
        lanes,
        origin,
        borderless,
        hide_cursor: true,
        topmost: true,
    };

    // The GUI compresses once and hands over the finished SMC1 container. Nothing is
    // compressed here, so the window opens as soon as the fountain coding finishes.
    let stats = if from_stdin {
        let mut object = Vec::new();
        std::io::stdin().read_to_end(&mut object)?;
        // Fail before opening a window if the pipe carried something else entirely.
        sendmecongo_core::container::peek_name(&object)?;
        sendmecongo_core::play_object(&object, &preset, &opts)?
    } else {
        if input.is_empty() {
            return Err("either --in <file> or --object-stdin is required".into());
        }
        let path = std::path::Path::new(&input);
        let data = std::fs::read(path)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("payload.bin");
        sendmecongo_core::play(&data, name, &preset, &opts)?
    };

    say(format!(
        "emitted {} symbols in {:.1}s → {:.1} sym/s ({} lanes, {}px/tile)",
        stats.emitted, stats.wall_secs, stats.sym_per_sec, stats.lanes, stats.tile
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The player must refuse junk on the pipe before it opens a window, otherwise a
    /// bad hand-off shows up as an empty screen with no explanation.
    #[test]
    fn a_non_container_on_the_pipe_is_rejected_early() {
        assert!(sendmecongo_core::container::peek_name(b"not a container at all").is_err());
        assert!(sendmecongo_core::container::peek_name(&[]).is_err());
    }

    #[test]
    fn an_unknown_preset_is_a_clean_error() {
        assert!(preset::by_name("turbo9000").is_none());
        assert!(preset::by_name("turbo60").is_some());
    }
}
