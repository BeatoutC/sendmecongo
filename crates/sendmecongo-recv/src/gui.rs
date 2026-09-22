//! The receiver's window.
//!
//! A bundled app has no terminal, so the receiver used to sit in front of a
//! bouncing Dock icon for a minute with no idea whether anything was happening.
//! This is the answer: pick a recording (or drop one on the window), watch a
//! progress bar that is tied to something real, then see what came out.
//!
//! The bar's denominator is not a guess. The first frame header the collector
//! reads carries the object length and the symbol size, so `K` — how many
//! distinct symbols the file is actually made of — is known within the first
//! second, and `Counters::fraction` is literally "collected ÷ needed".
//!
//! Work happens on a worker thread; the window only reads `Counters` (atomics)
//! and polls a channel for the result. Nothing in the pipeline knows the UI
//! exists, which is why the command line can drive exactly the same code.

use crate::pipeline::{self, Counters, CANCELLED};
use crate::{execute, human_bytes, human_duration, prepare, Job, Options, Report, TrackFacts};
use sendmecongo_ui::i18n::{self, fill};
use sendmecongo_ui::theme::{self, Theme};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How many candidate rows fit before the list card scrolls internally — the window
/// height must never depend on how many recordings happen to lie around.
const VISIBLE_ROWS: f32 = 5.0;
/// Measured from the rendered row, not estimated: the icon is 32 pt and the two
/// margins plus the hairline take the rest. Understating it does not clip anything —
/// the list still scrolls — but the card comes out shorter than the five rows it
/// advertises, which looks like a bug.
const ROW_HEIGHT: f32 = 60.0;
/// More candidates than this and the filter box appears at the top of the list card.
const FILTER_THRESHOLD: usize = 8;
/// The size the layout was designed at, plus the menu bar strip on the platforms that
/// draw one inside the window (macOS uses the system menu bar and needs no room for it).
/// `window::autosize` shrinks it on a small monitor but never grows past it.
const IDEAL_WINDOW: [f32; 2] = [
    900.0,
    if cfg!(target_os = "macos") {
        740.0
    } else {
        766.0
    },
];

pub fn launch(args: &[String]) -> Result<(), String> {
    let (options, video) = crate::parse_args(args)?;
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(IDEAL_WINDOW)
            .with_min_inner_size([620.0, 480.0])
            .with_title(i18n::t().rcv_title)
            // See the sender: without this eframe puts egui's logo in the Dock instead of
            // the app's own icon.
            .with_icon(egui::IconData::default()),
        ..Default::default()
    };
    eframe::run_native(
        i18n::t().rcv_title,
        native,
        Box::new(move |cc| Ok(Box::new(RecvApp::new(cc, options, video)))),
    )
    .map_err(|e| fill(i18n::t().rcv_open_window_failed, &[&e]))
}

enum Stage {
    Choosing,
    Running {
        video: PathBuf,
        /// The recording's own geometry. Kept as numbers so the line can be re-rendered
        /// if the language changes while the job is still running.
        track: TrackFacts,
        threads: usize,
        job: Job,
        started: Instant,
    },
    Done(Box<Report>),
    /// M2.2: a checkpoint, not a failure — grid of what arrived, what is still
    /// missing, and the repair code for the sender.
    Partial(Box<PartialView>),
    Failed {
        /// Set when the failure happened after a recording was chosen, so the
        /// retry button knows what to retry.
        video: Option<PathBuf>,
    },
}

/// One source block's reception picture, pre-aggregated into display cells so
/// the per-frame render cost does not scale with the symbol count.
/// Cell values: 0 = nobody in this bucket arrived, 1 = some did, 2 = all did.
struct BlockView {
    sbn: u8,
    cells: Vec<u8>,
    got: usize,
    k: usize,
}

/// Everything the partial stage draws, computed once at the transition.
struct PartialView {
    partial: crate::PartialReport,
    blocks: Vec<BlockView>,
    repair_code: String,
    /// (preset name, seconds) of the fastest matching preset, for the ETA line.
    eta: Option<(String, f64)>,
}

/// How many cells a block grid tops out at; each cell then stands for a bucket
/// of `ceil(symbols / CELLS)` consecutive ESIs.
const GRID_CELLS: usize = 2048;

