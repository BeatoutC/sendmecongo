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
use std::fmt::Write as _;
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

    if args.iter().any(|a| a == "-h" || a == "--help") {
        sendmecongo_ui::i18n::init(lang_arg(&args).as_deref(), false);
        print_usage();
        return Ok(());
    }

    let prepare_only = args.iter().any(|a| a == "--prepare-only");
    let playing = args.iter().any(|a| a == "--play");

    // Before anything can print or draw: the player/cli is short-lived and
    // has no preference of its own, so only the GUI remembers a choice between runs.
    sendmecongo_ui::i18n::init(lang_arg(&args).as_deref(), !playing && !prepare_only);

    if prepare_only {
        let sub_args: Vec<String> = args.iter().filter(|a| *a != "--prepare-only").cloned().collect();
        std::process::exit(match run_prepare(&sub_args) {
            Ok(()) => 0,
            Err(e) => {
                say_err(format!("sendmecongo-send: {e}"));
                1
            }
        });
    }

    if playing {
        let sub_args: Vec<String> = args.iter().filter(|a| *a != "--play").cloned().collect();
        std::process::exit(match run_player(&sub_args) {
            Ok(()) => 0,
            Err(e) => {
                say_err(format!("sendmecongo: {e}"));
                2
            }
        });
    }

    // `--open <file>`, `--file <file>`, or `--in <file>` preloads a file into the GUI,
    // so it can be launched from a shell without a trip through the file picker.
    let initial = args
        .windows(2)
        .find(|w| w[0] == "--open" || w[0] == "--file" || w[0] == "--in")
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
            // instead of the app's icon. On macOS the empty icon is still the answer: it
            // leaves the bundle's AppIcon.icns in charge. Windows has no bundle to fall back
            // on, so there the same app.ico the build script embeds in the .exe is decoded
            // and handed to the window — without it the title bar and the taskbar showed the
            // generic application glyph.
            .with_icon(sendmecongo_ui::icon::window_icon(include_bytes!("../app.ico"))),
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
    let mut repair_code = String::new();

    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--file" | "--in" => {
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
            "--resume-code" | "--repair-code" => {
                repair_code = value();
                i += 2;
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
    let object = if from_stdin {
        let mut object = Vec::new();
        std::io::stdin().read_to_end(&mut object)?;
        // Fail before opening a window if the pipe carried something else entirely.
        sendmecongo_core::container::peek_name(&object)?;
        object
    } else {
        if input.is_empty() {
            return Err("either --file <file>, --in <file> or --object-stdin is required".into());
        }
        let path = std::path::Path::new(&input);
        let data = std::fs::read(path)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("payload.bin");
        let (method, compressed) = sendmecongo_core::compress::best(&data);
        sendmecongo_core::container::encode(name, &data, method, &compressed)
    };

    // M2.3 repair broadcast: only the fresh repair symbols the receiver's code
    // asked for. The code is validated against the container here, before any
    // window opens — a mistyped code must fail on the terminal, not on air.
    let stats = if repair_code.is_empty() {
        sendmecongo_core::play_object(&object, &preset, &opts)?
    } else {
        let request = sendmecongo_core::RepairRequest::decode(&repair_code)
            .map_err(|e| sendmecongo_ui::i18n::fill(sendmecongo_ui::i18n::t().snd_repair_invalid, &[&e]))?;
        let session_matches = sendmecongo_core::crc32(&object) == request.session;
        let size_matches = preset.symbol_size() == request.symbol_size
            && object.len() as u32 == request.object_len;
        if !session_matches || !size_matches {
            return Err(sendmecongo_ui::i18n::t()
                .snd_repair_mismatch
                .to_string()
                .into());
        }
        sendmecongo_core::play_repair(&object, &preset, &opts, &request.deficits)?
    };

    say(format!(
        "emitted {} symbols in {:.1}s → {:.1} sym/s ({} lanes, {}px/tile)",
        stats.emitted, stats.wall_secs, stats.sym_per_sec, stats.lanes, stats.tile
    ));
    Ok(())
}

fn run_prepare(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    let mut preset_name = String::from("turbo60");
    let mut json_target: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--file" | "--in" => {
                input = value();
                i += 2;
            }
            "--preset" => {
                preset_name = value();
                i += 2;
            }
            "--json" => {
                json_target = Some(value());
                i += 2;
            }
            _ => i += 1,
        }
    }

    if input.is_empty() {
        return Err("either --file <path> or --in <path> is required".into());
    }

    let preset = preset::by_name(&preset_name)
        .ok_or_else(|| format!("unknown preset: {preset_name}"))?;

    let path = std::path::Path::new(&input);
    let prepared = prepare::prepare_sync(path).map_err(|e| e)?;

    let session = sendmecongo_core::crc32(&prepared.object);
    let symbol_size = preset.symbol_size();
    let source_symbols = prepared.symbols(symbol_size);

    if let Some(target) = json_target {
        let json_str = format_prepare_json(
            path,
            &prepared,
            session,
            &preset_name,
            symbol_size,
            source_symbols,
        );
        if target == "-" || target.is_empty() {
            say(json_str);
        } else {
            std::fs::write(&target, json_str)?;
            say(format!(
                "Prepared {} ({} → {}) in {:.2}s. JSON written to {}",
                prepare::file_name(path),
                prepare::human(prepared.raw),
                prepare::human(prepared.wire()),
                prepared.secs,
                target
            ));
        }
    } else {
        say(format!(
            "File:         {} ({})",
            prepare::file_name(path),
            prepare::human(prepared.raw)
        ));
        say(format!(
            "Container:    {} ({}, CRC-32 {:08x}) in {:.2}s",
            prepare::human(prepared.wire()),
            prepared.method(),
            session,
            prepared.secs
        ));
        say(format!(
            "Preset:       {} (symbol size: {} B, source symbols: {})",
            preset_name,
            symbol_size,
            source_symbols
        ));
        let est = (source_symbols as f64) / preset.fps;
        say(format!(
            "Est. filming: ~{:.1}s ({} @ {:.0} sym/s)",
            est, preset.name, preset.fps
        ));
    }

    Ok(())
}

