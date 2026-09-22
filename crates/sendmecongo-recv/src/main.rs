//! sendmecongo-recv — recover a file from a phone recording of an SMQ1 QR stream.
//!
//! This is the receiver's *entire* job, in one static executable. The Python +
//! OpenCV + zxing-cpp + Rust-toolchain chain the GUI used to hand out is gone;
//! the person holding the phone hands over a recording and gets a file back.
//!
//!     sendmecongo-recv 录像.mov
//!     sendmecongo-recv 录像.mov --compare 原文件 --json 结果.json
//!
//! Run with no arguments at all and it looks around for a recording and asks.
//! On macOS it also ships as `sendmecongo-recv.app`, because a receiver should be
//! able to double-click something — see `finder` for what that changes.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod decode;
mod finder;
mod gui;
mod isobmff;
mod pipeline;
mod scan;

use sendmecongo_core::Progress;
use sendmecongo_ui::i18n::{fill, pad_label, t, Text};
use pipeline::{Config, CANCELLED};
use std::fmt::Write as _;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

const VIDEO_EXTENSIONS: [&str; 5] = ["mov", "mp4", "m4v", "MOV", "MP4"];

fn attach_parent_console() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() -> ExitCode {
    attach_parent_console();
    // LaunchServices still hands a bundled app `-psn_0_1234` on some systems.
    // It is not a file, and treating it as one fails with a puzzling message.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with("-psn_"))
        .collect();
    let gui = !args.iter().any(|a| a == "-h" || a == "--help") && wants_gui(&args);
    // Before the first line of help or the first widget: the language has to be settled
    // while there is still nothing on screen to be wrong.
    sendmecongo_ui::i18n::init(lang_arg(&args).as_deref(), gui);

    if gui {
        sendmecongo_ui::launch::note_unbundled("sendmecongo-recv");
        return match gui::launch(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                // The window is the only way this app can talk to a person; if it
                // cannot open, an invisible line on stderr is not an answer.
                let t = sendmecongo_ui::i18n::t();
                finder::alert(t.rcv_alert_title, &message);
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        };
    }

    match run(&args) {
        Ok(CliExit::Done) => ExitCode::SUCCESS,
        // A partial receive is a checkpoint, not a failure — but scripts and
        // agents still need to tell it apart from a finished file.
        Ok(CliExit::Partial) => ExitCode::from(2),
        Err(message) => {
            eprintln!();
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// The two ways a command-line run can end well.
enum CliExit {
    Done,
    Partial,
}

/// The tag given to `--lang`, if any. Read before anything else so that even an argument
/// error comes out in the language that was asked for.
fn lang_arg(args: &[String]) -> Option<String> {
    let at = args.iter().position(|arg| arg == "--lang")?;
    args.get(at + 1).cloned()
}

/// A bundled app started from Finder has no terminal: a command line that prints
/// its progress is invisible there, and one that asks a question hangs forever.
/// Those launches get the window instead. `--gui` asks for it explicitly, which
/// is also how it is tested from a shell.
fn wants_gui(args: &[String]) -> bool {
    wants_gui_with(
        args,
        finder::launched_from_finder(),
        sendmecongo_ui::launch::from_terminal(),
        finder::dialogs_disabled(),
    )
}

/// The decision, with every input passed in, so the truth table can be tested
/// without a terminal, a bundle, or an environment variable.
fn wants_gui_with(args: &[String], from_finder: bool, terminal: bool, no_dialog: bool) -> bool {
    if args.iter().any(|a| a == "--gui") {
        return true;
    }
    if no_dialog {
        return false;
    }
    if from_finder {
        return true;
    }
    // Started with nothing to do and a terminal to talk to. There is no script to be
    // written yet, and the window does the same job — finding the recordings nearby,
    // asking which one, showing progress — with the result visible instead of scrolled
    // away. This is also the path a *bare binary* takes when Finder/Explorer double-clicks it:
    // macOS opens those in Terminal and Windows starts it with a console attached, so without
    // this rule the app looks like a console program that numbers a list at you and immediately
    // exits if standard input closes or no files are given.
    args.is_empty() && (terminal || cfg!(target_os = "windows"))
}

fn run(args: &[String]) -> Result<CliExit, String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        usage();
        return Ok(CliExit::Done);
    }
    let (options, video) = parse_args(args)?;

    let video = match video {
        Some(v) => v,
        None => match choose_interactively()? {
            Some(v) => v,
            None => return Ok(CliExit::Done),
        },
    };
    recv(&video, &options)
}

/// Flags shared by the command line and the window.
fn parse_args(args: &[String]) -> Result<(Options, Option<PathBuf>), String> {
    let mut options = Options::default();
    let mut video: Option<PathBuf> = None;
    let mut index = 0usize;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--out" => options.out = Some(PathBuf::from(take(args, &mut index, arg)?)),
            "--compare" => options.compare = Some(PathBuf::from(take(args, &mut index, arg)?)),
            "--dump" => options.dump = Some(PathBuf::from(take(args, &mut index, arg)?)),
            "--json" => options.json = Some(PathBuf::from(take(args, &mut index, arg)?)),
            "--resume" => options.resume.push(PathBuf::from(take(args, &mut index, arg)?)),
            "--threads" => {
                let value = take(args, &mut index, arg)?;
                options.threads = Some(
                    value
                        .parse()
                        .map_err(|_| fill(t().cli_err_threads, &[&arg, &value]))?,
                );
            }
            // Already applied in `main`, before anything could be printed.
            "--lang" => {
                let _ = take(args, &mut index, arg)?;
            }
            "--quiet" | "-q" => options.quiet = true,
            "--gui" => {}
            other if other.starts_with("--") => {
                return Err(fill(t().cli_err_unknown_option, &[&other]));
            }
            other => {
                if video.is_some() {
                    return Err(fill(t().cli_err_one_video, &[&other]));
                }
                video = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }
    Ok((options, video))
}

fn take(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| fill(t().cli_err_needs_value, &[&flag]))
}