impl PartialView {
    fn build(partial: crate::PartialReport) -> Self {
        let mut blocks = Vec::new();
        let mut repair_code = String::new();
        let mut eta = None;
        if let Some(progress) = &partial.progress {
            for (sbn, k) in progress.source_blocks() {
                let k = k as usize;
                // Buckets cover every ESI anyone could hold, including repair
                // symbols beyond K.
                let mut max_esi = k;
                for (s, esi) in progress.symbols() {
                    if s == sbn {
                        max_esi = max_esi.max(esi as usize + 1);
                    }
                }
                let bucket = max_esi.div_ceil(GRID_CELLS).max(1);
                let cells_len = max_esi.div_ceil(bucket);
                let mut filled = vec![0usize; cells_len];
                let mut got = 0usize;
                for (s, esi) in progress.symbols() {
                    if s == sbn {
                        filled[esi as usize / bucket] += 1;
                        got += 1;
                    }
                }
                let cells = filled
                    .iter()
                    .map(|n| {
                        if *n == 0 {
                            0
                        } else if *n >= bucket {
                            2
                        } else {
                            1
                        }
                    })
                    .collect();
                blocks.push(BlockView {
                    sbn,
                    cells,
                    got,
                    k,
                });
            }
            repair_code = progress.repair_request().encode();
            // The ETA assumes the fastest preset this symbol size could have come
            // from; it is an expectation, not a promise.
            eta = sendmecongo_core::preset::ALL
                .iter()
                .filter(|p| p.symbol_size() == progress.symbol_size)
                .max_by(|a, b| a.fps.total_cmp(&b.fps))
                .map(|p| (p.name.to_string(), partial.needed as f64 / p.fps));
        }
        Self {
            partial,
            blocks,
            repair_code,
            eta,
        }
    }
}


struct RecvApp {
    options: Options,
    stage: Stage,
    /// Nearby recordings, newest first, each with its size in bytes.
    candidates: Vec<(PathBuf, u64)>,
    /// Folders worth looking in, resolved once at startup. `search_folders` walks
    /// `/Volumes` and stats a few directories — cheap, but not something to repeat on
    /// every frame just to draw the empty state.
    scanned_folders: Vec<PathBuf>,
    /// The folder scan currently in flight, if any. It runs on its own thread: reading
    /// 下载 / 桌面 / 影片 makes macOS raise a "wants to access files in …" prompt the
    /// first time, and a prompt that goes up before the window exists looks exactly
    /// like an app that opens nothing at all.
    scan: Option<mpsc::Receiver<Vec<(PathBuf, u64)>>>,
    /// Substring filter over the candidate list (name and folder), case-insensitive.
    filter: String,
    receiver: Option<mpsc::Receiver<Result<crate::ExecResult, String>>>,
    /// Set when the repair code was last copied, so the button can say so for a moment.
    copied_at: Option<Instant>,
    cjk_ok: bool,
    error_text: String,
    /// The language this window last drew itself in; a language picked from the menu bar
    /// arrives with no event, so the window compares against this once a frame.
    lang: i18n::Lang,
}

/// Look through `folders` for recordings, on a thread of its own.
///
/// The window is created and painted while this runs, which is the whole point — the
/// permission prompts a first run triggers then land on top of a live window instead
/// of on top of nothing.
fn spawn_scan(folders: Vec<PathBuf>) -> mpsc::Receiver<Vec<(PathBuf, u64)>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(scan_in(&folders));
    });
    rx
}

impl RecvApp {
    fn new(cc: &eframe::CreationContext<'_>, options: Options, video: Option<PathBuf>) -> Self {
        let cjk_ok = sendmecongo_ui::font::install(&cc.egui_ctx).is_some();
        sendmecongo_ui::font::apply_style(&cc.egui_ctx);
        // Listing the folders to look in is only directory stats, so it is safe to do
        // here; opening them is the part that can block on a permission prompt, and
        // that is what `spawn_scan` moves off this thread.
        let folders = crate::finder::search_folders();
        let mut app = Self {
            options,
            stage: Stage::Choosing,
            candidates: Vec::new(),
            scanned_folders: folders.clone(),
            scan: Some(spawn_scan(folders)),
            filter: String::new(),
            receiver: None,
            copied_at: None,
            cjk_ok,
            error_text: String::new(),
            lang: i18n::current(),
        };
        // Started with a path (`open -a … --args foo.mov`, or a drag onto the
        // binary): go straight to work instead of asking again.
        if let Some(video) = video {
            app.begin(&video, &cc.egui_ctx);
        }
        app
    }

