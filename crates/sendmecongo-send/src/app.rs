//! The sender GUI: pick a file, pick a preset, watch the QR loop run.
//!
//! Everything the operator needs to know to make the optical link work was learned the
//! hard way during M0 testing (see docs/TESTING.md), so the shooting instructions live in
//! the UI rather than in a README nobody opens at the right moment.
//!
//! The file is read and compressed on a worker thread ([`crate::prepare`]), never here.
//! A 5 MB file spends seconds inside brotli, and a window that stops responding while it
//! waits is a window the operator believes has crashed. The finished SMC1 container is
//! kept here and handed to the player process over stdin, so compression happens once
//! per file rather than once per click.

use crate::display::{self, Display};
use crate::prepare::{self, Job, Phase, Prepared};
use sendmecongo_core::preset::{self, Preset};
use sendmecongo_core::RepairRequest;
use sendmecongo_ui::i18n::{self, fill};
use sendmecongo_ui::theme::{self, Theme};
use eframe::egui;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The size the redesigned layout was designed at: the whole send page — segmented
/// control, drop card, the amber recording-time callout, the five settings rows and the
/// action row — fits without scrolling, plus one row for the menu bar the redesign did
/// not have. `window::autosize` shrinks it to fit a smaller monitor; it is the setting
/// list that suffers first when it does.
pub const IDEAL_WINDOW: [f32; 2] = [1000.0, 826.0];

/// A centred dialog, for the cases where silently doing nothing would look like a
/// button that is broken.
#[derive(Clone)]
pub enum Hint {
    NoFile,
    NotReady {
        phase: &'static str,
        permille: u32,
        cancelling: bool,
    },
    Failed(String),
}

/// What the file will cost on the wire, derived from the prepared container.
///
/// Deliberately computed from the container rather than by re-reading and re-compressing
/// the file: the container does not depend on the preset, only the symbol count does. So
/// switching preset restates these numbers instantly.
#[derive(Debug, Clone, PartialEq)]
struct Estimate {
    raw: usize,
    wire: usize,
    /// K — source symbols. The receiver needs K' ≥ K of them, so this is the floor.
    symbols: usize,
    /// Time to play out K symbols once, assuming every frame decodes.
    min_seconds: f64,
    /// The floor doubled. The sender cannot see the receiver's capture rate — the channel
    /// is one-way by design — so the only honest advice is "record at least this much".
    recommended_seconds: f64,
}

/// What the file section should show this frame. Snapshotting it first keeps the
/// drawing code free of borrow puzzles.
#[derive(Clone)]
enum Body {
    Empty,
    Running {
        phase: Phase,
        fraction: f32,
        cancelling: bool,
    },
    Ready {
        estimate: Estimate,
        method: &'static str,
        note: Option<String>,
        secs: f64,
    },
    Failed(String),
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Send,
    Receive,
}

/// How far the file has got.
enum Prep {
    None,
    Running(Job),
    Ready(Prepared),
    Failed(String),
}

pub struct SenderApp {
    tab: Tab,
    file: Option<PathBuf>,
    preset_idx: usize,
    lanes_override: Option<usize>,
    cycles: usize,
    borderless: bool,
    fit_display: bool,
    size_manual: usize,
    displays: Vec<Display>,
    display_idx: usize,
    /// Set once the operator picks a monitor by hand, which stops the auto-follow below
    /// from overriding their choice on the next frame.
    display_pinned: bool,
    manual_origin: Option<(isize, isize)>,
    prep: Prep,
    /// True between pressing "cancel" and the worker actually stopping.
    cancelling: bool,
    hint: Option<Hint>,
    child: Option<Child>,
    /// Last non-empty line the player wrote to stderr, kept so a crash can explain itself.
    child_log: Arc<Mutex<String>>,
    status: String,
    status_bad: bool,
    /// False when no system CJK font was found. That is the only case worth surfacing —
    /// otherwise the font path is build-time trivia the operator has no use for.
    cjk_font_ok: bool,
    /// The "拍摄要求" disclosure starts closed; the amber tag tells the operator it matters.
    tips_open: bool,
    /// M2.3: the receiver's repair code, typed or pasted by the operator. Empty
    /// means a normal full broadcast; a validated code turns the next start into
    /// a repair broadcast of only the missing symbols.
    repair_code: String,
    /// The language this window last drew itself in. The menu bar is nobody's window, so a
    /// language picked there arrives without any event: comparing against this each frame
    /// is how the window notices.
    lang: i18n::Lang,
}

