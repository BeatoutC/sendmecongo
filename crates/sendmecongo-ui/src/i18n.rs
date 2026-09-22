//! User-interface languages.
//!
//! Both apps used to spell every label out in Simplified Chinese, which meant the one
//! thing an operator could never change was the language they read. This module is the
//! single place a string is allowed to live.
//!
//! Three properties matter more than the translations themselves:
//!
//! * **A missing translation is a compile error.** [`Text`] is a struct of `&'static str`,
//!   and every language fills in every field. Add a key and the build breaks until all
//!   the languages have it — which is the only reliable way to stop a screen from coming
//!   out half-translated three releases later.
//! * **Adding a language touches one file.** [`Lang::ALL`], the enum, and one new
//!   `impl`-style table in `i18n/<code>.rs`. Nothing at a call site changes: everything
//!   reads through [`t()`].
//! * **The choice is the user's.** `--lang` beats `SENDMECONGO_LANG` beats a language the
//!   user picked in the window beats the system locale beats Simplified Chinese, which
//!   is what the app shipped with and therefore the only safe default.
//!
//! Strings that interpolate numbers are templates full of `{}` and go through [`fill`],
//! so a translation can move the number wherever its grammar wants it.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

pub mod en;
pub mod zh_hans;
pub mod zh_hant;

/// A language the interface can be drawn in.
///
/// The codes are BCP 47 tags, which is also what `--lang` accepts (`zh-Hant`, `en-GB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// Simplified Chinese — the language the UI was written in, and the default.
    ZhHans,
    /// Traditional Chinese (Taiwan / Hong Kong / Macao).
    ZhHant,
    /// English.
    En,
}

impl Lang {
    /// Every language, in the order the picker lists them.
    pub const ALL: [Lang; 3] = [Lang::ZhHans, Lang::ZhHant, Lang::En];

    /// What a launch with nothing to go on gets.
    pub const DEFAULT: Lang = Lang::ZhHans;

    /// The tag written to the preference file and printed by `--help`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::ZhHans => "zh-Hans",
            Lang::ZhHant => "zh-Hant",
            Lang::En => "en",
        }
    }

    /// The language's name *in that language* — a picker that says "Chinese" is useless
    /// to the person who cannot read "Chinese".
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::ZhHans => "简体中文",
            Lang::ZhHant => "繁體中文",
            Lang::En => "English",
        }
    }

    /// The table of strings for this language.
    pub fn table(self) -> &'static Text {
        match self {
            Lang::ZhHans => &zh_hans::TEXT,
            Lang::ZhHant => &zh_hant::TEXT,
            Lang::En => &en::TEXT,
        }
    }

    fn index(self) -> u8 {
        match self {
            Lang::ZhHans => 0,
            Lang::ZhHant => 1,
            Lang::En => 2,
        }
    }

    fn from_index(value: u8) -> Lang {
        match value {
            1 => Lang::ZhHant,
            2 => Lang::En,
            _ => Lang::ZhHans,
        }
    }

    /// Read a locale tag: `zh-Hant-TW`, `zh_TW`, `en_US.UTF-8`, `en-GB`.
    ///
    /// Returns `None` for anything unrecognised, so the caller can keep looking rather
    /// than silently falling back to the default.
    ///
    /// Chinese is decided by script first (`Hans`/`Hant`), then by region: Taiwan, Hong
    /// Kong and Macao are Traditional, everything else is Simplified. That is the same
    /// rule the C libraries use, and it is why `zh-TW` needs no translating table of its
    /// own.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let tag = tag
            .trim()
            .trim_end_matches(".UTF-8")
            .trim_end_matches(".utf8");
        if tag.is_empty() {
            return None;
        }
        let lower = tag.to_ascii_lowercase().replace('_', "-");
        let parts: Vec<&str> = lower.split('-').filter(|p| !p.is_empty()).collect();
        let primary = *parts.first()?;
        match primary {
            "zh" => {
                for part in &parts[1..] {
                    match *part {
                        "hant" => return Some(Lang::ZhHant),
                        "hans" => return Some(Lang::ZhHans),
                        _ => {}
                    }
                }
                let traditional_region =
                    parts[1..].iter().any(|p| matches!(*p, "tw" | "hk" | "mo"));
                Some(if traditional_region {
                    Lang::ZhHant
                } else {
                    Lang::ZhHans
                })
            }
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

/// A language chosen in the window is remembered across runs.
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language everything should be drawn in right now.
pub fn current() -> Lang {
    Lang::from_index(CURRENT.load(Ordering::Relaxed))
}

/// The string table for [`current`]. Every call site reads this and nothing else.
pub fn t() -> &'static Text {
    current().table()
}

/// Switch language and remember the choice for next time.
pub fn set_lang(lang: Lang) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
    save(lang);
}

/// Switch language for this run only, without touching the preference file.
pub fn set_lang_ephemeral(lang: Lang) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
}