    fn begin(&mut self, video: &PathBuf, ctx: &egui::Context) {
        let track = match prepare(video) {
            Ok(track) => track,
            Err(message) => {
                self.fail(message, Some(video.clone()));
                return;
            }
        };
        let threads = self
            .options
            .threads
            .unwrap_or_else(pipeline::default_threads)
            .max(1);
        let job = Job::new();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.error_text.clear();
        self.stage = Stage::Running {
            video: video.clone(),
            track: TrackFacts::of(&track),
            threads,
            job: job.clone(),
            started: Instant::now(),
        };

        let video = video.clone();
        let mut options = self.options.clone();
        // M2.2: a checkpoint named after this recording means "continue where the
        // last take stopped" — no buttons to find, it just resumes. Camera clip
        // names are timestamped, so a stale checkpoint from another transfer is
        // not a realistic collision.
        if options.resume.is_empty() {
            let checkpoint = crate::manifest_path_for(&video);
            if checkpoint.exists() {
                options.resume.push(checkpoint);
            }
        }
        std::thread::spawn(move || {
            let _ = tx.send(execute(&video, &track, &options, &job));
        });
        ctx.request_repaint();
    }

    fn fail(&mut self, message: String, video: Option<PathBuf>) {
        self.error_text = message;
        self.stage = Stage::Failed { video };
    }

    fn back_to_choices(&mut self) {
        self.receiver = None;
        self.rescan();
        self.stage = Stage::Choosing;
    }

    /// Look for recordings again, off the UI thread. The previous list stays on
    /// screen until the new one arrives, so the list does not blink.
    fn rescan(&mut self) {
        self.scan = Some(spawn_scan(self.scanned_folders.clone()));
    }

    /// Poll the folder-scan thread, if one is running.
    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan else {
            return;
        };
        match rx.try_recv() {
            Ok(found) => {
                self.candidates = found;
                self.scan = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            // The thread is gone without sending: treat it as "found nothing" rather
            // than leaving the window stuck on a progress line forever.
            Err(mpsc::TryRecvError::Disconnected) => {
                self.scan = None;
            }
        }
    }