#[derive(Default, Clone)]
struct Options {
    out: Option<PathBuf>,
    compare: Option<PathBuf>,
    dump: Option<PathBuf>,
    json: Option<PathBuf>,
    /// Earlier recordings' `.smr.json` manifests, merged before the run (M2).
    resume: Vec<PathBuf>,
    threads: Option<usize>,
    quiet: bool,
}

/// A handle on a running job: the numbers to display and the switch to stop it.
///
/// The pipeline neither knows nor cares who is holding this — a progress line on
/// a terminal and a progress bar in a window read exactly the same fields.
#[derive(Clone)]
struct Job {
    counters: Arc<pipeline::Counters>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl Job {
    fn new() -> Self {
        Self {
            counters: Arc::new(pipeline::Counters::default()),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn stop(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The few things about a recording that both the running screen and the report show.
///
/// Kept as numbers rather than a finished sentence: the window can be switched to another
/// language while a job is in flight, and a string formatted when the recording was opened
/// would be the one line left in the old language.
#[derive(Clone, Copy)]
struct TrackFacts {
    secs: f64,
    width: u32,
    height: u32,
    codec: isobmff::Codec,
    frames: usize,
    fps: f64,
}

impl TrackFacts {
    fn of(track: &isobmff::VideoTrack) -> Self {
        Self {
            secs: track.duration_secs(),
            width: track.width,
            height: track.height,
            codec: track.codec,
            frames: track.samples.len(),
            fps: track.fps(),
        }
    }

    fn summary(&self, t: &Text, threads: usize) -> String {
        fill(
            t.rcv_summary,
            &[
                &human_duration(t, self.secs),
                &self.width,
                &self.height,
                &self.codec,
                &self.frames,
                &format!("{:.2}", self.fps),
                &threads,
            ],
        )
    }
}

/// The verdict of `--compare`, kept as a value rather than as a sentence so the window can
/// colour it without parsing its own text back out.
#[derive(Clone)]
enum Comparison {
    Identical {
        original: PathBuf,
    },
    Mismatch {
        original: PathBuf,
        expected: usize,
        got: usize,
    },
}

#[derive(Clone)]
struct Report {
    video: PathBuf,
    /// Where the recovered file was written.
    target: PathBuf,
    bytes: usize,
    track: TrackFacts,
    threads: usize,
    /// What the pipeline counted while it worked.
    frames_decoded: usize,
    codes_found: usize,
    unique_symbols: usize,
    symbols_received: usize,
    duplicates: usize,
    /// Symbol geometry out of the first frame header, when one was seen.
    symbol_size: u16,
    object_len: u32,
    source_symbols: usize,
    /// Set when the run started from earlier recordings' manifests:
    /// (symbols carried in, recordings merged).
    resumed: Option<(usize, usize)>,
    comparison: Option<Comparison>,
    elapsed_secs: f64,
    throughput: f64,
}

/// A run that ended short of enough symbols: a checkpoint, not a failure.
/// Everything needed to resume is on disk at `manifest_path`.
struct PartialReport {
    video: PathBuf,
    /// `None` when no valid frame was ever seen — nothing worth resuming with,
    /// which *is* a plain failure and gets the old `err_symbols_short` text.
    manifest_path: Option<PathBuf>,
    received: usize,
    needed: usize,
    source: usize,
    resumed: Option<(usize, usize)>,
    /// The full symbol sets, for the window's per-block reception grid (M2.2).
    progress: Option<Progress>,
    // Raw counters, for the no-progress failure message.
    pictures: usize,
    codes: usize,
    rejected: usize,
    decode_errors: usize,
}

impl PartialReport {
    /// The one-message summary, shared by the terminal and the window.
    fn message(&self, t: &Text) -> String {
        match &self.manifest_path {
            Some(path) => {
                let saved = fill(t.rcv_manifest_saved, &[&path.display()]);
                let hint = fill(t.rcv_partial_hint, &[&path.display()]);
                format!("{saved}\n{hint}")
            }
            None => fill(
                t.err_symbols_short,
                &[
                    &self.received,
                    &self.source,
                    &self.pictures,
                    &self.codes,
                    &self.rejected,
                    &self.decode_errors,
                ],
            ),
        }
    }
}

/// What `execute` produced. The window shows both; the command line also maps
/// them to distinct exit codes (0 done / 2 partial) for scripts and agents.
enum ExecResult {
    Done(Report),
    Partial(PartialReport),
}

/// Load every `--resume` manifest and merge them into one starting point.
///
/// Each checkpoint is two files: `X.smr.json` (the index — merge key, estimates,
/// recordings list) and `X.smr.bin` (the symbol payloads, same framing as
/// `--dump`). Both are required: the JSON cannot rebuild the decoder on its own.
/// Returns the cumulative checkpoint frames and the combined recordings list.
fn load_resume(paths: &[PathBuf]) -> Result<(Vec<Vec<u8>>, Vec<String>), String> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut recordings = Vec::new();
    let mut key: Option<(u32, u64, u16)> = None;
    for path in paths {
        let text = std::fs::read_to_string(path)
            .map_err(|e| fill(t().rcv_resume_failed, &[&path.display(), &e]))?;
        let manifest = Progress::manifest_from_json(&text)
            .map_err(|e| fill(t().rcv_resume_failed, &[&path.display(), &e]))?;
        let progress = Progress::from_manifest(&manifest)
            .map_err(|e| fill(t().rcv_resume_failed, &[&path.display(), &e]))?;
        // Pre-flight: all checkpoints must belong to the same transfer.
        let this = (progress.session, progress.object_len, progress.symbol_size);
        if let Some(previous) = key {
            if previous != this {
                return Err(fill(
                    t().rcv_resume_failed,
                    &[&path.display(), &"session/object_len/symbol_size 不一致".to_string()],
                ));
            }
        } else {
            key = Some(this);
        }
        let bin = frames_bin_path_for(path);
        frames.extend(read_frames_bin(&bin)?);
        recordings.extend(manifest.recordings.clone());
    }
    Ok((frames, recordings))
}

/// Where the checkpoint lives: next to the recording, named after it.
fn manifest_path_for(video: &Path) -> PathBuf {
    video.with_extension("smr.json")
}

/// The payload sidecar of a checkpoint at `manifest` (`X.smr.json` -> `X.smr.bin`).
fn frames_bin_path_for(manifest: &Path) -> PathBuf {
    manifest.with_extension("smr.bin")
}

/// Read `u32-len + frame` sequences — the same framing `--dump` writes.
fn read_frames_bin(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let data = std::fs::read(path)
        .map_err(|e| fill(t().rcv_resume_failed, &[&path.display(), &e]))?;
    let mut frames = Vec::new();
    let mut cursor = 0usize;
    while cursor + 4 <= data.len() {
        let len = u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        if cursor + len > data.len() {
            return Err(fill(
                t().rcv_resume_failed,
                &[&path.display(), &"checkpoint truncated".to_string()],
            ));
        }
        frames.push(data[cursor..cursor + len].to_vec());
        cursor += len;
    }
    Ok(frames)
}

/// Write the `.smr.json` checkpoint and its `.smr.bin` payload sidecar.
/// `frames` is the *cumulative* distinct set (checkpoint in + this recording's
/// new symbols) — `pipeline::Outcome::symbols` is exactly that. Returns the
/// manifest path, or `None` when there is no session state worth keeping.
fn save_manifest(
    video: &Path,
    progress: Option<&Progress>,
    prior_recordings: &[String],
    frames: &[Vec<u8>],
) -> Result<Option<PathBuf>, String> {
    let Some(progress) = progress else {
        return Ok(None);
    };
    let name = video
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| video.display().to_string());
    let mut recordings = prior_recordings.to_vec();
    if !recordings.contains(&name) {
        recordings.push(name);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let manifest = progress.to_manifest(recordings, now);
    let json = Progress::manifest_to_json(&manifest).map_err(|e| e.to_string())?;
    let path = manifest_path_for(video);
    std::fs::write(&path, json).map_err(|e| fill(t().rcv_json_failed, &[&e]))?;

    let bin = frames_bin_path_for(&path);
    let mut payload = Vec::with_capacity(frames.iter().map(|f| f.len() + 4).sum());
    for frame in frames {
        payload.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        payload.extend_from_slice(frame);
    }
    std::fs::write(&bin, payload).map_err(|e| fill(t().rcv_dump_failed, &[&e]))?;

    Ok(Some(path))
}

/// The report rendered as text, in one language.
struct Lines {
    track_summary: String,
    counters: String,
    symbols: String,
    output: String,
    verify: Option<String>,
    /// Whether `verify` should be drawn in green: the comparison was actually run and it
    /// matched. False means "no comparison was asked for", not "it failed".
    identical: bool,
    timing: String,
    warnings: Vec<String>,
}

impl Report {
    fn lines(&self, t: &Text) -> Lines {
        let verify = self.comparison.as_ref().map(|comparison| match comparison {
            Comparison::Identical { original } => {
                fill(t.rcv_verify_identical, &[&original.display()])
            }
            Comparison::Mismatch { original, .. } => {
                fill(t.rcv_verify_mismatch, &[&original.display()])
            }
        });
        let warnings = match &self.comparison {
            Some(Comparison::Mismatch {
                original,
                expected,
                got,
            }) => vec![fill(
                t.rcv_verify_mismatch_warn,
                &[&original.display(), expected, got],
            )],
            _ => Vec::new(),
        };
        Lines {
            track_summary: self.track.summary(t, self.threads),
            counters: fill(
                t.rcv_counters_line,
                &[
                    &self.frames_decoded,
                    &self.codes_found,
                    &self.unique_symbols,
                    &self.symbols_received,
                    &self.duplicates,
                ],
            ),
            symbols: fill(
                t.rcv_symbols_line,
                &[&self.symbol_size, &self.object_len, &self.source_symbols],
            ),
            output: format!("{}  ({})", self.target.display(), human_bytes(self.bytes)),
            verify,
            identical: matches!(self.comparison, Some(Comparison::Identical { .. })),
            timing: fill(
                t.rcv_timing_line,
                &[
                    &format!("{:.1}", self.elapsed_secs),
                    &human_rate(self.throughput),
                ],
            ),
            warnings,
        }
    }
}

/// Open the recording and read its geometry.
///
/// Split out of [`execute`] so a caller can show what it is about to work on —
/// the GUI puts the duration, resolution and codec on screen before it commits a
/// minute of CPU to the job.
fn prepare(video: &Path) -> Result<isobmff::VideoTrack, String> {
    let track = isobmff::open(video).map_err(|e| fill(t().rcv_open_failed, &[&e]))?;
    if track.samples.is_empty() {
        return Err(t().rcv_no_frames.to_string());
    }
    Ok(track)
}

/// The whole job minus presentation: decode, recognise, recover, write, verify.
///
/// `counters` is handed in rather than created here so that whoever is watching —
/// a progress line on a terminal, or a window with a progress bar — can read the
/// same numbers while the work is running.
fn execute(
    video: &Path,
    track: &isobmff::VideoTrack,
    options: &Options,
    job: &Job,
) -> Result<ExecResult, String> {
    let threads = options
        .threads
        .unwrap_or_else(pipeline::default_threads)
        .max(1);
    let mut config = Config::new(threads);
    config.counters = Arc::clone(&job.counters);
    config.cancel = Arc::clone(&job.cancel);

    // M2 resume: earlier recordings' symbols are the starting point.
    let mut prior_recordings: Vec<String> = Vec::new();
    let mut resumed: Option<(usize, usize)> = None;
    if !options.resume.is_empty() {
        let (frames, recordings) = load_resume(&options.resume)?;
        resumed = Some((frames.len(), recordings.len()));
        config.resume_frames = frames;
        prior_recordings = recordings;
    }

    let outcome = pipeline::run(video, track, &config, |_| {})?;

    // Complete or partial, the checkpoint is refreshed so a later run can pick
    // up exactly where this one stopped.
    let manifest_path = save_manifest(
        video,
        outcome.progress.as_ref(),
        &prior_recordings,
        &outcome.symbols,
    )?;

    let Some(received) = &outcome.received else {
        let progress = outcome.progress.as_ref();
        let source = progress
            .map(|p| p.source_blocks().iter().map(|(_, k)| *k as usize).sum())
            .unwrap_or(0);
        let partial = PartialReport {
            video: video.to_path_buf(),
            manifest_path: manifest_path.clone(),
            received: progress.map_or(0, Progress::received),
            needed: progress.map_or(0, Progress::needed_estimate),
            source,
            resumed,
            progress: progress.cloned(),
            pictures: outcome.counters.pictures(),
            codes: outcome.counters.codes(),
            rejected: outcome.counters.rejected(),
            decode_errors: outcome.counters.decode_errors(),
        };
        if let (Some(path), Some(progress)) = (&manifest_path, progress) {
            if let Some(json_path) = &options.json {
                std::fs::write(json_path, partial_json(&partial, progress, path, &outcome))
                    .map_err(|e| fill(t().rcv_json_failed, &[&e]))?;
            }
        }
        return Ok(ExecResult::Partial(partial));
    };

    let bytes = received.data.len();

    // --- write the file ---------------------------------------------------
    let out_dir = options
        .out
        .clone()
        .unwrap_or_else(|| video.parent().unwrap_or(Path::new(".")).to_path_buf());
    std::fs::create_dir_all(&out_dir).map_err(|e| fill(t().rcv_mkdir_failed, &[&e]))?;
    let target = out_dir.join(safe_name(&received.name));
    std::fs::write(&target, &received.data)
        .map_err(|e| fill(t().rcv_write_failed, &[&target.display(), &e]))?;

    // --- verify -----------------------------------------------------------
    let comparison = match &options.compare {
        Some(original) => {
            let expected = std::fs::read(original)
                .map_err(|e| fill(t().rcv_read_failed, &[&original.display(), &e]))?;
            Some(if expected == received.data {
                Comparison::Identical {
                    original: original.clone(),
                }
            } else {
                Comparison::Mismatch {
                    original: original.clone(),
                    expected: expected.len(),
                    got: bytes,
                }
            })
        }
        None => None,
    };

    let header = outcome.first_header;
    let source_symbols = header
        .map(|h| h.object_len.div_ceil(h.symbol_size as u32) as usize)
        .unwrap_or(0);
    let symbol_size = header.map(|h| h.symbol_size).unwrap_or(0);

    // --- optional artefacts ----------------------------------------------
    if let Some(path) = &options.dump {
        let mut dump = Vec::with_capacity(outcome.symbols.iter().map(|s| s.len() + 4).sum());
        for symbol in &outcome.symbols {
            dump.extend_from_slice(&(symbol.len() as u32).to_le_bytes());
            dump.extend_from_slice(symbol);
        }
        std::fs::write(path, &dump).map_err(|e| fill(t().rcv_dump_failed, &[&e]))?;
    }

    let elapsed_secs = outcome.elapsed.as_secs_f64();
    let report = Report {
        video: video.to_path_buf(),
        target,
        bytes,
        track: TrackFacts::of(track),
        threads,
        frames_decoded: outcome.counters.pictures(),
        codes_found: outcome.counters.codes(),
        unique_symbols: received.frames_used,
        symbols_received: outcome.counters.symbols(),
        duplicates: received.duplicates,
        symbol_size,
        object_len: header.map(|h| h.object_len).unwrap_or(0),
        source_symbols,
        resumed,
        comparison,
        elapsed_secs,
        throughput: bytes as f64 / elapsed_secs.max(1e-9),
    };

    if let Some(path) = &options.json {
        std::fs::write(path, json(&report, received, &outcome))
            .map_err(|e| fill(t().rcv_json_failed, &[&e]))?;
    }

    Ok(ExecResult::Done(report))
}

/// The command-line run: same job, narrated on the terminal.
fn recv(video: &Path, options: &Options) -> Result<CliExit, String> {
    let interactive = std::io::stderr().is_terminal() && !options.quiet;
    let track = prepare(video)?;
    let facts = TrackFacts::of(&track);
    let threads = options
        .threads
        .unwrap_or_else(pipeline::default_threads)
        .max(1);

    if interactive {
        eprintln!("{}", t().cli_banner);
        eprintln!("{}{}", pad_label(t().label_video), video.display());
        eprintln!("{}{}", " ".repeat(10), facts.summary(t(), threads));
        eprintln!();
    }

    let started = std::time::Instant::now();
    let job = Job::new();

    // The pipeline reports progress through a callback it invokes from the
    // collecting thread, which prints at most four times a second anyway; a
    // timer here gives a line that keeps ticking through a long stretch where
    // nothing completes at all. That "it went quiet" moment is the one that
    // makes a person think the tool has hung.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watch = interactive.then(|| {
        let counters = Arc::clone(&job.counters);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let line = fill(
                    t().cli_progress,
                    &[
                        &counters.pictures(),
                        &counters.codes(),
                        &counters.unique(),
                        &counters.target(),
                        &human_duration(t(), started.elapsed().as_secs_f64()),
                    ],
                );
                eprint!("\r\x1b[2K{line}");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        })
    });