impl SenderApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        let cjk_font_ok = sendmecongo_ui::font::install(&cc.egui_ctx).is_some();
        sendmecongo_ui::font::apply_style(&cc.egui_ctx);
        // Warm the capacity cache off the UI thread: the first caller otherwise pays
        // ~10 ms per preset while encoding QR codes to measure them, which is exactly
        // the kind of hitch this window can no longer afford.
        std::thread::spawn(|| {
            for p in preset::ALL {
                let _ = p.symbol_size();
            }
        });
        let mut app = Self {
            tab: Tab::Send,
            file: None,
            preset_idx: preset::ALL
                .iter()
                .position(|p| p.name == "turbo60")
                .unwrap_or(0),
            lanes_override: None,
            cycles: 0,
            borderless: true,
            fit_display: true,
            size_manual: 1600,
            displays: display::list(),
            display_idx: 0,
            display_pinned: false,
            manual_origin: None,
            prep: Prep::None,
            cancelling: false,
            hint: None,
            child: None,
            child_log: Arc::new(Mutex::new(String::new())),
            status: i18n::t().snd_status_ready.to_string(),
            status_bad: false,
            cjk_font_ok,
            tips_open: false,
            repair_code: String::new(),
            lang: i18n::current(),
        };
        if let Some(path) = initial {
            app.set_file(path);
        }
        app
    }

    fn preset(&self) -> Preset {
        preset::ALL[self.preset_idx]
    }

    /// The repair code as a validated request. `None` when the box is empty
    /// (normal broadcast); `Some(Err)` when it does not parse or does not match
    /// the prepared container and preset. Full validation needs the prepared
    /// object, so a syntactically fine code on an unprepared file is reported
    /// as unverifiable rather than blessed.
    fn repair_request(&self) -> Option<Result<RepairRequest, String>> {
        let text = self.repair_code.trim();
        if text.is_empty() {
            return None;
        }
        let request = match RepairRequest::decode(text) {
            Ok(r) => r,
            Err(e) => return Some(Err(e.to_string())),
        };
        let Prep::Ready(prepared) = &self.prep else {
            return None;
        };
        let matches = sendmecongo_core::crc32(&prepared.object) == request.session
            && prepared.object.len() as u32 == request.object_len
            && self.preset().symbol_size() == request.symbol_size;
        Some(if matches {
            Ok(request)
        } else {
            Err(i18n::t().snd_repair_mismatch.to_string())
        })
    }

    fn lanes(&self) -> usize {
        self.lanes_override
            .unwrap_or(self.preset().lanes as usize)
            .max(1)
    }

    fn handle_drop(&mut self, ctx: &egui::Context) {
        let dropped: Option<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.set_file(path);
        }
    }

    /// Picking a file restarts the whole pipeline: the code stream on screen and the
    /// file named in the window must never be two different things.
    fn set_file(&mut self, path: PathBuf) {
        self.stop();
        self.file = Some(path.clone());
        self.prep = Prep::Running(Job::spawn(path));
        self.cancelling = false;
        self.hint = None;
        self.set_status(i18n::t().snd_status_preparing, false);
    }

    fn clear_file(&mut self) {
        self.stop();
        self.file = None;
        // Dropping the job cancels it.
        self.prep = Prep::None;
        self.cancelling = false;
        self.set_status(i18n::t().snd_status_ready, false);
    }

    fn set_status(&mut self, text: impl Into<String>, bad: bool) {
        self.status = text.into();
        self.status_bad = bad;
    }

    /// Rebuild the status line in the language just chosen.
    ///
    /// Everything else on screen is formatted at draw time and follows the language by
    /// itself. The status line is the exception: it is a snapshot of what happened, so
    /// switching language would otherwise leave the old text sitting there until the
    /// next click. Failure texts are left alone — they carry the worker's own words.
    fn restate_status(&mut self) {
        if self.child.is_some() {
            let text = self.playing_status();
            self.set_status(text, false);
            return;
        }
        let text = match &self.prep {
            Prep::None => i18n::t().snd_status_ready.to_string(),
            Prep::Running(job) => {
                let (phase, fraction) = job.progress();
                progress_text(i18n::t(), phase, fraction, self.cancelling)
            }
            Prep::Ready(prepared) => {
                if prepared.note.is_some() {
                    i18n::t().snd_status_encode_done_skipped.to_string()
                } else {
                    i18n::t().snd_status_encode_done.to_string()
                }
            }
            Prep::Failed(_) => return,
        };
        self.set_status(text, false);
    }

    /// The "playing" status line, also used to restate it after a language change.
    fn playing_status(&self) -> String {
        let p = self.preset();
        let lanes = self.lanes();
        let size = match self.displays.get(self.display_idx) {
            Some(display) if self.fit_display => display.fit(lanes).0,
            _ => self.size_manual.max(200),
        };
        fill(
            i18n::t().snd_status_playing,
            &[&size, &(size / lanes), &lanes, &p.fps],
        )
    }

    /// Default the output monitor to whichever one the GUI window is sitting on — the
    /// operator already put it where they want by dragging it there, and on the common
    /// two-screen air-gap setup (console on the left, camera-facing screen on the right)
    /// this makes the right choice without any configuration.
    fn track_display(&mut self, ctx: &egui::Context) {
        if self.display_pinned || self.displays.len() < 2 {
            return;
        }
        let rect = ctx.input(|i| i.viewport().outer_rect);
        let Some(rect) = rect else {
            return;
        };
        // Windows enumerates in physical pixels while egui rects are logical
        // points. The window sits on the monitor whose scale factor produced
        // those logical coordinates, so multiplying by it lands on the physical
        // position the monitor list uses. macOS bounds are already logical, and
        // the multiplication would corrupt them there — hence the cfg.
        #[cfg(windows)]
        let center = {
            let scale = ctx
                .input(|i| i.viewport().native_pixels_per_point)
                .unwrap_or(1.0) as f64;
            rect.center().x as f64 * scale
        };
        #[cfg(not(windows))]
        let center = rect.center().x as f64;
        let center = center as isize;
        if let Some(idx) = self
            .displays
            .iter()
            .position(|d| center >= d.x && center < d.x + d.width as isize)
        {
            self.display_idx = idx;
        }
    }

    fn body(&self) -> Body {
        match &self.prep {
            Prep::None => Body::Empty,
            Prep::Running(job) => {
                let (phase, fraction) = job.progress();
                Body::Running {
                    phase,
                    fraction,
                    cancelling: self.cancelling,
                }
            }
            Prep::Ready(prepared) => Body::Ready {
                estimate: estimate_for(prepared, &self.preset()),
                method: prepared.method(),
                note: prepared.note.clone(),
                secs: prepared.secs,
            },
            Prep::Failed(msg) => Body::Failed(msg.clone()),
        }
    }

    fn cancel_prep(&mut self) {
        if let Prep::Running(job) = &self.prep {
            job.cancel();
            self.cancelling = true;
        }
    }

    /// Re-run the pipeline. Only needed if the file changed on disk — picking a new
    /// preset does *not* require it, because the container is preset-independent.
    fn recook(&mut self) {
        if let Some(path) = self.file.clone() {
            self.set_file(path);
        }
    }

    fn poll_prep(&mut self, ctx: &egui::Context) {
        let mut outcome = None;
        let mut waiting = false;
        let mut live = (Phase::Read, 0.0);
        if let Prep::Running(job) = &mut self.prep {
            outcome = job.poll();
            if outcome.is_none() {
                waiting = true;
                live = job.progress();
            }
        }

        if let Some(result) = outcome {
            match result {
                Ok(prepared) => {
                    let note = prepared.note.is_some();
                    self.prep = Prep::Ready(prepared);
                    self.cancelling = false;
                    self.set_status(
                        if note {
                            i18n::t().snd_status_encode_done_skipped
                        } else {
                            i18n::t().snd_status_encode_done
                        },
                        false,
                    );
                }
                Err(msg) if msg == prepare::CANCELLED => {
                    self.prep = Prep::None;
                    self.cancelling = false;
                    self.set_status(i18n::t().snd_status_encode_cancelled, false);
                }
                Err(msg) => {
                    self.prep = Prep::Failed(msg.clone());
                    self.cancelling = false;
                    self.hint = Some(Hint::Failed(msg));
                }
            }
        }

        if waiting {
            let (phase, fraction) = live;
            let text = progress_text(i18n::t(), phase, fraction, self.cancelling);
            self.set_status(text, false);
            // Without this the window stops repainting while it waits, and the progress
            // bar would freeze — the exact symptom this whole change exists to remove.
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn start(&mut self) {
        match &self.prep {
            Prep::Ready(_) => {}
            Prep::None if self.file.is_none() => {
                self.hint = Some(Hint::NoFile);
                self.set_status(i18n::t().snd_status_no_file, true);
                return;
            }
            Prep::None => {
                // The file is there but nothing was prepared — re-run rather than nag.
                self.recook();
                return;
            }
            Prep::Running(job) => {
                let (phase, fraction) = job.progress();
                self.hint = Some(Hint::NotReady {
                    phase: phase.label(i18n::t()),
                    permille: (fraction * 1000.0) as u32,
                    cancelling: self.cancelling,
                });
                return;
            }
            Prep::Failed(msg) => {
                self.hint = Some(Hint::Failed(msg.clone()));
                return;
            }
        }

        let prepared = match &self.prep {
            Prep::Ready(p) => Arc::clone(&p.object),
            _ => return,
        };
        self.stop();
        if let Err(e) = self.spawn_player(prepared) {
            self.set_status(e, true);
        }
    }

    /// Start the player, handing it the already-compressed container over stdin.
    fn spawn_player(&mut self, object: Arc<Vec<u8>>) -> Result<(), String> {
        let p = self.preset();
        let lanes = self.lanes();
        let (size, fitted) = if self.fit_display {
            self.displays[self.display_idx.min(self.displays.len() - 1)].fit(lanes)
        } else {
            (self.size_manual.max(200), (0isize, 0isize))
        };
        let origin = self.manual_origin.unwrap_or(fitted);

        let exe = std::env::current_exe().map_err(|e| fill(i18n::t().snd_error_self_exe, &[&e]))?;
        let mut cmd = Command::new(exe);
        cmd.arg("--play")
            .arg("--object-stdin")
            .arg("--preset")
            .arg(p.name)
            .arg("--size")
            .arg(size.to_string())
            .arg("--lanes")
            .arg(lanes.to_string())
            // The child inherits our environment but not our choice, so say it out loud
            // and the one line it may print on failure comes back in the same language.
            .arg("--lang")
            .arg(i18n::current().code());
        if self.borderless {
            cmd.arg("--borderless")
                .arg("--at-x")
                .arg(origin.0.to_string())
                .arg("--at-y")
                .arg(origin.1.to_string());
        }
        if self.cycles > 0 {
            cmd.arg("--cycles").arg(self.cycles.to_string());
        }
        // A validated repair code turns this into a repair broadcast; the player
        // re-validates against the container it receives, authoritatively.
        if let Some(Ok(_)) = self.repair_request() {
            cmd.arg("--repair-code").arg(self.repair_code.trim());
        }

        // stdin carries the container; stdout is dropped and stderr is captured. On
        // Windows a console-subsystem child would otherwise put a black box on screen.
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| fill(i18n::t().snd_error_spawn_player, &[&e]))?;

        if let Some(mut stdin) = child.stdin.take() {
            // Off the UI thread: a multi-megabyte write can outlast the pipe buffer,
            // and the window must not wait on the child to start reading.
            std::thread::spawn(move || {
                use std::io::Write;
                let _ = stdin.write_all(&object);
                let _ = stdin.flush();
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let log = Arc::clone(&self.child_log);
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = line.trim();
                            if !text.is_empty() {
                                if let Ok(mut slot) = log.lock() {
                                    *slot = text.to_string();
                                }
                            }
                        }
                    }
                }
            });
        }

        self.child = Some(child);
        let status = self.playing_status();
        self.set_status(status, false);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            self.set_status(i18n::t().snd_status_stopped, false);
        }
    }

    fn poll_child(&mut self) {
        let outcome = match self.child.as_mut() {
            Some(child) => child.try_wait(),
            None => return,
        };
        match outcome {
            Ok(Some(exit)) => {
                self.child = None;
                if exit.success() {
                    self.set_status(i18n::t().snd_status_playback_ended, false);
                    return;
                }
                let last = self.child_log.lock().map(|l| l.clone()).unwrap_or_default();
                let text = if last.is_empty() {
                    fill(i18n::t().snd_status_player_failed, &[&exit])
                } else {
                    fill(i18n::t().snd_status_player_failed_log, &[&exit, &last])
                };
                self.set_status(text, true);
            }
            Ok(None) => {}
            Err(e) => {
                self.child = None;
                self.set_status(fill(i18n::t().snd_status_child_error, &[&e]), true);
            }
        }
    }

    /// A centred dialog for the cases where doing nothing would look like a dead button.
    fn ui_hint(&mut self, ctx: &egui::Context, t: &Theme) {
        let Some(hint) = self.hint.clone() else {
            return;
        };
        let cancelling = self.cancelling;
        let mut close = false;
        let mut pick = false;
        let mut cancel = false;

        egui::Modal::new(egui::Id::new("sendmecongo-hint")).show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.set_max_width(430.0);
            ui.label(
                egui::RichText::new(i18n::t().snd_hint_title)
                    .size(16.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(8.0);
            match &hint {
                Hint::NoFile => {
                    ui.label(egui::RichText::new(i18n::t().snd_hint_no_file).color(t.text_primary));
                }
                Hint::NotReady {
                    phase,
                    permille,
                    cancelling,
                } => {
                    if *cancelling {
                        ui.label(
                            egui::RichText::new(i18n::t().snd_hint_cancelling)
                                .color(t.text_primary),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(i18n::t().snd_hint_not_ready).color(t.text_primary),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(fill(
                                i18n::t().snd_hint_current,
                                &[phase, &format!("{:.0}", *permille as f32 / 10.0)],
                            ))
                            .color(t.text_secondary),
                        );
                    }
                }
                Hint::Failed(msg) => {
                    ui.label(
                        egui::RichText::new(i18n::t().snd_hint_encode_failed)
                            .strong()
                            .color(t.error),
                    );
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(msg).color(t.text_primary));
                }
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if matches!(hint, Hint::NoFile)
                    && theme::primary_button(ui, t, i18n::t().snd_pick_file, false, true).clicked()
                {
                    pick = true;
                }
                let offer_cancel = matches!(hint, Hint::NotReady { .. }) && !cancelling;
                if offer_cancel
                    && theme::plain_button(ui, t, i18n::t().snd_cancel_encoding).clicked()
                {
                    cancel = true;
                }
                if theme::plain_button(ui, t, i18n::t().snd_got_it).clicked() {
                    close = true;
                }
            });
        });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }

        if pick {
            close = true;
            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::t().snd_pick_dialog_title)
                .pick_file()
            {
                self.set_file(path);
            }
        }
        if cancel {
            close = true;
            self.cancel_prep();
        }
        if close {
            self.hint = None;
        }
    }

    fn ui_file(&mut self, ui: &mut egui::Ui, t: &Theme) {
        if self.file.is_none() {
            let mut pick = false;
            theme::drop_zone(ui, t, 132.0, |ui| {
                upload_badge(ui, t);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(i18n::t().snd_drop_title)
                        .size(15.0)
                        .strong()
                        .color(t.text_primary),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(i18n::t().snd_drop_sub)
                        .size(12.0)
                        .color(t.text_secondary),
                );
                ui.add_space(10.0);
                if theme::plain_button(ui, t, i18n::t().snd_pick_file).clicked() {
                    pick = true;
                }
            });
            if pick {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(i18n::t().snd_pick_dialog_title)
                    .pick_file()
                {
                    self.set_file(path);
                }
            }
            return;
        }

        let body = self.body();
        let path = self.file.clone().unwrap_or_default();
        let playing = self.child.is_some();
        let mut pick = false;
        let mut clear = false;
        let mut cancel = false;

        // The file card: what is being sent, plus the one-line encoding summary.
        theme::card(t).show(ui, |ui| {
            ui.horizontal(|ui| {
                ext_badge(ui, t, &path);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(file_name_of(&path))
                            .size(14.0)
                            .strong()
                            .color(t.text_primary),
                    );
                    match &body {
                        Body::Ready {
                            estimate,
                            method,
                            secs,
                            ..
                        } => {
                            ui.label(
                                egui::RichText::new(fill(
                                    i18n::t().snd_file_summary,
                                    &[
                                        &prepare::human(estimate.raw),
                                        &prepare::human(estimate.wire),
                                        &estimate.symbols,
                                        method,
                                        &format!("{secs:.1}"),
                                    ],
                                ))
                                .size(12.0)
                                .color(t.text_secondary),
                            );
                        }
                        Body::Running { .. } => {
                            ui.label(
                                egui::RichText::new(i18n::t().snd_encoding)
                                    .size(12.0)
                                    .color(t.text_secondary),
                            );
                        }
                        Body::Failed(msg) => {
                            ui.label(egui::RichText::new(msg).size(12.0).color(t.error));
                        }
                        Body::Empty => {}
                    }
                });
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match &body {
                        Body::Running { cancelling, .. } => {
                            if !cancelling
                                && theme::text_button(ui, t, i18n::t().cancel, Some(t.error))
                                    .clicked()
                            {
                                cancel = true;
                            }
                        }
                        _ => {
                            if theme::text_button(ui, t, i18n::t().clear, Some(t.text_secondary))
                                .clicked()
                            {
                                clear = true;
                            }
                            if theme::text_button(ui, t, i18n::t().change, None).clicked() {
                                pick = true;
                            }
                        }
                    },
                );
            });
        });

        match &body {
            Body::Running {
                phase,
                fraction,
                cancelling,
            } => {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let text = progress_text(i18n::t(), *phase, *fraction, *cancelling);
                    ui.label(
                        egui::RichText::new(text)
                            .size(12.5)
                            .strong()
                            .color(t.text_primary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{:.0}%", fraction * 100.0))
                                .size(12.5)
                                .color(t.text_secondary),
                        );
                    });
                });
                ui.add_space(-4.0);
                ui.add(
                    egui::ProgressBar::new(*fraction)
                        .desired_height(6.0)
                        .corner_radius(3.0),
                );
                ui.label(
                    egui::RichText::new(if *cancelling {
                        i18n::t().snd_cancelling_soon
                    } else {
                        i18n::t().snd_progress_note
                    })
                    .size(11.5)
                    .color(t.text_tertiary),
                );
            }
            Body::Ready { estimate, note, .. } => {
                if let Some(note) = note {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new(note).size(12.0).color(t.warn_sub));
                    });
                }
                ui.add_space(10.0);
                // The single most important number on this screen: the operator has to
                // know how long to keep filming, and nothing else tells them.
                theme::callout(t).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("⏱").size(20.0));
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let title = if playing {
                                fill(
                                    i18n::t().snd_callout_playing,
                                    &[&duration_text(i18n::t(), estimate.recommended_seconds)],
                                )
                            } else {
                                fill(
                                    i18n::t().snd_callout_idle,
                                    &[&duration_text(i18n::t(), estimate.recommended_seconds)],
                                )
                            };
                            ui.label(
                                egui::RichText::new(title)
                                    .size(17.0)
                                    .strong()
                                    .color(t.warn_text),
                            );
                            ui.label(
                                egui::RichText::new(if playing {
                                    i18n::t().snd_callout_playing_sub.to_string()
                                } else {
                                    fill(
                                        i18n::t().snd_callout_idle_sub,
                                        &[&duration_text(i18n::t(), estimate.min_seconds)],
                                    )
                                })
                                .size(12.5)
                                .color(t.warn_sub),
                            );
                        });
                    });
                });
            }
            _ => {}
        }

        if pick {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::t().snd_pick_dialog_title)
                .pick_file()
            {
                self.set_file(path);
            }
        }
        if clear {
            self.clear_file();
        }
        if cancel {
            self.cancel_prep();
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui, t: &Theme) {
        theme::section_title(ui, t, i18n::t().snd_settings_title);
        let p = self.preset();
        theme::card_edge(t).show(ui, |ui| {
            setting_row(ui, t, i18n::t().snd_set_preset, |ui| {
                egui::ComboBox::from_id_salt("preset")
                    .width(330.0)
                    .selected_text(
                        egui::RichText::new(fill(
                            i18n::t().snd_preset_selected,
                            &[
                                &p.name,
                                &format!("{:.0}", p.fps),
                                &format!("{:.0}", p.nominal_bps() / 1024.0),
                            ],
                        ))
                        .size(13.0),
                    )
                    .show_ui(ui, |ui| {
                        for (i, cand) in preset::ALL.iter().enumerate() {
                            let tag = if cand.lanes > 1 {
                                i18n::t().snd_preset_dual
                            } else {
                                ""
                            };
                            ui.selectable_value(
                                &mut self.preset_idx,
                                i,
                                fill(
                                    i18n::t().snd_preset_option,
                                    &[
                                        &cand.name,
                                        &format!("{:.0}", cand.fps),
                                        &format!("{:.0}", cand.nominal_bps() / 1024.0),
                                        &tag,
                                    ],
                                ),
                            );
                        }
                    });
            });
            theme::hairline(ui, t);

            setting_row(ui, t, i18n::t().snd_set_lanes, |ui| {
                let default = self.preset().lanes as usize;
                theme::segmented(
                    ui,
                    t,
                    &mut self.lanes_override,
                    &[
                        (None, fill(i18n::t().snd_lanes_auto, &[&default])),
                        (Some(1), "1".to_string()),
                        (Some(2), "2".to_string()),
                    ],
                );
            });
            theme::hairline(ui, t);

            setting_row(ui, t, i18n::t().snd_set_display, |ui| {
                ui.checkbox(
                    &mut self.fit_display,
                    egui::RichText::new(i18n::t().snd_fit_display).size(13.0),
                );
                if self.fit_display {
                    // Snapshot the labels: the closure below also writes to self, and
                    // iterating the display list while doing that won't borrow-check.
                    let labels: Vec<(String, bool)> = self
                        .displays
                        .iter()
                        .map(|d| (d.label.clone(), d.primary))
                        .collect();
                    let mut idx = self.display_idx;
                    let current = labels.get(idx).map(|(l, _)| l.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("display")
                        .selected_text(egui::RichText::new(current).size(13.0))
                        .show_ui(ui, |ui| {
                            for (i, (label, primary)) in labels.iter().enumerate() {
                                let tag = if *primary {
                                    i18n::t().snd_display_main_tag
                                } else {
                                    ""
                                };
                                ui.selectable_value(&mut idx, i, format!("{label}{tag}"));
                            }
                        });
                    if idx != self.display_idx {
                        self.display_idx = idx;
                        self.display_pinned = true;
                    }
                } else {
                    ui.add(
                        egui::Slider::new(&mut self.size_manual, 600..=2400)
                            .text(i18n::t().snd_window_width),
                    );
                }
            });
            theme::hairline(ui, t);

            setting_row(ui, t, i18n::t().snd_set_window, |ui| {
                ui.checkbox(
                    &mut self.borderless,
                    egui::RichText::new(i18n::t().snd_borderless).size(13.0),
                );
                if let Some(d) = self.displays.get(self.display_idx) {
                    let (w, origin) = d.fit(self.lanes());
                    ui.label(
                        egui::RichText::new(fill(
                            i18n::t().snd_will_fill,
                            &[&w, &(w / self.lanes()), &origin.0, &origin.1],
                        ))
                        .size(11.5)
                        .color(t.text_tertiary),
                    );
                }
            });
            theme::hairline(ui, t);

            setting_row(ui, t, i18n::t().snd_set_cycles, |ui| {
                ui.add(
                    egui::Slider::new(&mut self.cycles, 0..=20).text(i18n::t().snd_cycles_endless),
                );
            });
        });
    }

    fn ui_play(&mut self, ui: &mut egui::Ui, t: &Theme) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let playing = self.child.is_some();
            let ready = matches!(self.prep, Prep::Ready(_));
            if playing {
                if theme::danger_button(ui, t, i18n::t().snd_stop_play, true).clicked() {
                    self.stop();
                }
            } else {
                // Stay clickable even when it cannot start anything: a click that
                // explains itself beats a button that looks dead.
                if theme::primary_button(ui, t, i18n::t().snd_play, true, ready).clicked() {
                    self.start();
                }
                let hint = match &self.prep {
                    Prep::Running(_) => i18n::t().snd_encoding_hint,
                    Prep::Failed(_) => i18n::t().snd_encode_failed_hint,
                    Prep::None => i18n::t().snd_pick_first_hint,
                    Prep::Ready(_) => "",
                };
                if !hint.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(hint).size(12.0).color(t.text_tertiary));
                }
            }
            ui.add_space(4.0);
            if theme::text_button(ui, t, i18n::t().snd_reencode, None).clicked() {
                self.recook();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(i18n::t().snd_esc_stops)
                        .size(11.5)
                        .color(t.text_tertiary),
                );
            });
        });

        self.ui_repair(ui, t);
    }

    /// M2.3: the repair-code box. Empty means a normal broadcast; a code that
    /// parses and matches the prepared container turns the next start into a
    /// replay of only the missing symbols.
    fn ui_repair(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let i = i18n::t();
        ui.add_space(10.0);
        theme::card_edge(t).show(ui, |ui| {
            ui.label(
                egui::RichText::new(i.snd_repair_title)
                    .size(13.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(i.snd_repair_hint)
                    .size(11.5)
                    .color(t.text_secondary),
            );
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.repair_code)
                    .hint_text(i.snd_repair_placeholder)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            match self.repair_request() {
                Some(Ok(request)) => {
                    let Prep::Ready(prepared) = &self.prep else {
                        return;
                    };
                    let full = prepared.symbols(self.preset().symbol_size()) as u64
                        * (100 + self.preset().repair_pct as u64)
                        / 100;
                    let pct = (request.total() as u64 * 100 / full.max(1)) as u32;
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(fill(i.snd_repair_ok, &[&request.total(), &pct]))
                            .size(12.0)
                            .color(t.success),
                    );
                }
                Some(Err(message)) => {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(fill(i.snd_repair_invalid, &[&message]))
                            .size(12.0)
                            .color(t.error),
                    );
                }
                None => {}
            }
        });
    }

    fn ui_tips(&mut self, ui: &mut egui::Ui, t: &Theme) {
        ui.add_space(14.0);
        let lanes = self.lanes();
        theme::card_edge(t).show(ui, |ui| {
            let header = egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(if self.tips_open { "▾" } else { "▸" })
                                .size(11.0)
                                .color(t.text_tertiary),
                        );
                        ui.label(
                            egui::RichText::new(i18n::t().snd_tips_title)
                                .size(13.5)
                                .strong()
                                .color(t.text_primary),
                        );
                        ui.add_space(6.0);
                        theme::chip(ui, t, i18n::t().snd_tips_chip);
                    });
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if header.clicked() {
                self.tips_open = !self.tips_open;
            }

            if self.tips_open {
                theme::hairline(ui, t);
                egui::Frame::default()
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        let mut tips: Vec<&'static str> = Vec::new();
                        if lanes > 1 {
                            tips.push(i18n::t().snd_tip_dual_4k60);
                            tips.push(i18n::t().snd_tip_dual_30fps);
                        } else {
                            tips.push(i18n::t().snd_tip_single_4k30);
                        }
                        tips.push(i18n::t().snd_tip_lock_focus);
                        tips.push(i18n::t().snd_tip_tripod);
                        tips.push(i18n::t().snd_tip_dnd);
                        tips.push(i18n::t().snd_tip_format);

                        let split = tips.len().div_ceil(2);
                        let (left, right) = (tips[..split].to_vec(), tips[split..].to_vec());
                        ui.columns(2, |cols| {
                            for (items, col) in [&left, &right].into_iter().zip(cols.iter_mut()) {
                                for tip in items {
                                    col.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("✓")
                                                .size(11.0)
                                                .strong()
                                                .color(t.success),
                                        );
                                        ui.label(
                                            egui::RichText::new(*tip)
                                                .size(12.5)
                                                .color(t.text_secondary),
                                        );
                                    });
                                }
                            }
                        });

                        if !self.cjk_font_ok {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(i18n::t().cjk_font_missing)
                                    .size(12.0)
                                    .color(t.warn_sub),
                            );
                        }
                    });
            }
        });
    }

    /// The receiving side is a different person on a different machine, and nothing else
    /// in this app tells them anything. Spell out their whole job here.
    ///
    /// Their job is deliberately *not* "install this toolchain": the whole point of
    /// `sendmecongo-recv` is that they get one file and drag the recording onto it. So this
    /// page has to explain where that file comes from, and what the tool will ask of them
    /// (nothing) — the previous version of this page handed out a shell pipeline and a
    /// pip install, which is not something you can ask of the person holding the phone.
    fn ui_receive(&mut self, ui: &mut egui::Ui, t: &Theme) {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(i18n::t().snd_rcv_intro)
                .size(13.0)
                .color(t.text_secondary),
        );
        ui.add_space(10.0);

        let dual = self.preset().lanes > 1;
        theme::card_edge(t).show(ui, |ui| {
            step_row(ui, t, "1", i18n::t().snd_rcv_step1_title, |ui| {
                step_text(ui, t, i18n::t().snd_rcv_step1_body);
                if dual {
                    ui.label(
                        egui::RichText::new(i18n::t().snd_rcv_step1_dual)
                            .size(12.5)
                            .color(t.warn_text),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(i18n::t().snd_rcv_step1_single)
                            .size(12.5)
                            .color(t.text_secondary),
                    );
                }
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_step1_codec)
                        .size(12.0)
                        .color(t.text_tertiary),
                );
            });
            theme::hairline(ui, t);

            step_row(ui, t, "2", i18n::t().snd_rcv_step2_title, |ui| {
                step_text(ui, t, i18n::t().snd_rcv_step2_movie);
                step_text(ui, t, i18n::t().snd_rcv_step2_tool);
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_step2_note)
                        .size(12.0)
                        .color(t.text_tertiary),
                );
            });
            theme::hairline(ui, t);

            step_row(ui, t, "3", i18n::t().snd_rcv_step3_title, |ui| {
                step_text(ui, t, i18n::t().snd_rcv_step3_body);
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_step3_warn)
                        .size(12.5)
                        .color(t.warn_text),
                );
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_step3_cli_note)
                        .size(12.0)
                        .color(t.text_tertiary),
                );
                ui.code(egui::RichText::new(i18n::t().snd_rcv_step3_cli).size(11.5));
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_step3_tail)
                        .size(12.0)
                        .color(t.text_tertiary),
                );
            });
            theme::hairline(ui, t);

            step_row(ui, t, "4", i18n::t().snd_rcv_step4_title, |ui| {
                step_text(ui, t, i18n::t().snd_rcv_step4_body);
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_step4_note)
                        .size(12.5)
                        .color(t.text_secondary),
                );
            });
        });

        ui.add_space(12.0);
        theme::callout(t).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("💡").size(16.0));
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(i18n::t().snd_rcv_callout)
                        .size(12.5)
                        .color(t.warn_sub),
                );
            });
        });
    }
}