fn format_prepare_json(
    path: &std::path::Path,
    prepared: &prepare::Prepared,
    session: u32,
    preset_name: &str,
    symbol_size: u16,
    source_symbols: usize,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"tool\": \"sendmecongo-send\",");
    let _ = writeln!(out, "  \"action\": \"prepare\",");
    let _ = writeln!(out, "  \"file\": {},", quote(&path.to_string_lossy()));
    let _ = writeln!(out, "  \"name\": {},", quote(&prepare::file_name(path)));
    let _ = writeln!(out, "  \"raw_bytes\": {},", prepared.raw);
    let _ = writeln!(out, "  \"container_bytes\": {},", prepared.wire());
    let _ = writeln!(out, "  \"compression\": {},", quote(prepared.method()));
    let _ = writeln!(out, "  \"session\": \"{:08x}\",", session);
    let _ = writeln!(out, "  \"preset\": {},", quote(preset_name));
    let _ = writeln!(out, "  \"symbol_size\": {},", symbol_size);
    let _ = writeln!(out, "  \"source_symbols\": {},", source_symbols);
    let _ = writeln!(out, "  \"prepare_secs\": {:.3},", prepared.secs);
    let _ = writeln!(out, "  \"estimated_seconds\": {{");
    let presets = [
        ("turbo60", 60.0),
        ("turbo30", 30.0),
        ("turbo15", 15.0),
        ("megabit", 60.0),
        ("balanced", 15.0),
        ("robust", 10.0),
    ];
    for (i, (name, fps)) in presets.iter().enumerate() {
        let p_size = preset::by_name(name).map(|p| p.symbol_size()).unwrap_or(symbol_size);
        let p_syms = prepared.symbols(p_size);
        let est = (p_syms as f64) / fps;
        let comma = if i + 1 < presets.len() { "," } else { "" };
        let _ = writeln!(out, "    {}: {:.1}{}", quote(name), est, comma);
    }
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    out
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print_usage() {
    println!("sendmecongo-send — optical file transfer sender\n");
    println!("Usage:");
    println!("  sendmecongo-send [options]                                Launch configuration GUI");
    println!("  sendmecongo-send --file <file> --play [options]            Launch QR player directly");
    println!("  sendmecongo-send --file <file> --prepare-only [--json <p>] Dry-run container preparation\n");
    println!("Options:");
    println!("  --file, --in <path>        Input file to transfer");
    println!("  --preset <name>            Channel preset (default: turbo60)");
    println!("                             Choices: robust, balanced, turbo15, turbo30, turbo60, megabit");
    println!("  --play                     Launch full-screen optical player directly");
    println!("  --prepare-only             Inspect, compress and calculate symbol metrics without opening a window");
    println!("  --json <path>              Write prepare metrics to a JSON file (or '-' for stdout)");
    println!("  --resume-code, --repair-code <SMR1> Replay only deficits specified by repair code");
    println!("  --size <pixels>            QR code window size (default: 1600)");
    println!("  --cycles <N>               Stop after N full cycles (0 = infinite)");
    println!("  --lanes <N>                Split screen into N parallel lanes (1 or 2)");
    println!("  --borderless               Remove window titlebar and borders");
    println!("  --lang <en|zh-Hans|zh-Hant> UI and output language");
    println!("  -h, --help                 Show this help message");
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

    #[test]
    fn prepare_only_requires_input_file() {
        assert!(run_prepare(&[]).is_err());
    }

    #[test]
    fn prepare_only_runs_cleanly_on_small_bin() {
        let small_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/small.bin")
            .canonicalize()
            .expect("testdata/small.bin");
        let args = vec![
            "--file".to_string(),
            small_bin.to_string_lossy().to_string(),
            "--preset".to_string(),
            "turbo60".to_string(),
        ];
        assert!(run_prepare(&args).is_ok());
    }

    #[test]
    fn prepare_only_json_matches_contract() {
        let small_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/small.bin")
            .canonicalize()
            .expect("testdata/small.bin");
        let tmp = std::env::temp_dir().join(format!("prep_test_{}.json", std::process::id()));
        let args = vec![
            "--file".to_string(),
            small_bin.to_string_lossy().to_string(),
            "--json".to_string(),
            tmp.to_string_lossy().to_string(),
        ];
        assert!(run_prepare(&args).is_ok());
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("\"action\": \"prepare\""));
        assert!(content.contains("\"symbol_size\":"));
        assert!(content.contains("\"source_symbols\":"));
        assert!(content.contains("\"estimated_seconds\":"));
        assert!(content.contains("\"session\":"));
        let _ = std::fs::remove_file(tmp);
    }
}