    let result = execute(video, &track, options, &job);

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(handle) = watch {
        let _ = handle.join();
        eprint!("\r\x1b[2K");
    }

    // `CANCELLED` is a sentinel the window matches on, not something to read. A person at
    // a terminal gets the sentence.
    let exec = match result {
        Ok(exec) => exec,
        Err(message) if message == CANCELLED => return Err(t().err_cancelled.to_string()),
        Err(message) => return Err(message),
    };
    match exec {
        ExecResult::Done(report) => {
            if !options.quiet {
                print_report(&report);
            }
            let warnings = report.lines(t()).warnings;
            if warnings.is_empty() {
                Ok(CliExit::Done)
            } else {
                Err(warnings.join("\n"))
            }
        }
        ExecResult::Partial(partial) => {
            if partial.manifest_path.is_none() {
                // No valid frame at all: a plain failure, not a checkpoint.
                return Err(partial.message(t()));
            }
            if !options.quiet {
                println!();
                println!("{}{}", pad_label(t().label_video), partial.video.display());
                if let Some((symbols, recordings)) = partial.resumed {
                    println!(
                        "{}{}",
                        pad_label(t().label_recover),
                        fill(t().rcv_resume_loaded, &[&symbols, &recordings])
                    );
                }
                println!(
                    "{}{} / {}  (-{})",
                    pad_label(t().label_symbols),
                    partial.received,
                    partial.source,
                    partial.needed
                );
                println!("{}{}", pad_label(t().label_recover), partial.message(t()));
                println!();
            }
            Ok(CliExit::Partial)
        }
    }
}