impl eframe::App for SenderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The menu bar's language menu. Building it needs the application object, which
        // exists by the time the first frame is drawn; `install` is idempotent, so calling
        // it every frame simply costs one comparison.
        sendmecongo_ui::menu::install(i18n::t().snd_title);
        if self.lang != i18n::current() {
            self.lang = i18n::current();
            i18n::retitle(ctx, i18n::t().snd_title);
            sendmecongo_ui::menu::refresh(i18n::t().snd_title);
            self.restate_status();
        }

        sendmecongo_ui::window::autosize(ctx, IDEAL_WINDOW);
        self.poll_child();
        self.poll_prep(ctx);
        self.handle_drop(ctx);
        self.track_display(ctx);
        let t = theme::theme(ctx);

        // macOS keeps menus in the menu bar at the top of the screen, which belongs to the
        // application rather than to this window (see `sendmecongo_ui::menu`). Everywhere else
        // the window has to draw its own row.
        #[cfg(not(target_os = "macos"))]
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::default()
                    .fill(t.window_bg)
                    .stroke(egui::Stroke::new(1.0_f32, t.hairline))
                    .inner_margin(egui::Margin::symmetric(14, 2)),
            )
            .show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    if i18n::language_menu(ui) {
                        // The title is the platform's, and the status line is a snapshot:
                        // both have to be told that the language moved.
                        i18n::retitle(ctx, i18n::t().snd_title);
                        self.restate_status();
                    }
                });
            });

        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::default()
                    .fill(t.window_bg)
                    .stroke(egui::Stroke::new(1.0_f32, t.hairline))
                    .inner_margin(egui::Margin::symmetric(22, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let busy = self.child.is_some() || matches!(self.prep, Prep::Running(_));
                    let color = if self.status_bad {
                        t.error
                    } else if busy {
                        t.pending
                    } else {
                        t.success
                    };
                    theme::status_dot(ui, color);
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new(&self.status).size(12.0).color(
                        if self.status_bad {
                            t.error
                        } else {
                            t.text_secondary
                        },
                    ));
                });
            });

        egui::CentralPanel::default()
            .frame(theme::window_frame(&t))
            .show(ctx, |ui| {
                theme::segmented(
                    ui,
                    &t,
                    &mut self.tab,
                    &[
                        (Tab::Send, i18n::t().snd_tab_send.to_string()),
                        (Tab::Receive, i18n::t().snd_tab_receive.to_string()),
                    ],
                );
                egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                    Tab::Send => {
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(i18n::t().snd_intro)
                                .size(13.0)
                                .color(t.text_secondary),
                        );
                        ui.add_space(8.0);
                        self.ui_file(ui, &t);
                        self.ui_settings(ui, &t);
                        self.ui_play(ui, &t);
                        self.ui_tips(ui, &t);
                        ui.add_space(8.0);
                    }
                    Tab::Receive => self.ui_receive(ui, &t),
                });
            });

        // Last, so it sits above everything and swallows input while it is up.
        self.ui_hint(ctx, &t);
    }
}