    /// Poll the worker thread and the drop target. Called once per frame.
    fn pump(&mut self, ctx: &egui::Context) {
        self.poll_scan();
        if self.scan.is_some() {
            // The scan thread cannot ask for a repaint, and this window would
            // otherwise sit on "正在查找" until the user moved the mouse.
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if let Some(rx) = &self.receiver {
            match rx.try_recv() {
                Ok(Ok(crate::ExecResult::Done(report))) => {
                    self.stage = Stage::Done(Box::new(report));
                    self.receiver = None;
                }
                Ok(Ok(crate::ExecResult::Partial(partial))) => {
                    if partial.manifest_path.is_some() {
                        // M2.2: the checkpoint is on disk; show what is missing.
                        self.stage = Stage::Partial(Box::new(PartialView::build(partial)));
                    } else {
                        // No valid frame at all: a plain failure.
                        let video = match &self.stage {
                            Stage::Running { video, .. } => Some(video.clone()),
                            _ => None,
                        };
                        self.fail(partial.message(i18n::t()), video);
                    }
                    self.receiver = None;
                }
                Ok(Err(message)) => {
                    let video = match &self.stage {
                        Stage::Running { video, .. } => Some(video.clone()),
                        _ => None,
                    };
                    if message == CANCELLED {
                        self.back_to_choices();
                    } else {
                        self.fail(message, video);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Stage::Running { video, .. } = &self.stage {
                        let video = Some(video.clone());
                        self.fail(i18n::t().rcv_thread_died.to_string(), video);
                    }
                    self.receiver = None;
                }
            }
        }
        if matches!(self.stage, Stage::Running { .. }) {
            // The counters are updated by other threads; without this the window
            // would only redraw when the user moves the mouse.
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Inside the window drag-and-drop works — it is the one place it does,
        // because a drop on the Dock icon never reaches us as an argument.
        let dropped: Option<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.begin(&path, ctx);
        }
    }

    /// The bottom status bar: one dot, one line, always an honest summary.
    fn status(&self, t: &Theme) -> (String, egui::Color32) {
        let i = i18n::t();
        match &self.stage {
            Stage::Choosing => (i.rcv_status_waiting.to_string(), t.success),
            Stage::Running { job, .. } => {
                let counters: &Counters = &job.counters;
                let (unique, target) = (counters.unique(), counters.target());
                if target > 0 {
                    (fill(i.rcv_status_progress, &[&unique, &target]), t.pending)
                } else {
                    (i.rcv_status_first_frame.to_string(), t.pending)
                }
            }
            Stage::Done(_) => (i.rcv_status_done.to_string(), t.success),
            Stage::Partial(_) => (i.rcv_partial_title.to_string(), t.pending),
            Stage::Failed { .. } => (i.rcv_status_failed.to_string(), t.error),
        }
    }
}

impl eframe::App for RecvApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The language menu in the macOS menu bar; idempotent, so a cheap call per frame.
        sendmecongo_ui::menu::install(i18n::t().rcv_title);
        if self.lang != i18n::current() {
            self.lang = i18n::current();
            i18n::retitle(ctx, i18n::t().rcv_title);
            sendmecongo_ui::menu::refresh(i18n::t().rcv_title);
        }

        // Wide enough for the candidate rows to breathe, tall enough for the drop card
        // plus five rows plus the footer; any less and the list is the only thing visible.
        sendmecongo_ui::window::autosize(ctx, IDEAL_WINDOW);
        self.pump(ctx);
        let t = theme::theme(ctx);
        let (status_text, status_color) = self.status(&t);

        // macOS keeps menus in the screen's menu bar (see `sendmecongo_ui::menu`); everywhere
        // else the window draws its own row.
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
                        // The status line is rebuilt every frame, but the title belongs to
                        // the platform and has to be told the language moved.
                        i18n::retitle(ctx, i18n::t().rcv_title);
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
                    theme::status_dot(ui, status_color);
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new(status_text).size(12.0).color(
                        if status_color == t.error {
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
                egui::ScrollArea::vertical().show(ui, |ui| match &self.stage {
                    Stage::Choosing => self.ui_choosing(ui, ctx, &t),
                    Stage::Running { .. } => self.ui_running(ui, &t),
                    Stage::Done(_) => self.ui_done(ui, &t),
                    Stage::Partial(_) => self.ui_partial(ui, &t),
                    Stage::Failed { .. } => self.ui_failed(ui, &t),
                });
            });
    }
}

impl RecvApp {
    fn ui_choosing(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &Theme) {
        let mut picked: Option<PathBuf> = None;

        let i = i18n::t();
        theme::drop_zone(ui, t, 118.0, |ui| {
            upload_badge(ui, t);
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(i.rcv_drop_title)
                    .size(15.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(i.rcv_drop_sub)
                    .size(12.0)
                    .color(t.text_secondary),
            );
            ui.add_space(10.0);
            if theme::plain_button(ui, t, i.rcv_pick).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter(i.rcv_video_filter, &crate::VIDEO_EXTENSIONS)
                    .pick_file()
                {
                    picked = Some(path);
                }
            }
        });