/// Everything the receiver needs to decide whether to trust the result.
fn print_report(report: &Report) {
    let t = t();
    let lines = report.lines(t);
    println!();
    println!("{}{}", pad_label(t.label_video), report.video.display());
    println!("{}{}", " ".repeat(10), lines.track_summary);
    println!("{}{}", pad_label(t.label_decode), lines.counters);
    println!("{}{}", " ".repeat(10), lines.symbols);
    if let Some((symbols, recordings)) = report.resumed {
        println!(
            "{}{}",
            " ".repeat(10),
            fill(t.rcv_resume_loaded, &[&symbols, &recordings])
        );
    }
    println!("{}{}", pad_label(t.label_recover), lines.output);
    if let Some(verify) = &lines.verify {
        println!("{}{}", pad_label(t.label_verify), verify);
    }
    println!("{}{}", pad_label(t.label_timing), lines.timing);
    for warning in &lines.warnings {
        println!();
        println!("{}{}", pad_label(t.label_warning), warning);
    }
    println!();
}

fn json(report: &Report, received: &sendmecongo_core::Received, outcome: &pipeline::Outcome) -> String {
    let t = t();
    let lines = report.lines(t);
    let header = outcome.first_header;
    let mut out = String::new();
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"tool\": \"sendmecongo-recv\",");
    let _ = writeln!(
        out,
        "  \"input\": {},",
        quote(&report.video.to_string_lossy())
    );
    let _ = writeln!(out, "  \"output\": {},", quote(&lines.output));
    let _ = writeln!(out, "  \"result\": \"COMPLETE\",");
    let _ = writeln!(out, "  \"bytes\": {},", received.data.len());
    let _ = writeln!(
        out,
        "  \"frames_decoded\": {},",
        outcome.counters.pictures()
    );
    let _ = writeln!(out, "  \"codes_found\": {},", outcome.counters.codes());
    let _ = writeln!(
        out,
        "  \"symbols_received\": {},",
        outcome.counters.symbols()
    );
    let _ = writeln!(out, "  \"unique_symbols\": {},", received.frames_used);
    let _ = writeln!(out, "  \"duplicate_symbols\": {},", received.duplicates);
    let _ = writeln!(
        out,
        "  \"rejected_symbols\": {},",
        outcome.counters.rejected()
    );
    let _ = writeln!(
        out,
        "  \"decode_errors\": {},",
        outcome.counters.decode_errors()
    );
    let _ = writeln!(out, "  \"gops\": {},", outcome.counters.gops());
    let _ = writeln!(
        out,
        "  \"symbol_size\": {},",
        header.map(|h| h.symbol_size).unwrap_or(0)
    );
    let _ = writeln!(
        out,
        "  \"object_len\": {},",
        header.map(|h| h.object_len).unwrap_or(0)
    );
    let _ = writeln!(
        out,
        "  \"source_symbols\": {},",
        header
            .map(|h| h.object_len.div_ceil(h.symbol_size as u32))
            .unwrap_or(0)
    );
    let _ = writeln!(
        out,
        "  \"elapsed_sec\": {:.3},",
        outcome.elapsed.as_secs_f64()
    );
    let _ = writeln!(
        out,
        "  \"throughput_bytes_per_sec\": {:.1},",
        received.data.len() as f64 / outcome.elapsed.as_secs_f64().max(1e-9)
    );
    let _ = writeln!(
        out,
        "  \"identical\": {}",
        match &report.comparison {
            Some(Comparison::Identical { .. }) => "true".to_string(),
            Some(Comparison::Mismatch { .. }) => "false".to_string(),
            None => "null".into(),
        }
    );
    let _ = writeln!(out, "}}");
    out
}

