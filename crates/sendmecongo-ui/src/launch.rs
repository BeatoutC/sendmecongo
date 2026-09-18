//! Launch-context questions both apps have to ask.
//!
//! The one that matters: on macOS a Mach-O executable with no `.app` around it is
//! opened by Finder *in Terminal.app*, so double-clicking the file that `cargo build`
//! just produced parks a console window next to the GUI — or, for the receiver, prints
//! a prompt into one and never opens a window at all. Nothing inside the process can
//! close a window Terminal opened; the answer is to ship the `.app`, and to say so in
//! the one place the person is definitely looking.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The `.app` bundle an executable lives inside, if any.
///
/// Walks up from `…/Foo.app/Contents/MacOS/Foo` rather than assuming a depth, so the
/// same answer comes back for a bundle moved anywhere, including onto a DMG.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|path| path.extension().is_some_and(|e| e == "app"))
        .map(PathBuf::from)
}

/// [`bundle_of`] for the running process.
pub fn bundle() -> Option<PathBuf> {
    bundle_of(&std::env::current_exe().ok()?)
}

/// True when the process was started from a shell that is still attached.
pub fn from_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// One line, printed only in the situation that creates the unwanted window: macOS,
/// no bundle, and a terminal to print on.
///
/// `app` is the bundle that *should* have been launched (`sendmecongo-send` or `sendmecongo-recv`).
pub fn note_unbundled(app: &str) {
    if !cfg!(target_os = "macos") || bundle().is_some() || !from_terminal() {
        return;
    }
    let _ = writeln!(
        std::io::stderr(),
        "{}",
        crate::i18n::fill(crate::i18n::t().note_unbundled, &[&app])
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_is_found_from_the_binary_inside_it() {
        assert_eq!(
            bundle_of(Path::new("/x/dist/sendmecongo-recv.app/Contents/MacOS/sendmecongo-recv")),
            Some(PathBuf::from("/x/dist/sendmecongo-recv.app"))
        );
        // A bundle mounted on a DMG, which is the copy that actually reaches people.
        assert_eq!(
            bundle_of(Path::new("/Volumes/sendmecongo 接收/sendmecongo-recv.app/Contents/MacOS/sendmecongo-recv")),
            Some(PathBuf::from("/Volumes/sendmecongo 接收/sendmecongo-recv.app"))
        );
    }

    #[test]
    fn a_target_directory_binary_has_no_bundle() {
        // The case this module exists for: `cargo build` output, where Finder
        // double-click means Terminal.
        assert_eq!(bundle_of(Path::new("/x/sendmecongo/target/release/sendmecongo-send")), None);
    }
}