/// Decide the language, once, at startup.
///
/// `explicit` is `--lang <tag>`; `remember` says whether a previous choice made in the
/// window should outrank the environment. It does for the GUI — the user picked it on
/// purpose — and it does not for a command-line run, where the locale of the shell that
/// invoked the tool is the interesting signal.
///
/// Unknown tags are ignored rather than fatal: a typo in `--lang` should not stop a file
/// transfer that someone is standing in front of a screen waiting for.
pub fn init(explicit: Option<&str>, remember: bool) {
    let lang = explicit
        .and_then(Lang::from_tag)
        .or_else(|| {
            std::env::var("SENDMECONGO_LANG")
                .ok()
                .and_then(|v| Lang::from_tag(&v))
        })
        .or_else(|| remember.then(load).flatten())
        .or_else(from_environment)
        // Last resort before the default: an app launched by double-clicking inherits no
        // shell environment at all, so on macOS the system preference is the only signal
        // that exists. Skipped on the command line, where it would be pure latency.
        .or_else(|| remember.then(system_locale).flatten())
        .unwrap_or(Lang::DEFAULT);
    set_lang_ephemeral(lang);
}

/// The usual locale variables, in the order libc consults them.
fn from_environment() -> Option<Lang> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        // `C` and `POSIX` mean "no locale", i.e. nothing to learn here.
        if value.is_empty() || value == "C" || value == "POSIX" {
            continue;
        }
        // `LANGUAGE` may hold a colon-separated preference list.
        for candidate in value.split(':') {
            if let Some(lang) = Lang::from_tag(candidate) {
                return Some(lang);
            }
        }
    }
    None
}

/// macOS: the language the user set in System Settings.
///
/// `AppleLocale` reads "zh_CN" or "en_US" and needs no plist parsing. A process started
/// by Finder has no `LANG`, so without this a double-clicked `.app` could never follow
/// the system language.
#[cfg(target_os = "macos")]
fn system_locale() -> Option<Lang> {
    let output = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Lang::from_tag(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
fn system_locale() -> Option<Lang> {
    None
}

/// `<config>/sendmecongo/language`, the usual per-user location on each platform.
fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/sendmecongo"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("sendmecongo"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(base.join("sendmecongo"))
    }
}

fn preference_file() -> Option<PathBuf> {
    Some(config_dir()?.join("language"))
}

/// Best effort, always: a read-only home directory must not stop an app from starting.
fn load() -> Option<Lang> {
    load_from(&preference_file()?)
}

fn save(lang: Lang) {
    if let Some(path) = preference_file() {
        save_to(&path, lang);
    }
}

/// The preference file's contents, split out from where it lives so the round trip can be
/// tested without writing into the real one.
fn load_from(path: &Path) -> Option<Lang> {
    let text = std::fs::read_to_string(path).ok()?;
    Lang::from_tag(text.trim())
}

/// Never fails loudly: this runs at the moment the user picked a language, and a language
/// that cannot be remembered is worth exactly one line of nothing.
fn save_to(path: &Path, lang: Lang) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{}\n", lang.code()));
}

/// Retitle the window in the language just chosen.
///
/// The title belongs to the platform, so a language change has to be pushed to it: every
/// other string is read from the table at draw time and follows along by itself.
pub fn retitle(ctx: &egui::Context, title: &str) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_string()));
}