/// The `--json` contract for a partial run (docs/M2-DESIGN §2.2): an agent reads
/// this and nothing else to decide the next move.
fn partial_json(
    partial: &PartialReport,
    progress: &Progress,
    manifest: &Path,
    outcome: &pipeline::Outcome,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"tool\": \"sendmecongo-recv\",");
    let _ = writeln!(
        out,
        "  \"input\": {},",
        quote(&partial.video.to_string_lossy())
    );
    let _ = writeln!(out, "  \"result\": \"PARTIAL\",");
    let _ = writeln!(out, "  \"progress\": {{");
    let _ = writeln!(out, "    \"session\": \"{:08x}\",", progress.session);
    let _ = writeln!(out, "    \"object_len\": {},", progress.object_len);
    let _ = writeln!(out, "    \"symbol_size\": {},", progress.symbol_size);
    let _ = writeln!(out, "    \"received\": {},", progress.received());
    let _ = writeln!(out, "    \"source_symbols\": {},", partial.source);
    let _ = writeln!(out, "    \"needed_estimate\": {},", partial.needed);
    let _ = writeln!(
        out,
        "    \"manifest\": {}",
        quote(&manifest.to_string_lossy())
    );
    let _ = writeln!(out, "  }},");
    let _ = writeln!(out, "  \"frames_decoded\": {},", outcome.counters.pictures());
    let _ = writeln!(out, "  \"codes_found\": {},", outcome.counters.codes());
    let _ = writeln!(out, "  \"symbols_received\": {},", outcome.counters.symbols());
    let _ = writeln!(out, "  \"rejected_symbols\": {},", outcome.counters.rejected());
    let _ = writeln!(out, "  \"decode_errors\": {},", outcome.counters.decode_errors());
    let _ = writeln!(out, "  \"elapsed_sec\": {:.3}", outcome.elapsed.as_secs_f64());
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