/// One labelled row inside the settings group list.
fn setting_row(ui: &mut egui::Ui, t: &Theme, key: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(14, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [64.0, 22.0],
                    egui::Label::new(egui::RichText::new(key).size(13.5).color(t.text_primary)),
                );
                ui.add_space(10.0);
                add(ui);
            });
        });
}

/// One numbered step inside the "接收说明" card.
fn step_row(ui: &mut egui::Ui, t: &Theme, num: &str, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(15, 14))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 13.0, t.accent);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    num,
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .size(14.0)
                            .strong()
                            .color(t.text_primary),
                    );
                    add(ui);
                });
            });
        });
}

/// Body text inside a step row.
fn step_text(ui: &mut egui::Ui, t: &Theme, text: &str) {
    ui.label(egui::RichText::new(text).size(12.5).color(t.text_secondary));
}

/// The blue upload glyph at the centre of the drop zone.
fn upload_badge(ui: &mut egui::Ui, t: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 52.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 13.0, t.accent);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "↑",
        egui::FontId::proportional(28.0),
        egui::Color32::WHITE,
    );
}

/// The grey file-type tile on the left of the file card.
fn ext_badge(ui: &mut egui::Ui, t: &Theme, path: &std::path::Path) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_uppercase())
        .filter(|e| !e.is_empty() && e.chars().count() <= 5)
        .unwrap_or_else(|| "FILE".to_string());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 9.0, t.control_bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ext,
        egui::FontId::proportional(10.0),
        t.text_secondary,
    );
}