        if self.candidates.is_empty() {
            let scanning = self.scan.is_some();
            theme::section_title(
                ui,
                t,
                if scanning {
                    i.rcv_scanning_title
                } else {
                    i.rcv_none_found_title
                },
            );
            theme::card(t).show(ui, |ui| {
                if scanning {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(i.rcv_looking_in)
                                .size(13.0)
                                .color(t.text_primary),
                        );
                    });
                    for folder in &self.scanned_folders {
                        ui.label(
                            egui::RichText::new(format!("  {}", folder.display()))
                                .size(12.0)
                                .color(t.text_secondary),
                        );
                    }
                } else {
                    ui.label(
                        egui::RichText::new(i.rcv_looked_in)
                            .size(13.0)
                            .color(t.text_primary),
                    );
                    for folder in &self.scanned_folders {
                        ui.label(
                            egui::RichText::new(format!("  {}", folder.display()))
                                .size(12.0)
                                .color(t.text_secondary),
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(i.rcv_put_together)
                            .size(12.5)
                            .color(t.text_secondary),
                    );
                }
            });
            ui.add_space(8.0);
            let label = if scanning {
                i.rcv_scanning
            } else {
                i.rcv_rescan
            };
            if theme::text_button(ui, t, label, None).clicked() && !scanning {
                self.rescan();
            }
        } else {
            theme::section_title(ui, t, fill(i.rcv_list_title, &[&self.candidates.len()]));

            let filter = self.filter.trim().to_lowercase();
            let filtered: Vec<(PathBuf, u64)> = self
                .candidates
                .iter()
                .filter(|(path, _)| {
                    filter.is_empty() || path.display().to_string().to_lowercase().contains(&filter)
                })
                .cloned()
                .collect();

            theme::card_edge(t).show(ui, |ui| {
                if self.candidates.len() > FILTER_THRESHOLD {
                    egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("🔍").size(12.0));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.filter)
                                        .hint_text(
                                            egui::RichText::new(i.rcv_filter_hint)
                                                .size(12.5)
                                                .color(t.text_tertiary),
                                        )
                                        .desired_width(f32::INFINITY)
                                        .frame(false),
                                );
                            });
                        });
                    theme::hairline(ui, t);
                }

                if filtered.is_empty() {
                    egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(14, 16))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(i.rcv_no_match)
                                    .size(12.5)
                                    .color(t.text_tertiary),
                            );
                        });
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(VISIBLE_ROWS * ROW_HEIGHT)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let last = filtered.len() - 1;
                            for (i, (path, bytes)) in filtered.iter().enumerate() {
                                if candidate_row(ui, t, path, *bytes) {
                                    picked = Some(path.clone());
                                }
                                if i < last {
                                    theme::hairline(ui, t);
                                }
                            }
                        });
                    if filter.is_empty() && filtered.len() > VISIBLE_ROWS as usize {
                        theme::hairline(ui, t);
                        egui::Frame::default()
                            .inner_margin(egui::Margin::symmetric(14, 7))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(fill(
                                        i.rcv_count_scroll,
                                        &[&filtered.len()],
                                    ))
                                    .size(11.5)
                                    .color(t.text_tertiary),
                                );
                            });
                    }
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(i.rcv_search_note)
                        .size(11.5)
                        .color(t.text_tertiary),
                );
            });
            ui.add_space(2.0);
            let label = if self.scan.is_some() {
                i.rcv_scanning
            } else {
                i.rcv_rescan
            };
            if theme::text_button(ui, t, label, None).clicked() && self.scan.is_none() {
                self.rescan();
            }
        }

        if !self.cjk_ok {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(i.cjk_font_missing)
                    .size(12.0)
                    .color(t.warn_sub),
            );
        }

        if let Some(path) = picked {
            self.begin(&path, ctx);
        }
    }

    fn ui_running(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let Stage::Running {
            video,
            track,
            threads,
            job,
            started,
        } = &self.stage
        else {
            return;
        };
        let counters: &Counters = &job.counters;
        let elapsed = started.elapsed().as_secs_f64();
        let (fraction, unique, target) =
            (counters.fraction(), counters.unique(), counters.target());
        let (pictures, total, codes) = (counters.pictures(), counters.total(), counters.codes());
        let name = file_name(video);
        let summary = track.summary(i18n::t(), *threads);

        theme::card(t).show(ui, |ui| {
            ui.horizontal(|ui| {
                video_badge(ui, t, 40.0);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(name)
                            .size(14.0)
                            .strong()
                            .color(t.text_primary),
                    );
                    ui.label(
                        egui::RichText::new(summary)
                            .size(12.0)
                            .color(t.text_secondary),
                    );
                });
            });
        });

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(i18n::t().rcv_running)
                    .size(13.5)
                    .strong()
                    .color(t.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{:.0}%", fraction * 100.0))
                        .size(20.0)
                        .strong()
                        .color(t.text_primary),
                );
            });
        });
        ui.add_space(-2.0);
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_height(8.0)
                .corner_radius(4.0),
        );

        ui.add_space(14.0);
        let i = i18n::t();
        let (symbols_n, symbols_s) = if target > 0 {
            (format!("{unique}"), format!("/ {target}"))
        } else {
            (i.rcv_reading.to_string(), String::new())
        };
        ui.columns(4, |cols| {
            theme::stat_card(&mut cols[0], t, &symbols_n, &symbols_s, i.rcv_stat_symbols);
            theme::stat_card(
                &mut cols[1],
                t,
                &format!("{pictures}"),
                &format!("/ {total}"),
                i.rcv_stat_frames,
            );
            theme::stat_card(&mut cols[2], t, &format!("{codes}"), "", i.rcv_stat_codes);
            theme::stat_card(
                &mut cols[3],
                t,
                &human_duration(i, elapsed),
                "",
                i.rcv_stat_elapsed,
            );
        });

        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(i.rcv_stall_note)
                .size(12.0)
                .color(t.text_secondary),
        );

        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::plain_button(ui, t, i18n::t().stop).clicked() {
                job.stop();
            }
        });
    }

    fn ui_done(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let Stage::Done(report) = &self.stage else {
            return;
        };
        let report: Report = (**report).clone();
        let i = i18n::t();
        let lines = report.lines(i);

        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            theme::badge(ui, t.success, "✓");
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i.rcv_done_title)
                    .size(17.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(file_name(&report.target))
                    .size(14.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.label(
                egui::RichText::new(report.target.display().to_string())
                    .size(11.0)
                    .monospace()
                    .color(t.text_tertiary),
            );
        });
        ui.add_space(14.0);

        // A comparison that was never asked for is not a failure, so the row stays in
        // the normal text colour rather than going green or red.
        let verify = match &lines.verify {
            Some(line) => line.clone(),
            None => i.rcv_verify_default.to_string(),
        };
        theme::card_edge(t).show(ui, |ui| {
            theme::kv_row(
                ui,
                t,
                i.label_size,
                egui::RichText::new(human_bytes(report.bytes)).color(t.text_primary),
            );
            theme::hairline(ui, t);
            theme::kv_row(
                ui,
                t,
                i.label_verify,
                egui::RichText::new(verify).color(if lines.identical {
                    t.success
                } else {
                    t.text_primary
                }),
            );
            theme::hairline(ui, t);
            theme::kv_row(
                ui,
                t,
                i.label_video,
                egui::RichText::new(lines.track_summary.clone()).color(t.text_primary),
            );
            theme::hairline(ui, t);
            theme::kv_row(
                ui,
                t,
                i.label_stats,
                egui::RichText::new(lines.counters.clone()).color(t.text_primary),
            );
            theme::hairline(ui, t);
            theme::kv_row(
                ui,
                t,
                i.label_symbols,
                egui::RichText::new(lines.symbols.clone()).color(t.text_primary),
            );
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(&lines.timing)
                    .size(11.5)
                    .color(t.text_tertiary),
            );
        });

        for warning in &lines.warnings {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(warning).size(12.5).color(t.warn_sub));
        }

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if theme::primary_button(ui, t, i.rcv_reveal, false, true).clicked() {
                crate::finder::reveal(&report.target);
            }
            if theme::plain_button(ui, t, i.rcv_again).clicked() {
                self.back_to_choices();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::text_button(ui, t, i.quit, Some(t.text_secondary)).clicked() {
                    std::process::exit(0);
                }
            });
        });
    }

    /// M2.2: a checkpoint screen — what arrived, what is missing, how long the
    /// retake is, and the repair code that makes the retake short.
    fn ui_partial(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let Stage::Partial(view) = &self.stage else {
            return;
        };
        let i = i18n::t();
        let received = view.partial.received;
        let source = view.partial.source.max(1);
        let needed = view.partial.needed;
        let fraction = (received as f32 / source as f32).clamp(0.0, 1.0);

        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            theme::badge(ui, t.pending, "!");
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i.rcv_partial_title)
                    .size(17.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(file_name(&view.partial.video))
                    .size(13.0)
                    .color(t.text_secondary),
            );
        });
        ui.add_space(12.0);

        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_height(8.0)
                .corner_radius(4.0),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(fill(i.rcv_partial_needed, &[&received, &source, &needed]))
                .size(14.0)
                .strong()
                .color(t.text_primary),
        );
        if let Some((preset, secs)) = &view.eta {
            ui.label(
                egui::RichText::new(fill(
                    i.rcv_partial_eta,
                    &[preset, &human_duration(i, *secs)],
                ))
                .size(12.5)
                .color(t.text_secondary),
            );
        }
        if let Some((symbols, _)) = view.partial.resumed {
            ui.label(
                egui::RichText::new(fill(i.rcv_auto_resume, &[&symbols]))
                    .size(12.0)
                    .color(t.text_secondary),
            );
        }
        if let Some(path) = &view.partial.manifest_path {
            ui.label(
                egui::RichText::new(path.display().to_string())
                    .size(11.0)
                    .monospace()
                    .color(t.text_tertiary),
            );
        }

        // The repair code: what the operator carries back to the sender.
        ui.add_space(14.0);
        theme::card_edge(t).show(ui, |ui| {
            ui.label(
                egui::RichText::new(i.rcv_repair_code)
                    .size(13.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&view.repair_code)
                        .size(15.0)
                        .monospace()
                        .color(t.text_primary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let copied = self
                        .copied_at
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(2));
                    let label = if copied { i.rcv_copied } else { i.rcv_copy };
                    if theme::plain_button(ui, t, label).clicked() {
                        ui.ctx().copy_text(view.repair_code.clone());
                        self.copied_at = Some(Instant::now());
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(i.rcv_repair_code_hint)
                    .size(11.5)
                    .color(t.text_secondary),
            );
        });

        // The grid: one card per source block (usually exactly one).
        ui.add_space(14.0);
        theme::card_edge(t).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(i.rcv_grid_title)
                        .size(13.0)
                        .strong()
                        .color(t.text_primary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(i.rcv_grid_legend)
                            .size(11.0)
                            .color(t.text_tertiary),
                    );
                });
            });
            ui.add_space(6.0);
            for block in &view.blocks {
                if view.blocks.len() > 1 {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {}/{}",
                            fill(i.rcv_block_label, &[&block.sbn]),
                            block.got,
                            block.k
                        ))
                        .size(11.5)
                        .color(t.text_secondary),
                    );
                    ui.add_space(2.0);
                }
                symbol_grid(ui, t, &block.cells);
                ui.add_space(4.0);
            }
        });

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if theme::plain_button(ui, t, i.rcv_back).clicked() {
                self.back_to_choices();
            }
        });
    }

    fn ui_failed(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let Stage::Failed { video } = &self.stage else {
            return;
        };
        let video = video.clone();
        let i = i18n::t();

        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            theme::badge(ui, t.error, "!");
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i.rcv_failed_title)
                    .size(17.0)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(human_advice(&self.error_text))
                    .size(13.0)
                    .color(t.text_secondary),
            );
        });
        ui.add_space(12.0);

        egui::Frame::default()
            .fill(t.code_bg)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&self.error_text)
                                .size(11.5)
                                .monospace()
                                .color(t.code_fg),
                        );
                    });
            });

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if let Some(path) = video {
                if theme::primary_button(ui, t, i.rcv_retry, false, true).clicked() {
                    let ctx = ui.ctx().clone();
                    self.begin(&path, &ctx);
                    return;
                }
            }
            if theme::plain_button(ui, t, i.rcv_other_video).clicked() {
                self.back_to_choices();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::text_button(ui, t, i.quit, Some(t.text_secondary)).clicked() {
                    std::process::exit(0);
                }
            });
        });
    }
}