/// Never let a container-supplied name escape the output directory.
fn safe_name(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("recovered.bin")
        .trim();
    let cleaned: String = base
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\0' | '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        })
        .collect();
    let cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        "recovered.bin".into()
    } else {
        cleaned
    }
}

fn human_bytes(n: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn human_rate(bytes_per_second: f64) -> String {
    format!("{}/s", human_bytes(bytes_per_second as usize))
}

fn human_duration(t: &Text, secs: f64) -> String {
    if secs < 60.0 {
        fill(t.seconds_short, &[&format!("{secs:.1}")])
    } else {
        let total = secs.round() as u64;
        fill(t.minutes_seconds, &[&(total / 60), &(total % 60)])
    }
}

/// Run with no arguments: find a recording nearby and ask which one.
///
/// Only reached from a terminal — a bundled launch goes to the window, which
/// does its own selection.
fn choose_interactively() -> Result<Option<PathBuf>, String> {
    let mut candidates = collect_candidates();
    candidates.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    if candidates.is_empty() {
        usage();
        return Ok(None);
    }

    if !std::io::stdin().is_terminal() {
        let (path, _) = candidates.remove(0);
        return Ok(Some(path));
    }

    println!("{}", t().cli_found_videos);
    for (index, (path, _)) in candidates.iter().take(8).enumerate() {
        println!("  {}. {}", index + 1, path.display());
    }
    print!("{}", t().cli_choose);
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| fill(t().cli_read_choice_failed, &[&e]))?;
    let line = line.trim();
    let choice: usize = if line.is_empty() {
        1
    } else {
        line.parse()
            .map_err(|_| fill(t().cli_not_a_number, &[&line]))?
    };
    if choice == 0 || choice > candidates.len().min(8) {
        return Err(fill(t().cli_out_of_range, &[&candidates.len().min(8)]));
    }
    Ok(Some(candidates.remove(choice - 1).0))
}