/// The display name of a path, falling back to the whole path when it has no file name.
fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The symbol count and filming time depend on the preset; the container does not.
/// Deriving both from one prepared container is what makes changing preset instant.
fn estimate_for(prepared: &Prepared, p: &Preset) -> Estimate {
    let symbols = prepared.symbols(p.symbol_size());
    // Playing out K symbols takes K/fps seconds; the stream loops, so this is the
    // point at which enough *distinct* symbols have gone past the lens.
    let min_seconds = symbols as f64 / p.fps;
    Estimate {
        raw: prepared.raw,
        wire: prepared.wire(),
        symbols,
        min_seconds,
        recommended_seconds: min_seconds * 2.0,
    }
}

/// The status/heading line for a job in flight: "正在读取文件 42%" while encoding, and
/// the plain "正在取消 42%" once the operator has asked it to stop.
fn progress_text(t: &i18n::Text, phase: Phase, fraction: f32, cancelling: bool) -> String {
    let percent = format!("{:.0}", fraction * 100.0);
    if cancelling {
        fill(t.snd_status_cancelling, &[&percent])
    } else {
        fill(t.snd_status_phase, &[&phase.label(t), &percent])
    }
}

/// Advice about how long to film must never round down, so this always rounds up.
fn duration_text(t: &i18n::Text, secs: f64) -> String {
    let total = secs.ceil().max(1.0) as u64;
    if total < 60 {
        return fill(t.seconds_short, &[&total]);
    }
    let (minutes, rest) = (total / 60, total % 60);
    if rest == 0 {
        fill(t.minutes_only, &[&minutes])
    } else {
        fill(t.minutes_seconds, &[&minutes, &rest])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(object_len: usize) -> Prepared {
        Prepared {
            raw: object_len / 2,
            object: Arc::new(vec![0u8; object_len]),
            comp: sendmecongo_core::container::COMP_RAW,
            note: None,
            secs: 0.1,
        }
    }

    /// The bug this replaced: switching preset left the old preset's symbol count and
    /// filming advice on screen, because nothing recomputed them.
    #[test]
    fn switching_preset_restates_the_numbers_without_touching_the_container() {
        let file = prepared(300_000);
        let turbo = estimate_for(&file, &preset::TURBO60);
        let mega = estimate_for(&file, &preset::MEGABIT);
        assert_eq!(turbo.wire, mega.wire, "the container is preset-independent");
        assert_eq!(turbo.wire, 300_000);
        assert_ne!(
            turbo.symbols, mega.symbols,
            "symbol size differs between v30 and v40, so K must too"
        );
        assert!(turbo.recommended_seconds > turbo.min_seconds);
    }

    #[test]
    fn filming_advice_never_rounds_down() {
        use sendmecongo_ui::i18n::Lang;
        let hans = Lang::ZhHans.table();
        assert_eq!(duration_text(hans, 0.2), "1 秒");
        assert_eq!(duration_text(hans, 59.0), "59 秒");
        assert_eq!(
            duration_text(hans, 59.5),
            "1 分钟",
            "rounds up across the boundary"
        );
        assert_eq!(duration_text(hans, 60.0), "1 分钟");
        assert_eq!(duration_text(hans, 125.0), "2 分 5 秒");
        assert_eq!(duration_text(hans, 120.0), "2 分钟");
    }

    /// Every language has to be able to say the same thing. A template with a missing
    /// `{}` leaves the placeholder visible on screen, which is exactly the sort of thing
    /// that survives a translation review.
    #[test]
    fn the_filming_advice_reads_in_every_language() {
        for lang in sendmecongo_ui::i18n::Lang::ALL {
            for secs in [1.0, 59.0, 59.5, 60.0, 120.0, 125.0, 3599.0, 3600.0] {
                let text = duration_text(lang.table(), secs);
                assert!(
                    !text.trim().is_empty(),
                    "{} said nothing for {secs}",
                    lang.code()
                );
                assert!(
                    !text.contains("{}"),
                    "{} left a template hole for {secs}: {text}",
                    lang.code()
                );
                // Under a minute the seconds are literal; over it, the minutes are.
                let total = secs.ceil().max(1.0) as u64;
                let expected = if total < 60 { total } else { total / 60 };
                assert!(
                    text.contains(&expected.to_string()),
                    "{} lost the number for {secs}: {text}",
                    lang.code()
                );
            }
        }
    }

    /// The progress line and the status line are the two places a stale language is
    /// most visible, because they are overwritten in place rather than redrawn.
    #[test]
    fn the_progress_line_is_complete_in_every_language() {
        for lang in sendmecongo_ui::i18n::Lang::ALL {
            let text = progress_text(lang.table(), Phase::Brotli, 0.42, false);
            assert!(text.contains("42%"), "{}: {text}", lang.code());
            assert!(
                text.contains(Phase::Brotli.label(lang.table())),
                "{}: {text}",
                lang.code()
            );
            let cancelling = progress_text(lang.table(), Phase::Brotli, 0.42, true);
            assert!(cancelling.contains("42%"), "{}: {cancelling}", lang.code());
            assert!(!cancelling.contains("{}"), "{}: {cancelling}", lang.code());
        }
    }
}