/// The one-sentence advice above the technical details. Symbol shortfalls get the
/// "record longer" advice; everything else (unreadable file, unknown codec) gets a
/// pointer to the details below — the two situations have completely different remedies.
///
/// The routing looks for the word for "symbol" in all three languages: the error text was
/// written by the worker in whatever language was current when it failed, which is not
/// necessarily the one the window is drawing in now.
fn human_advice(error: &str) -> &'static str {
    let lower = error.to_lowercase();
    let shortfall = ["symbol", "符号", "符號"]
        .iter()
        .any(|marker| lower.contains(marker));
    let t = i18n::t();
    if shortfall {
        t.rcv_advice_shortfall
    } else {
        t.rcv_advice_other
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// One row in the candidates list: icon, name, folder · size, and the action.
/// Returns true when the row's "还原 ›" was clicked.
fn candidate_row(ui: &mut egui::Ui, t: &Theme, path: &PathBuf, bytes: u64) -> bool {
    let mut chosen = false;
    egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(14, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                video_badge(ui, t, 32.0);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(file_name(path))
                            .size(13.5)
                            .color(t.text_primary),
                    );
                    let folder = path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(
                        egui::RichText::new(format!("{folder} · {}", human_bytes(bytes as usize)))
                            .size(11.0)
                            .color(t.text_tertiary),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::text_button(ui, t, i18n::t().rcv_row_action, None).clicked() {
                        chosen = true;
                    }
                });
            });
        });
    chosen
}