type Dated = (PathBuf, std::time::SystemTime);

fn collect_candidates() -> Vec<Dated> {
    // From a bundle, `current_exe` is …/sendmecongo-recv.app/Contents/MacOS/sendmecongo-recv
    // and the working directory is `/`, so the useful places are the folder the
    // app was put in and the user's own folders. See `finder::search_folders`.
    let dirs = finder::search_folders();

    let mut seen_paths = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !VIDEO_EXTENSIONS.contains(&extension) {
                continue;
            }
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            out.push((path, modified));
        }
    }
    out
}

fn usage() {
    println!("{}", t().cli_usage);
}

#[cfg(test)]
mod tests {
    use super::{wants_gui_with, Comparison, Report, TrackFacts};
    use sendmecongo_ui::i18n::Lang;
    use std::path::PathBuf;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// The checkpoint pair round-trips: JSON index + frame sidecar written by
    /// `save_manifest` come back through `load_resume` as the same symbol set.
    #[test]
    fn a_checkpoint_pair_survives_save_and_load() {
        use sendmecongo_core::{Receiver, Sender};
        let dir = std::env::temp_dir().join(format!("smc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("第一段.mov");
        std::fs::write(&video, b"not really a video").unwrap();

        // A first recording that captured part of the stream.
        let sender = Sender::new("x.bin", &vec![5u8; 40_000], 1000, 20).unwrap();
        let mut receiver = Receiver::new();
        let mut frames = Vec::new();
        for frame in sender.frames().iter().take(30) {
            receiver.push(frame).unwrap();
            frames.push(frame.clone());
        }
        let progress = receiver.export_progress().unwrap();
        let manifest = super::save_manifest(&video, Some(&progress), &[], &frames)
            .unwrap()
            .expect("a checkpoint was written");
        assert!(manifest.ends_with("第一段.smr.json"));
        assert!(super::frames_bin_path_for(&manifest).exists());

        let (loaded_frames, recordings) = super::load_resume(&[manifest.clone()]).unwrap();
        assert_eq!(loaded_frames, frames);
        assert_eq!(recordings, vec!["第一段.mov".to_string()]);

        // And the loaded frames rebuild a receiver at exactly the same spot.
        let mut resumed = Receiver::new();
        for frame in &loaded_frames {
            resumed.push(frame).unwrap();
        }
        assert_eq!(resumed.frames_used(), receiver.frames_used());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two checkpoints from different transfers (different symbol sizes) are
    /// refused at load time, not silently mixed into one decoder.
    #[test]
    fn mismatched_checkpoints_are_refused() {
        use sendmecongo_core::{Receiver, Sender};
        let dir = std::env::temp_dir().join(format!("smc-test-mm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut manifests = Vec::new();
        for (name, symbol_size) in [("a.mov", 500u16), ("b.mov", 1000u16)] {
            let video = dir.join(name);
            std::fs::write(&video, b"x").unwrap();
            let sender = Sender::new("same.bin", &vec![1u8; 20_000], symbol_size, 20).unwrap();
            let mut receiver = Receiver::new();
            let mut frames = Vec::new();
            for frame in sender.frames().iter().take(5) {
                receiver.push(frame).unwrap();
                frames.push(frame.clone());
            }
            let progress = receiver.export_progress().unwrap();
            manifests.push(
                super::save_manifest(&video, Some(&progress), &[], &frames)
                    .unwrap()
                    .unwrap(),
            );
        }
        assert!(super::load_resume(&manifests).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A report standing in for "2 MB off a 4K60 recording, verified against the original".
    /// Every field is a number, so `lines` is the only thing that turns it into text.
    fn sample_report(comparison: Option<Comparison>) -> Report {
        Report {
            video: PathBuf::from("/tmp/08.MOV"),
            target: PathBuf::from("/tmp/recv/random2m.bin"),
            bytes: 2_097_152,
            track: TrackFacts {
                secs: 46.2,
                width: 3840,
                height: 2160,
                codec: crate::isobmff::Codec::Hevc,
                frames: 2770,
                fps: 59.96,
            },
            threads: 6,
            frames_decoded: 2770,
            codes_found: 5542,
            unique_symbols: 1231,
            symbols_received: 1893,
            duplicates: 662,
            symbol_size: 1710,
            object_len: 2_097_152,
            source_symbols: 1227,
            resumed: None,
            comparison,
            elapsed_secs: 64.6,
            throughput: 32_464.0,
        }
    }

    /// The whole point of keeping the report as numbers: the same result renders in every
    /// language, with nothing pre-formatted and left behind in the wrong one.
    #[test]
    fn the_report_renders_completely_in_every_language() {
        let report = sample_report(Some(Comparison::Identical {
            original: PathBuf::from("/tmp/original.bin"),
        }));
        for lang in Lang::ALL {
            let lines = report.lines(lang.table());
            let everything = [
                &lines.track_summary,
                &lines.counters,
                &lines.symbols,
                &lines.output,
                &lines.timing,
            ];
            for text in everything {
                assert!(
                    !text.trim().is_empty(),
                    "{} produced an empty line",
                    lang.code()
                );
                assert!(
                    !text.contains("{}"),
                    "{} left a template hole: {text}",
                    lang.code()
                );
            }
            // The numbers the receiver actually judges by have to survive translation.
            assert!(lines.track_summary.contains("3840"), "{}", lang.code());
            assert!(lines.counters.contains("2770"), "{}", lang.code());
            assert!(lines.counters.contains("1231"), "{}", lang.code());
            assert!(lines.symbols.contains("1710"), "{}", lang.code());
            assert!(lines.timing.contains("64.6"), "{}", lang.code());
            assert!(lines.output.contains("2.00 MB"), "{}", lang.code());
            assert!(lines.identical, "{}", lang.code());
            assert!(lines.warnings.is_empty(), "{}", lang.code());
            let verify = lines.verify.expect("a comparison was run");
            assert!(
                verify.starts_with("IDENTICAL ✓"),
                "{}: {verify}",
                lang.code()
            );
            assert!(verify.contains("original.bin"), "{}: {verify}", lang.code());
        }
    }

    /// A mismatch is the one result that must never look like a success, in any language,
    /// and it has to come with its warning.
    #[test]
    fn a_mismatch_is_reported_as_a_mismatch_in_every_language() {
        let report = sample_report(Some(Comparison::Mismatch {
            original: PathBuf::from("/tmp/original.bin"),
            expected: 2_097_152,
            got: 2_097_100,
        }));
        for lang in Lang::ALL {
            let lines = report.lines(lang.table());
            assert!(!lines.identical, "{}", lang.code());
            let verify = lines.verify.expect("a comparison was run");
            assert!(
                verify.starts_with("MISMATCH ✗"),
                "{}: {verify}",
                lang.code()
            );
            assert_eq!(lines.warnings.len(), 1, "{}", lang.code());
            let warning = &lines.warnings[0];
            assert!(warning.contains("2097100"), "{}: {warning}", lang.code());
            assert!(!warning.contains("{}"), "{}: {warning}", lang.code());
        }
    }

    /// Without `--compare` there is nothing to verify, and that is not a failure.
    #[test]
    fn a_report_without_a_comparison_says_so_instead_of_claiming_success() {
        let lines = sample_report(None).lines(Lang::En.table());
        assert!(lines.verify.is_none());
        assert!(!lines.identical);
        assert!(lines.warnings.is_empty());
    }

    /// The recording's own summary is drawn while the job runs, from the same numbers.
    #[test]
    fn the_recording_summary_reads_in_every_language() {
        let report = sample_report(None);
        for lang in Lang::ALL {
            let text = report.track.summary(lang.table(), report.threads);
            assert!(text.contains("3840×2160"), "{}: {text}", lang.code());
            assert!(text.contains("HEVC"), "{}: {text}", lang.code());
            assert!(!text.contains("{}"), "{}: {text}", lang.code());
        }
    }

    #[test]
    fn a_bundled_launch_from_finder_gets_the_window() {
        // No terminal, no arguments: a command line would have nowhere to print.
        assert!(wants_gui_with(&[], true, false, false));
    }

    #[test]
    fn an_explicit_gui_flag_wins_over_everything() {
        assert!(wants_gui_with(
            &args(&["--gui", "录像.mov"]),
            false,
            true,
            true
        ));
    }

    #[test]
    fn a_recording_on_the_command_line_stays_on_the_command_line() {
        assert!(!wants_gui_with(&args(&["录像.mov"]), false, true, false));
        assert!(!wants_gui_with(
            &args(&["录像.mov", "--compare", "原文件"]),
            false,
            true,
            false
        ));
    }

    /// macOS only: on Windows and Linux a bare invocation is a console program and
    /// nothing opens a terminal window on its behalf.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bare_invocation_from_a_terminal_opens_the_window() {
        // What the receiver does when they type the command with no arguments — and
        // what Finder turns a double-click on the bare binary into.
        assert!(wants_gui_with(&[], false, true, false));
    }

    #[test]
    fn the_escape_hatch_keeps_the_command_line() {
        // Scripts and tests set SENDMECONGO_NO_DIALOG so a bare invocation cannot pop a window.
        assert!(!wants_gui_with(&[], false, true, true));
        assert!(!wants_gui_with(&["--quiet".to_string()], true, false, true));
    }

    #[test]
    fn a_quiet_run_is_scripted_not_interactive() {
        // `-q` is only ever passed by something that wants text.
        assert!(!wants_gui_with(&args(&["-q"]), false, true, false));
    }
}