/// Substitute the arguments into a template's `{}` placeholders, left to right.
///
/// Deliberately dumber than `format!`: the template is data, and a translation has to be
/// free to put the number at the front, at the back, or inside a different sentence.
/// Extra arguments are dropped and a placeholder with no argument is left visible, which
/// looks broken on screen in exactly the case that deserves to be noticed.
pub fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 24);
    let mut rest = template;
    let mut next = 0;
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        match args.get(next) {
            Some(arg) => {
                out.push_str(&arg.to_string());
                next += 1;
            }
            None => out.push_str("{}"),
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// Pad a label to the width the command-line report lines up its columns at.
///
/// Terminal columns, not chars: a CJK glyph is two of them. Not a general-purpose wcwidth
/// — it only has to be right for the labels in the tables below, in every language.
pub fn pad_label(label: &str) -> String {
    const COLUMNS: usize = 10;
    let width: usize = label.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum();
    format!("{label}{}", " ".repeat(COLUMNS.saturating_sub(width)))
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// The language menu, drawn in the menu bar at the top of both windows.
///
/// A menu rather than a combo box in the status bar. The status bar is the bottom-most
/// strip of the window, so a dropdown opening from there has nowhere to go: the popup
/// lands on or past the window edge and a click that appears to do nothing is exactly
/// what someone reports as "there is no way to change the language". A menu opens into
/// the window, and a menu is where anyone looks for this in the first place.
///
/// Returns true when the language changed, so the caller can refresh whatever it has
/// already drawn — the window title is owned by the platform, and any text captured as a
/// string (a status line, a finished report) has to be asked for again.
pub fn language_menu(ui: &mut egui::Ui) -> bool {
    let mut selected = current();
    let mut changed = false;
    ui.menu_button(t().language, |ui| {
        for lang in Lang::ALL {
            // A radio, not a plain item: which language is live right now is the first
            // thing anyone opening this menu wants to know.
            if ui
                .radio_value(&mut selected, lang, lang.native_name())
                .clicked()
            {
                changed = true;
                ui.close();
            }
        }
    });
    if changed {
        set_lang(selected);
    }
    changed
}

/// Every string the interface can show.
///
/// One flat struct on purpose: a nested tree would make the call sites shorter and the
/// translation tables easier to get wrong. The sections below are the same order as the
/// three language files, so the three columns of a key sit at the same line number in
/// each file and a reviewer can diff them against each other.
pub struct Text {
    // ── Shared ──────────────────────────────────────────────────────────────────
    /// Name of the picker, for tooltips and the `--help` text.
    pub language: &'static str,
    /// Shown when no CJK-capable system font was found.
    pub cjk_font_missing: &'static str,
    /// One line printed by `launch::note_unbundled`; `{}` is the app name.
    pub note_unbundled: &'static str,
    pub stop: &'static str,
    pub quit: &'static str,
    pub cancel: &'static str,
    /// The file card's "remove this file" action.
    pub clear: &'static str,
    /// The file card's "pick a different file" action.
    pub change: &'static str,
    /// `退出 {}` — the macOS application menu's Quit item, named after the app.
    pub quit_app: &'static str,
    /// `{} 秒` — under a minute, one decimal.
    pub seconds_short: &'static str,
    /// `{} 分钟` — a whole number of minutes.
    pub minutes_only: &'static str,
    /// `{} 分 {} 秒` — whole minutes plus seconds.
    pub minutes_seconds: &'static str,

    // ── Sender window ───────────────────────────────────────────────────────────
    pub snd_title: &'static str,
    pub snd_tab_send: &'static str,
    pub snd_tab_receive: &'static str,
    pub snd_intro: &'static str,
    pub snd_drop_title: &'static str,
    pub snd_drop_sub: &'static str,
    pub snd_pick_file: &'static str,
    pub snd_pick_dialog_title: &'static str,
    pub snd_status_ready: &'static str,
    pub snd_status_preparing: &'static str,
    pub snd_status_encode_done: &'static str,
    pub snd_status_encode_done_skipped: &'static str,
    pub snd_status_encode_cancelled: &'static str,
    pub snd_status_no_file: &'static str,
    /// `播放中 · {}×{}px · {} 路 · {} sym/s` (size, per-lane size, lanes, fps).
    pub snd_status_playing: &'static str,
    /// `正在取消 {}%`.
    pub snd_status_cancelling: &'static str,
    /// `{} {}%` — phase, percent.
    pub snd_status_phase: &'static str,
    pub snd_status_stopped: &'static str,
    pub snd_status_playback_ended: &'static str,
    /// `播放器异常退出（{}）` — exit status.
    pub snd_status_player_failed: &'static str,
    /// `播放器异常退出（{}）：{}` — exit status, last stderr line.
    pub snd_status_player_failed_log: &'static str,
    /// `读取播放器状态失败：{}`.
    pub snd_status_child_error: &'static str,
    /// `启动播放器失败：{}`.
    pub snd_error_spawn_player: &'static str,
    /// `找不到自身可执行文件：{}`.
    pub snd_error_self_exe: &'static str,
    pub snd_hint_title: &'static str,
    pub snd_hint_no_file: &'static str,
    pub snd_hint_not_ready: &'static str,
    /// `当前：{} {}%` — phase, percent.
    pub snd_hint_current: &'static str,
    pub snd_hint_cancelling: &'static str,
    pub snd_hint_encode_failed: &'static str,
    pub snd_got_it: &'static str,
    /// The hint dialog's "stop the encoding" button.
    pub snd_cancel_encoding: &'static str,
    pub snd_encoding: &'static str,
    /// `{} → {} · {} 个符号 · {} · 编码用时 {} 秒`.
    pub snd_file_summary: &'static str,
    pub snd_cancelling_soon: &'static str,
    pub snd_progress_note: &'static str,
    /// `正在播放 · 建议录像 ≥ {}`.
    pub snd_callout_playing: &'static str,
    /// `建议录像 ≥ {}`.
    pub snd_callout_idle: &'static str,
    pub snd_callout_playing_sub: &'static str,
    /// `理想情况 {} 即可收齐；…` — the ideal duration.
    pub snd_callout_idle_sub: &'static str,
    pub snd_settings_title: &'static str,
    pub snd_set_preset: &'static str,
    /// `{} · {} sym/s · 标称 {} KB/s` (name, symbols/s, KB/s).
    pub snd_preset_selected: &'static str,
    /// `{} · {} sym/s · {} KB/s{}` — same, plus the dual-lane tag.
    pub snd_preset_option: &'static str,
    pub snd_preset_dual: &'static str,
    pub snd_set_lanes: &'static str,
    /// `默认 ({})` — the preset's own lane count.
    pub snd_lanes_auto: &'static str,
    pub snd_set_display: &'static str,
    pub snd_fit_display: &'static str,
    pub snd_display_main_tag: &'static str,
    /// `显示器 {} · {}×{}` — index, width, height.
    pub snd_display_label: &'static str,
    pub snd_display_fallback: &'static str,
    pub snd_window_width: &'static str,
    pub snd_set_window: &'static str,
    pub snd_borderless: &'static str,
    /// `将铺满 {}×{} @ ({}, {})` — width, height, x, y.
    pub snd_will_fill: &'static str,
    pub snd_set_cycles: &'static str,
    pub snd_cycles_endless: &'static str,
    pub snd_play: &'static str,
    pub snd_stop_play: &'static str,
    pub snd_encoding_hint: &'static str,
    pub snd_encode_failed_hint: &'static str,
    pub snd_pick_first_hint: &'static str,
    pub snd_reencode: &'static str,
    pub snd_esc_stops: &'static str,
    pub snd_tips_title: &'static str,
    pub snd_tips_chip: &'static str,
    pub snd_tip_dual_4k60: &'static str,
    pub snd_tip_dual_30fps: &'static str,
    pub snd_tip_single_4k30: &'static str,
    pub snd_tip_lock_focus: &'static str,
    pub snd_tip_tripod: &'static str,
    pub snd_tip_dnd: &'static str,
    pub snd_tip_format: &'static str,
    pub snd_rcv_intro: &'static str,
    pub snd_rcv_step1_title: &'static str,
    pub snd_rcv_step1_body: &'static str,
    pub snd_rcv_step1_dual: &'static str,
    pub snd_rcv_step1_single: &'static str,
    pub snd_rcv_step1_codec: &'static str,
    pub snd_rcv_step2_title: &'static str,
    pub snd_rcv_step2_movie: &'static str,
    pub snd_rcv_step2_tool: &'static str,
    pub snd_rcv_step2_note: &'static str,
    pub snd_rcv_step3_title: &'static str,
    pub snd_rcv_step3_body: &'static str,
    pub snd_rcv_step3_warn: &'static str,
    pub snd_rcv_step3_cli_note: &'static str,
    pub snd_rcv_step3_cli: &'static str,
    pub snd_rcv_step3_tail: &'static str,
    pub snd_rcv_step4_title: &'static str,
    pub snd_rcv_step4_body: &'static str,
    pub snd_rcv_step4_note: &'static str,
    pub snd_rcv_callout: &'static str,

    // ── Sender: the prepare worker ──────────────────────────────────────────────
    pub prep_phase_read: &'static str,
    pub prep_phase_gzip: &'static str,
    pub prep_phase_brotli: &'static str,
    pub prep_phase_package: &'static str,
    pub prep_method_raw: &'static str,
    /// `无法启动编码线程：{}`.
    pub prep_thread_failed: &'static str,
    pub prep_thread_died: &'static str,
    /// `读不到文件：{}`.
    pub prep_meta_failed: &'static str,
    /// `读文件失败：{}`.
    pub prep_read_failed: &'static str,
    /// `压缩失败：{}`.
    pub prep_compress_failed: &'static str,
    /// `{} 有 {}，超过 64 MB 上限。…` — file name, human size.
    pub prep_too_large: &'static str,
    /// `已跳过 brotli：数据压不动（{} → {}），再压只会更慢。`
    pub prep_skip_incompressible: &'static str,
    /// `已跳过 brotli：预计只多省 {}（约 {}s 录像时间），但要花 {}s 压缩。`
    pub prep_skip_not_worth: &'static str,

    // ── Receiver window ────────────────────────────────────────────────────────
    pub rcv_title: &'static str,
    pub rcv_alert_title: &'static str,
    /// `打不开窗口：{}…`
    pub rcv_open_window_failed: &'static str,
    pub rcv_drop_title: &'static str,
    pub rcv_drop_sub: &'static str,
    pub rcv_pick: &'static str,
    pub rcv_video_filter: &'static str,
    pub rcv_scanning_title: &'static str,
    pub rcv_none_found_title: &'static str,
    pub rcv_looking_in: &'static str,
    pub rcv_looked_in: &'static str,
    pub rcv_put_together: &'static str,
    pub rcv_scanning: &'static str,
    pub rcv_rescan: &'static str,
    /// `附近的录像（共 {} 个，按时间从新到旧）`.
    pub rcv_list_title: &'static str,
    pub rcv_filter_hint: &'static str,
    pub rcv_no_match: &'static str,
    /// `共 {} 个 · 滚动查看全部`.
    pub rcv_count_scroll: &'static str,
    pub rcv_search_note: &'static str,
    pub rcv_row_action: &'static str,
    pub rcv_running: &'static str,
    pub rcv_stat_symbols: &'static str,
    pub rcv_stat_frames: &'static str,
    pub rcv_stat_codes: &'static str,
    pub rcv_stat_elapsed: &'static str,
    /// Shown in the symbol counter before the first header has been read.
    pub rcv_reading: &'static str,
    pub rcv_stall_note: &'static str,
    pub rcv_done_title: &'static str,
    pub rcv_verify_default: &'static str,
    pub rcv_reveal: &'static str,
    pub rcv_again: &'static str,
    pub rcv_failed_title: &'static str,
    pub rcv_advice_shortfall: &'static str,
    pub rcv_advice_other: &'static str,
    pub rcv_retry: &'static str,
    pub rcv_other_video: &'static str,
    pub rcv_status_waiting: &'static str,
    /// `已收 {} / {} 个符号` — unique, target.
    pub rcv_status_progress: &'static str,
    pub rcv_status_first_frame: &'static str,
    pub rcv_status_done: &'static str,
    pub rcv_status_failed: &'static str,
    pub rcv_thread_died: &'static str,
    /// `{} · {}×{} · {} · {} 帧 · {} fps · {} 线程`.
    pub rcv_summary: &'static str,
    /// `{} 帧解码 · {} 个码识别 · {} 个不同符号（收到 {} 个，重复 {} 个）`.
    pub rcv_counters_line: &'static str,
    /// `符号载荷 {} B · 容器 {} B · 源符号 K≈{}`.
    pub rcv_symbols_line: &'static str,
    /// `{} s · 吞吐 {}`.
    pub rcv_timing_line: &'static str,
    /// `IDENTICAL ✓  与 {} 逐字节一致` — the leading token is matched by the JSON
    /// writer and by `tools/verify-recv.sh`, so it stays in every language.
    pub rcv_verify_identical: &'static str,
    /// `MISMATCH ✗  与 {} 不同`.
    pub rcv_verify_mismatch: &'static str,
    /// `还原结果与 {} 不一致（长度 {} vs {}）——请把这段录像留好`.
    pub rcv_verify_mismatch_warn: &'static str,
    /// `打开录像失败：{}`.
    pub rcv_open_failed: &'static str,
    pub rcv_no_frames: &'static str,
    /// `创建输出目录失败：{}`.
    pub rcv_mkdir_failed: &'static str,
    /// `写入 {} 失败：{}` — path, error.
    pub rcv_write_failed: &'static str,
    /// `读取 {} 失败：{}` — path, error.
    pub rcv_read_failed: &'static str,
    /// `写入符号 dump 失败：{}`.
    pub rcv_dump_failed: &'static str,
    /// `写入 JSON 失败：{}`.
    pub rcv_json_failed: &'static str,
    /// `已载入续传进度：{} 个不同符号（来自 {} 段录像）` — symbols, recordings.
    pub rcv_resume_loaded: &'static str,
    /// `无法读取进度清单 {}：{}` — path, error.
    pub rcv_resume_failed: &'static str,
    /// `续传进度已保存：{}`.
    pub rcv_manifest_saved: &'static str,
    /// `补拍后运行：sendmecongo-recv 新录像.mov --resume {}`.
    pub rcv_partial_hint: &'static str,

    // ── Receiver: the command line ─────────────────────────────────────────────
    pub cli_banner: &'static str,
    pub cli_progress: &'static str,
    pub cli_found_videos: &'static str,
    pub cli_choose: &'static str,
    /// `读取选择失败：{}`.
    pub cli_read_choice_failed: &'static str,
    /// `{} 不是一个序号`.
    pub cli_not_a_number: &'static str,
    /// `序号要在 1 到 {} 之间`.
    pub cli_out_of_range: &'static str,
    pub cli_usage: &'static str,
    /// `{} 需要一个数字，拿到的是 {}`.
    pub cli_err_threads: &'static str,
    /// `无法识别的选项 {}（用 --help 看用法）`.
    pub cli_err_unknown_option: &'static str,
    /// `只能给一个录像文件，多出来的是 {}`.
    pub cli_err_one_video: &'static str,
    /// `{} 后面要跟一个值`.
    pub cli_err_needs_value: &'static str,
    /// `读取录像出错：{}`.
    pub cli_producer_failed: &'static str,

    // ── Report rows, shared by the window and the command line ─────────────────
    pub label_video: &'static str,
    pub label_decode: &'static str,
    pub label_recover: &'static str,
    pub label_verify: &'static str,
    pub label_warning: &'static str,
    pub label_timing: &'static str,
    pub label_size: &'static str,
    pub label_stats: &'static str,
    pub label_symbols: &'static str,

    // ── Failures raised below the UI ───────────────────────────────────────────
    /// `符号不足，无法还原。收到 {} 个不同符号…` — unique, needed, frames, codes,
    /// rejected, decode errors.
    pub err_symbols_short: &'static str,
    pub err_cancelled: &'static str,
    /// `初始化 H.264 解码器失败：{}`.
    pub err_h264_init: &'static str,
    pub err_h264_missing: &'static str,
    pub err_iso_not_isobmff: &'static str,
    pub err_iso_fragmented: &'static str,
    pub err_iso_no_video_track: &'static str,
    /// `缺少必需的 {} 表`.
    pub err_iso_missing_table: &'static str,
    /// `文件结构异常：{}`.
    pub err_iso_bad_structure: &'static str,
    /// `不支持的视频编码 {}（只支持 HEVC / H.264）`.
    pub err_iso_unsupported_codec: &'static str,
}

/// Every language must fill in every string, and no string may be empty or left as a
/// copy of the Simplified Chinese one by accident.
///
/// This is the test that catches the realistic failure: a new key added to [`Text`],
/// filled in for one language with an empty string, and shipped as a blank label.
#[cfg(test)]
mod tests {
    use super::*;

    fn all_fields(text: &Text) -> Vec<(&'static str, &'static str)> {
        // Written out rather than derived, because there is no reflection in Rust — but
        // the compiler refuses to build this function if a field is missing, which is
        // exactly the guarantee wanted: adding a key to `Text` breaks this test file
        // until the key is checked in every language.
        let mut out = vec![];
        macro_rules! push {
            ($($field:ident),* $(,)?) => { $( out.push((stringify!($field), text.$field)); )* };
        }
        push!(
            language,
            cjk_font_missing,
            note_unbundled,
            stop,
            quit,
            quit_app,
            cancel,
            clear,
            change,
            seconds_short,
            minutes_only,
            minutes_seconds,
            snd_title,
            snd_tab_send,
            snd_tab_receive,
            snd_intro,
            snd_drop_title,
            snd_drop_sub,
            snd_pick_file,
            snd_pick_dialog_title,
            snd_status_ready,
            snd_status_preparing,
            snd_status_encode_done,
            snd_status_encode_done_skipped,
            snd_status_encode_cancelled,
            snd_status_no_file,
            snd_status_playing,
            snd_status_cancelling,
            snd_status_phase,
            snd_status_stopped,
            snd_status_playback_ended,
            snd_status_player_failed,
            snd_status_player_failed_log,
            snd_status_child_error,
            snd_error_spawn_player,
            snd_error_self_exe,
            snd_hint_title,
            snd_hint_no_file,
            snd_hint_not_ready,
            snd_hint_current,
            snd_hint_cancelling,
            snd_hint_encode_failed,
            snd_got_it,
            snd_cancel_encoding,
            snd_encoding,
            snd_file_summary,
            snd_cancelling_soon,
            snd_progress_note,
            snd_callout_playing,
            snd_callout_idle,
            snd_callout_playing_sub,
            snd_callout_idle_sub,
            snd_settings_title,
            snd_set_preset,
            snd_preset_selected,
            snd_preset_option,
            snd_preset_dual,
            snd_set_lanes,
            snd_lanes_auto,
            snd_set_display,
            snd_fit_display,
            snd_display_main_tag,
            snd_display_label,
            snd_display_fallback,
            snd_window_width,
            snd_set_window,
            snd_borderless,
            snd_will_fill,
            snd_set_cycles,
            snd_cycles_endless,
            snd_play,
            snd_stop_play,
            snd_encoding_hint,
            snd_encode_failed_hint,
            snd_pick_first_hint,
            snd_reencode,
            snd_esc_stops,
            snd_tips_title,
            snd_tips_chip,
            snd_tip_dual_4k60,
            snd_tip_dual_30fps,
            snd_tip_single_4k30,
            snd_tip_lock_focus,
            snd_tip_tripod,
            snd_tip_dnd,
            snd_tip_format,
            snd_rcv_intro,
            snd_rcv_step1_title,
            snd_rcv_step1_body,
            snd_rcv_step1_dual,
            snd_rcv_step1_single,
            snd_rcv_step1_codec,
            snd_rcv_step2_title,
            snd_rcv_step2_movie,
            snd_rcv_step2_tool,
            snd_rcv_step2_note,
            snd_rcv_step3_title,
            snd_rcv_step3_body,
            snd_rcv_step3_warn,
            snd_rcv_step3_cli_note,
            snd_rcv_step3_cli,
            snd_rcv_step3_tail,
            snd_rcv_step4_title,
            snd_rcv_step4_body,
            snd_rcv_step4_note,
            snd_rcv_callout,
            prep_phase_read,
            prep_phase_gzip,
            prep_phase_brotli,
            prep_phase_package,
            prep_method_raw,
            prep_thread_failed,
            prep_thread_died,
            prep_meta_failed,
            prep_read_failed,
            prep_compress_failed,
            prep_too_large,
            prep_skip_incompressible,
            prep_skip_not_worth,
            rcv_title,
            rcv_alert_title,
            rcv_open_window_failed,
            rcv_drop_title,
            rcv_drop_sub,
            rcv_pick,
            rcv_video_filter,
            rcv_scanning_title,
            rcv_none_found_title,
            rcv_looking_in,
            rcv_looked_in,
            rcv_put_together,
            rcv_scanning,
            rcv_rescan,
            rcv_list_title,
            rcv_filter_hint,
            rcv_no_match,
            rcv_count_scroll,
            rcv_search_note,
            rcv_row_action,
            rcv_running,
            rcv_stat_symbols,
            rcv_stat_frames,
            rcv_stat_codes,
            rcv_stat_elapsed,
            rcv_reading,
            rcv_stall_note,
            rcv_done_title,
            rcv_verify_default,
            rcv_reveal,
            rcv_again,
            rcv_failed_title,
            rcv_advice_shortfall,
            rcv_advice_other,
            rcv_retry,
            rcv_other_video,
            rcv_status_waiting,
            rcv_status_progress,
            rcv_status_first_frame,
            rcv_status_done,
            rcv_status_failed,
            rcv_thread_died,
            rcv_summary,
            rcv_counters_line,
            rcv_symbols_line,
            rcv_timing_line,
            rcv_verify_identical,
            rcv_verify_mismatch,
            rcv_verify_mismatch_warn,
            rcv_open_failed,
            rcv_no_frames,
            rcv_mkdir_failed,
            rcv_write_failed,
            rcv_read_failed,
            rcv_dump_failed,
            rcv_json_failed,
            rcv_resume_loaded,
            rcv_resume_failed,
            rcv_manifest_saved,
            rcv_partial_hint,
            cli_banner,
            cli_progress,
            cli_found_videos,
            cli_choose,
            cli_read_choice_failed,
            cli_not_a_number,
            cli_out_of_range,
            cli_usage,
            cli_err_threads,
            cli_err_unknown_option,
            cli_err_one_video,
            cli_err_needs_value,
            cli_producer_failed,
            label_video,
            label_decode,
            label_recover,
            label_verify,
            label_warning,
            label_timing,
            label_size,
            label_stats,
            label_symbols,
            err_symbols_short,
            err_cancelled,
            err_h264_init,
            err_h264_missing,
            err_iso_not_isobmff,
            err_iso_fragmented,
            err_iso_no_video_track,
            err_iso_missing_table,
            err_iso_bad_structure,
            err_iso_unsupported_codec,
        );
        out
    }

    #[test]
    fn no_language_leaves_a_string_empty() {
        for lang in Lang::ALL {
            for (key, value) in all_fields(lang.table()) {
                assert!(
                    !value.trim().is_empty(),
                    "{} has no text for {key}",
                    lang.code()
                );
            }
        }
    }

    /// English must not still be holding the Simplified Chinese text.
    ///
    /// `SHARED` is the honest exception: a template made only of placeholders and ASCII
    /// field names reads the same in every language, so there is nothing to translate.
    #[test]
    fn english_is_translated_rather_than_copied() {
        const SHARED: &[&str] = &["snd_preset_option", "snd_status_phase"];
        let hans = all_fields(Lang::ZhHans.table());
        for ((key, simplified), (_, english)) in hans.iter().zip(all_fields(Lang::En.table())) {
            assert!(!english.trim().is_empty(), "English left {key} empty");
            if SHARED.contains(key) {
                continue;
            }
            assert_ne!(
                *simplified, english,
                "English still holds the Simplified Chinese text for {key}"
            );
        }
    }

    /// Traditional Chinese must be *converted*, not copied.
    ///
    /// The two scripts share most of their characters, so a file copied over and left
    /// half-converted looks fine at a glance and is the realistic way a 繁體 UI ships
    /// broken. This catches it precisely: any character below exists only in Simplified
    /// Chinese, so finding one in the Traditional table means a string was missed.
    #[test]
    fn traditional_chinese_is_converted_rather_than_copied() {
        const SIMPLIFIED_ONLY: &str = "设显录码压缩双击电标验还个为会时后发种别长门问间关开过这说\
             请读认选记让议据报换断续网线结费资员银钱钟铁题单应层级设备";
        for (key, value) in all_fields(Lang::ZhHant.table()) {
            for character in SIMPLIFIED_ONLY.chars() {
                assert!(
                    !value.contains(character),
                    "zh-Hant {key} still contains the Simplified character {character}: {value}"
                );
            }
        }
    }

    #[test]
    fn tags_are_read_the_way_the_platform_writes_them() {
        assert_eq!(Lang::from_tag("zh_CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_tag("zh-Hans"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_tag("zh-Hans-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_tag("zh"), Some(Lang::ZhHans));
        assert_eq!(Lang::from_tag("zh_TW"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_tag("zh-Hant"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_tag("zh-HK"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_tag("zh-MO"), Some(Lang::ZhHant));
        // Script beats region: a machine set to Traditional in Singapore is Traditional.
        assert_eq!(Lang::from_tag("zh-Hant-SG"), Some(Lang::ZhHant));
        assert_eq!(Lang::from_tag("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::from_tag("en-GB"), Some(Lang::En));
        assert_eq!(Lang::from_tag("ja_JP"), None);
        assert_eq!(Lang::from_tag(""), None);
    }

    #[test]
    fn a_missing_language_never_lands_on_an_empty_screen() {
        // Whatever happens, `t()` has to hand back something drawable.
        assert!(!t().snd_status_ready.is_empty());
    }

    /// The choice made in the window has to survive a restart, and a preference file that
    /// has been hand-edited, truncated or made unwritable must not stop the app.
    #[test]
    fn a_chosen_language_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("sendmecongo-i18n-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested/language");

        save_to(&file, Lang::ZhHant);
        assert_eq!(load_from(&file), Some(Lang::ZhHant));
        save_to(&file, Lang::En);
        assert_eq!(load_from(&file), Some(Lang::En));

        // Hand-edited, with the trailing newline a text editor adds.
        std::fs::write(&file, "  en-GB \n").unwrap();
        assert_eq!(load_from(&file), Some(Lang::En));
        // Nonsense falls back to the default rather than to nothing.
        std::fs::write(&file, "klingon").unwrap();
        assert_eq!(load_from(&file), None);
        assert_eq!(
            Lang::from_tag("klingon").unwrap_or(Lang::DEFAULT),
            Lang::ZhHans
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_preference_file_that_is_missing_or_unwritable_is_not_fatal() {
        assert_eq!(load_from(Path::new("/nonexistent/sendmecongo/language")), None);
        // The saving side is best effort; it must not panic on a path it cannot create.
        save_to(Path::new("/dev/null/nope/language"), Lang::En);
    }

    /// Every piece of text this frame drew, with where it landed.
    ///
    /// Reading the drawn shapes rather than guessing coordinates: a test that clicks "about
    /// 20 px below the menu bar" is a test that fails the day egui changes its menu padding.
    fn drawn_text(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    text.galley.text().to_owned(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                )),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// Draw one frame of a window that contains nothing but the menu bar, and report the
    /// text it drew.
    fn menu_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("menu").show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    language_menu(ui);
                });
            });
        });
        drawn_text(&output)
    }

    fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
        let modifiers = egui::Modifiers::default();
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            },
        ]
    }

    /// Where a piece of text was drawn, if it was drawn at all.
    fn find(texts: &[(String, egui::Rect)], needle: &str) -> Option<egui::Rect> {
        texts
            .iter()
            .find(|(text, _)| text.contains(needle))
            .map(|(_, rect)| *rect)
    }

    /// The whole point of the menu: open it, pick a language, and see the interface follow.
    ///
    /// Exercises the real `language_menu` in a real (headless) egui context with real
    /// pointer events, which is the only way to know that clicking actually works — a
    /// combo box in the status bar looked fine on screen and could not be clicked open.
    #[test]
    fn clicking_the_language_menu_switches_the_language() {
        // This test really does switch the language, and switching writes the preference
        // file. A test must not quietly rewrite the developer's own setting, so the file
        // is saved and put back.
        let preference = preference_file();
        let saved_file = preference.as_ref().and_then(|p| std::fs::read(p).ok());
        let started = current();

        let ctx = egui::Context::default();
        let label = started.table().language.to_owned();

        // 1. The menu bar knows the menu by its title, in the live language.
        let texts = menu_frame(&ctx, vec![]);
        let button = find(&texts, &label).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"));

        // 2. Clicking it opens a menu holding every language, named in its own script.
        let opened = menu_frame(&ctx, click_at(button.center()));
        let opened = if find(&opened, Lang::ZhHant.native_name()).is_some() {
            opened
        } else {
            // The popup may only be laid out on the frame after the click.
            menu_frame(&ctx, vec![])
        };
        for lang in Lang::ALL {
            assert!(
                find(&opened, lang.native_name()).is_some(),
                "{} is missing from the menu: {opened:?}",
                lang.code()
            );
        }

        // 3. Picking one switches immediately, and remembers.
        let item = find(&opened, Lang::ZhHant.native_name()).expect("item is on screen");
        assert_ne!(started, Lang::ZhHant, "test must start elsewhere");
        menu_frame(&ctx, click_at(item.center()));
        assert_eq!(current(), Lang::ZhHant, "the click did not take effect");
        let written = preference
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        assert_eq!(written.trim(), Lang::ZhHant.code());

        // 4. Put the developer's own setting back, byte for byte.
        set_lang_ephemeral(started);
        if let Some(path) = preference {
            match saved_file {
                Some(bytes) => std::fs::write(path, bytes).expect("restore preference"),
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }

    #[test]
    fn fill_puts_every_argument_in_its_place() {
        assert_eq!(fill("{} and {}", &[&"a", &"b"]), "a and b");
        // Word order is the translator's business, so a reordered template works too.
        assert_eq!(fill("{} · {}", &[&1, &"x"]), "1 · x");
        // Fewer arguments than placeholders leaves the leftover visible on purpose.
        assert_eq!(fill("{} {}", &[&"only"]), "only {}");
        // More arguments than placeholders are dropped rather than appended.
        assert_eq!(fill("{}", &[&"a", &"b"]), "a");
        assert_eq!(fill("no placeholders", &[]), "no placeholders");
    }

    #[test]
    fn report_columns_line_up_in_every_language() {
        for lang in Lang::ALL {
            let text = lang.table();
            for label in [
                text.label_video,
                text.label_decode,
                text.label_recover,
                text.label_verify,
                text.label_warning,
                text.label_timing,
            ] {
                let padded = pad_label(label);
                let columns: usize = padded.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum();
                assert_eq!(columns, 10, "{} {label:?} pads to {columns}", lang.code());
            }
        }
    }

    #[test]
    fn the_verify_token_survives_translation() {
        // `json()` and tools/verify-recv.sh both look for these two words.
        for lang in Lang::ALL {
            assert!(lang.table().rcv_verify_identical.starts_with("IDENTICAL ✓"));
            assert!(lang.table().rcv_verify_mismatch.starts_with("MISMATCH ✗"));
        }
    }
}