/// The grey "▶" tile used for recordings.
fn video_badge(ui: &mut egui::Ui, t: &Theme, side: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    ui.painter().rect_filled(rect, side / 4.5, t.control_bg);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "▶",
        egui::FontId::proportional(side / 2.6),
        t.text_secondary,
    );
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

/// Videos lying about, newest first. Shares `finder::search_folders` with the
/// command line so both modes agree on what "nearby" means. Each entry keeps its
/// size so the list can show it without re-statting every frame.
fn scan_in(folders: &[PathBuf]) -> Vec<(PathBuf, u64)> {
    let mut found: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in folders {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !crate::VIDEO_EXTENSIONS.contains(&extension) || !seen.insert(path.clone()) {
                continue;
            }
            let (modified, len) = entry
                .metadata()
                .map(|m| {
                    (
                        m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                        m.len(),
                    )
                })
                .unwrap_or((std::time::SystemTime::UNIX_EPOCH, 0));
            found.push((path, len, modified));
        }
    }
    found.sort_by_key(|(_, _, modified)| std::cmp::Reverse(*modified));
    found
        .into_iter()
        .map(|(path, len, _)| (path, len))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The advice must route symbol shortfalls to "record longer" and leave codec /
    /// filesystem failures pointing at the technical details — the two situations
    /// have completely different remedies.
    #[test]
    fn advice_matches_the_failure_kind() {
        assert!(human_advice("collector: 1229/1231 unique symbols").contains("符号"));
        assert!(human_advice("只收到 1229 个符号").contains("符号"));
        assert!(human_advice("unsupported codec: av01").contains("细节"));
        assert!(human_advice("无法读取文件").contains("细节"));
    }
}

