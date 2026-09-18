//! Where to look for the user's recording, and how to point at the result.
//!
//! A `.app` started by double-clicking has **no controlling terminal**: stdout
//! goes nowhere and stdin cannot be read. That is why the bundled app opens a
//! window (see `gui`) — a command line that prints its progress would be
//! invisible there, and one that asks a question would hang forever. What is
//! left here is the part that has nothing to do with the window: finding the
//! recording, and revealing what came out.
//!
//! One measured fact drives the whole search. Dropping a file onto a bundled app
//! does **not** hand the path over as an argument: LaunchServices sends an `odoc`
//! Apple Event, a plain executable has no Apple Event handler, and the process
//! ends up with no arguments at all. (Measured, not assumed — a probe bundle
//! built the same way reports `argc=0`.) So "the user dropped a video on us" and
//! "the user double-clicked us" are the same situation on the outside, and the
//! only way to find the file is to look where it would be. Inside the window the
//! drag-and-drop *does* work, because egui receives it directly.

use std::path::{Path, PathBuf};

/// Directory of the `.app` itself, when this process lives inside one.
///
/// The receiver's app and their recording usually sit in the same folder, so
/// this — not `Contents/MacOS`, and not the process's working directory (which
/// Finder sets to `/`) — is the place worth looking for videos.
pub fn bundle_parent() -> Option<PathBuf> {
    Some(sendmecongo_ui::launch::bundle()?.parent()?.to_path_buf())
}

/// `SENDMECONGO_NO_DIALOG=1` forces the plain command-line behaviour, which is what the
/// tests and any scripted use of a bundled binary want.
pub fn dialogs_disabled() -> bool {
    std::env::var_os("SENDMECONGO_NO_DIALOG").is_some()
}

/// True when this is a bundled app that Finder started rather than a shell.
///
/// The terminal check is what separates the two: double-clicking sets no
/// terminal, and `open Foo.app/Contents/MacOS/Foo` from a shell keeps one.
pub fn launched_from_finder() -> bool {
    !dialogs_disabled()
        && sendmecongo_ui::launch::bundle().is_some()
        && !sendmecongo_ui::launch::from_terminal()
}

/// Where a recording is likely to be lying, most useful first.
///
/// The app's own folder comes first because that is the documented way to use
/// it. Then the obvious user folders, then the home directory itself, then the
/// top level of every mounted volume — a recording arriving on a USB stick is
/// the normal case for a physically isolated transfer.
pub fn search_folders() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(folder) = bundle_parent() {
        dirs.push(folder);
    } else {
        if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.clone());
        for name in ["Downloads", "Desktop", "Movies", "下载", "桌面", "影片"] {
            dirs.push(home.join(name));
        }
    }

    if let Ok(volumes) = std::fs::read_dir("/Volumes") {
        for entry in volumes.flatten() {
            let path = entry.path();
            // `/Volumes/Macintosh HD` is a symlink back to `/`; following it
            // would mean listing the filesystem root, which is neither useful
            // nor cheap. Anything else that is not a real directory is skipped
            // for the same reason.
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                dirs.push(path);
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| dir.is_dir() && seen.insert(dir.clone()));
    dirs
}

/// Select the file in Finder, so "where did it go" needs no answer.
pub fn reveal(path: &Path) {
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .status();
}

/// A last-resort native alert, for the case where the window itself could not
/// open — otherwise the app would just bounce once in the Dock and vanish.
pub fn alert(title: &str, message: &str) {
    let script = alert_script(title, message);
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status();
}

fn alert_script(title: &str, message: &str) -> String {
    // AppleScript has no `\n` escape inside a literal; concatenating with
    // `return` is the portable way to build a multi-line message.
    let body = message
        .lines()
        .map(|line| format!("\"{}\"", escape(line)))
        .collect::<Vec<_>>()
        .join(" & return & ");
    let body = if body.is_empty() {
        "\"\"".to_string()
    } else {
        body
    };
    // The button label is part of the AppleScript source, so it is the one string that
    // cannot simply be swapped out later: whatever language it is compiled in is what
    // the dialog shows.
    let ok = sendmecongo_ui::i18n::t().snd_got_it;
    format!(
        "display dialog {body} buttons {{\"{ok}\"}} default button 1 \
         with title \"{}\" with icon caution",
        escape(title)
    )
}

/// AppleScript string literals take backslashes and double quotes literally, so
/// both have to be escaped; newlines are handled by the caller.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// Compile the generated AppleScript without running it. `display dialog`
    /// blocks until someone clicks, so the only way to test it is to compile it —
    /// which is exactly what catches a stray quote or an unbalanced brace, the
    /// failure mode that would otherwise reach the user as "the app does nothing".
    fn compiles(script: &str) -> Result<(), String> {
        let path = std::env::temp_dir().join("sendmecongo-finder-test.scpt");
        let output = std::process::Command::new("osacompile")
            .arg("-e")
            .arg(script)
            .arg("-o")
            .arg(&path)
            .output()
            .map_err(|e| format!("无法调用 osacompile: {e}"))?;
        let _ = std::fs::remove_file(&path);
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }

    #[test]
    fn the_alert_is_valid_applescript() {
        let script = alert_script("sendmecongo-recv 打不开窗口", "没有可用的图形环境。");
        compiles(&script).unwrap_or_else(|e| panic!("{e}\n---\n{script}"));
    }

    #[test]
    fn the_alert_survives_quotes_and_backslashes() {
        // A Windows-style path is the cheapest way to get both characters into
        // the message at once.
        let script = alert_script("没能还原", "打开录像失败：C:\\Users\\\"张三\"\\录像.mov");
        compiles(&script).unwrap_or_else(|e| panic!("{e}\n---\n{script}"));
    }

    #[test]
    fn the_search_folders_are_real_directories() {
        for folder in search_folders() {
            assert!(folder.is_dir(), "{} is not a directory", folder.display());
        }
    }
}