/// The per-block reception grid (M2.2). Each cell is a bucket of consecutive
/// ESIs: solid green when the whole bucket arrived, amber when some did, an
/// empty outline when none did. Painted, not widget-per-cell, so a 64 MB
/// transfer's tens of thousands of symbols cost one draw list.
fn symbol_grid(ui: &mut egui::Ui, t: &Theme, cells: &[u8]) {
    const CELL: f32 = 8.0;
    const GAP: f32 = 2.0;
    let width = ui.available_width();
    let cols = ((width + GAP) / (CELL + GAP)).floor().max(1.0) as usize;
    let rows = cells.len().div_ceil(cols);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, rows as f32 * (CELL + GAP)),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    for (index, cell) in cells.iter().enumerate() {
        let col = index % cols;
        let row = index / cols;
        let origin = rect.min + egui::vec2(col as f32 * (CELL + GAP), row as f32 * (CELL + GAP));
        let cell_rect = egui::Rect::from_min_size(origin, egui::vec2(CELL, CELL));
        match cell {
            2 => painter.rect_filled(cell_rect, 1.5, t.success),
            1 => painter.rect_filled(cell_rect, 1.5, t.pending),
            _ => painter.rect_stroke(
                cell_rect,
                1.5,
                egui::Stroke::new(1.0_f32, t.hairline),
                egui::StrokeKind::Inside,
            ),
        };
    }
}
