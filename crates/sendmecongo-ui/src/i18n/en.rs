//! English.
//!
//! The register is deliberately the same as the Chinese: a tool that is blunt about what
//! it needs the operator to do, and that says so in full sentences rather than labels.
//! Nothing here is machine-translated padding.

use super::Text;

pub static TEXT: Text = Text {
    // ── Shared ──────────────────────────────────────────────────────────────────
    language: "Language",
    cjk_font_missing: "No system CJK font was found — some labels may show as boxes",
    note_unbundled: "Note: running the binary directly also opens this terminal window. \
        Launch the {}.app in dist/ or the app inside the DMG instead and it will not appear.",
    stop: "Stop",
    quit: "Quit",
    quit_app: "Quit {}",
    cancel: "Cancel",
    clear: "Remove",
    change: "Change",
    seconds_short: "{} s",
    minutes_only: "{} min",
    minutes_seconds: "{}m {}s",

    // ── Sender window ───────────────────────────────────────────────────────────
    snd_title: "SendMeCongo Sender",
    snd_tab_send: "Send",
    snd_tab_receive: "Receiver guide",
    snd_intro: "Plays the file as a stream of QR codes; another machine films the screen and \
        recovers it. One-way, no network, no USB stick.",
    snd_drop_title: "Drop the file to send",
    snd_drop_sub: "The file is compressed and encoded on this machine only; nothing goes over a network",
    snd_pick_file: "Choose a file…",
    snd_pick_dialog_title: "Choose the file to export",
    snd_status_ready: "Ready",
    snd_status_preparing: "Preparing…",
    snd_status_encode_done: "Encoding finished",
    snd_status_encode_done_skipped: "Encoding finished (brotli was skipped — see the note below)",
    snd_status_encode_cancelled: "Encoding cancelled",
    snd_status_no_file: "No file chosen yet",
    snd_status_playing: "Playing · {}×{}px · {} lane(s) · {} sym/s",
    snd_status_cancelling: "Cancelling {}%",
    snd_status_phase: "{} {}%",
    snd_status_stopped: "Stopped",
    snd_status_playback_ended: "Playback finished",
    snd_status_player_failed: "Player exited abnormally ({})",
    snd_status_player_failed_log: "Player exited abnormally ({}): {}",
    snd_status_child_error: "Could not read the player's status: {}",
    snd_error_spawn_player: "Could not start the player: {}",
    snd_error_self_exe: "Cannot locate my own executable: {}",
    snd_hint_title: "Notice",
    snd_hint_no_file: "Choose the file to send first, or drag it into this window.",
    snd_hint_not_ready: "Encoding has not finished — let the progress bar run out before starting playback.",
    snd_hint_current: "Now: {} {}%",
    snd_hint_cancelling: "Cancelling the encoding, one moment…",
    snd_hint_encode_failed: "Encoding failed",
    snd_got_it: "Got it",
    snd_cancel_encoding: "Cancel encoding",
    snd_encoding: "Encoding",
    snd_file_summary: "{} → {} · {} symbols · {} · encoded in {} s",
    snd_cancelling_soon: "Cancelling — nearly done…",
    snd_progress_note: "A large file takes a few seconds to compress; playback starts as soon as it is done",
    snd_callout_playing: "Playing · record for ≥ {}",
    snd_callout_idle: "Record for ≥ {}",
    snd_callout_playing_sub: "Keep the phone recording until the receiver reports the file is \
        recovered. Stopping early only means filming from the start again.",
    snd_callout_idle_sub: "{} is enough in the ideal case; leave twice that for focus and \
        occlusions. A longer recording only repeats symbols, it never goes wrong.",
    snd_settings_title: "Playback settings",
    snd_set_preset: "Preset",
    snd_preset_selected: "{} · {} sym/s · nominal {} KB/s",
    snd_preset_option: "{} · {} sym/s · {} KB/s{}",
    snd_preset_dual: " · dual-lane",
    snd_set_lanes: "Lanes",
    snd_lanes_auto: "Auto ({})",
    snd_set_display: "Display",
    snd_fit_display: "Fill the display",
    snd_display_main_tag: " · main",
    snd_display_label: "Display {} · {}×{}",
    snd_display_fallback: "Main display (assuming 1920×1080)",
    snd_window_width: "Window width px",
    snd_set_window: "Window",
    snd_borderless: "Borderless fullscreen",
    snd_will_fill: "Will fill {}×{} @ ({}, {})",
    snd_set_cycles: "Loops",
    snd_cycles_endless: "0 = endless",
    snd_play: "▶ Start",
    snd_stop_play: "■ Stop",
    snd_encoding_hint: "encoding…",
    snd_encode_failed_hint: "Encoding failed — press “Re-encode” to try again",
    snd_pick_first_hint: "choose a file first",
    snd_reencode: "Re-encode",
    snd_esc_stops: "Press ESC to stop playback",
    snd_tips_title: "Shooting requirements",
    snd_tips_chip: "decides success",
    snd_tip_dual_4k60: "Dual-lane needs a 4K60 recording",
    snd_tip_dual_30fps: "At 30 fps the two lanes alternately lose focus and throughput halves",
    snd_tip_single_4k30: "Single lane: 4K30 is enough",
    snd_tip_lock_focus: "Long-press to lock AE / AF",
    snd_tip_tripod: "Phone on a stand, and do not move it",
    snd_tip_dnd: "Turn on Do Not Disturb",
    snd_tip_format: "No need to change the recording format (HEVC or H.264 both work)",
    snd_rcv_intro: "The receiver is someone else, on another machine. Pass this page on to them, \
        or just follow it.",
    snd_rcv_step1_title: "Record the whole session",
    snd_rcv_step1_body: "Point the phone at the screen and record from the start until the \
        receiver says it is done. Lock focus, silence notifications, fix the phone in place.",
    snd_rcv_step1_dual: "This preset is dual-lane — a 4K60 recording is required (at 30 fps the \
        two lanes alternately lose focus and throughput halves)",
    snd_rcv_step1_single: "This preset is single-lane; 4K30 is enough",
    snd_rcv_step1_codec: "Both HEVC and H.264 are accepted, so the camera format needs no change.",
    snd_rcv_step2_title: "Hand two things to the receiver",
    snd_rcv_step2_movie: "① The recording itself (.mov / .mp4, as it is, no transcoding)",
    snd_rcv_step2_tool: "② The receiver tool: on macOS send dist/sendmecongo-recv-<version>.dmg; \
        on Windows send dist\\sendmecongo-recv.exe",
    snd_rcv_step2_note: "Nothing has to be installed on their side: no Python, no codecs, no \
        network. The mounted dmg carries a first-launch note and a one-click fix-up script \
        (the app has no Apple developer signature, so the system blocks the first launch \
        after a download). Apple Silicon Macs only; copying it over on a USB stick does not \
        trigger that block.",
    snd_rcv_step3_title: "Recover the file on the receiving machine",
    snd_rcv_step3_body: "Double-click the receiver tool, then drag the recording into its window \
        or pick one from the list it shows. There is nothing to configure.",
    snd_rcv_step3_warn: "On macOS, dropping the file onto the window works but dropping it onto \
        the Dock or Finder icon does not (the system never hands the path over). On Windows you \
        can drop it straight onto the .exe.",
    snd_rcv_step3_cli_note: "The command line still works for scripting, and runs exactly the \
        same code as the window:",
    snd_rcv_step3_cli: "sendmecongo-recv recording.mov --out <dir> --compare <original>",
    snd_rcv_step3_tail: "Duration, codec, frame rate and lane count are all read out of the file; \
        the recovered file lands next to the recording by default, under its original name.",
    snd_rcv_step4_title: "Check the result",
    snd_rcv_step4_body: "Every symbol is verified and the file fingerprint is compared. \
        “IDENTICAL ✓” means a byte-for-byte match.",
    snd_rcv_step4_note: "If symbols are missing it will not hand over a broken file — it tells \
        you how many are short and what may have caused it.",
    snd_rcv_callout: "The stream loops, and every code carries its own number and checksum — so \
        record too long rather than too short. Extra frames are only repeats and cannot cause an \
        error; missing symbols come round again in later passes, and the tool stops by itself \
        once it has everything.",

    // ── Sender: the prepare worker ──────────────────────────────────────────────
    prep_phase_read: "Reading the file",
    prep_phase_gzip: "Compressing (gzip)",
    prep_phase_brotli: "Compressing (brotli, high quality)",
    prep_phase_package: "Packaging",
    prep_method_raw: "uncompressed",
    prep_thread_failed: "Could not start the encoding thread: {}",
    prep_thread_died: "the encoding thread ended unexpectedly",
    prep_meta_failed: "Cannot read the file: {}",
    prep_read_failed: "Reading the file failed: {}",
    prep_compress_failed: "Compression failed: {}",
    prep_too_large: "{} is {}, over the 64 MB limit.\n\n\
        The optical link measures about 100 KB/s, so this would need ten minutes of filming, \
        and preparing it peaks at four to five times the file size in memory.\n\n\
        Compress or split it first.",
    prep_skip_incompressible: "brotli skipped: the data does not compress ({} → {}), so pressing \
        harder only costs time.",
    prep_skip_not_worth: "brotli skipped: it would save only {} (about {}s of filming) at a cost \
        of {}s of compression.",

    // ── Receiver window ────────────────────────────────────────────────────────
    rcv_title: "SendMeCongo Receiver",
    rcv_alert_title: "sendmecongo-recv could not open a window",
    rcv_open_window_failed: "Could not open a window: {}\nWith no graphics environment, use the \
        command line instead: sendmecongo-recv recording.mov",
    rcv_drop_title: "Drop a phone recording here",
    rcv_drop_sub: "Nothing to set up — codec, frame rate and length are detected automatically",
    rcv_pick: "Choose a recording…",
    rcv_video_filter: "Video",
    rcv_scanning_title: "Looking for recordings nearby…",
    rcv_none_found_title: "No recordings found nearby",
    rcv_looking_in: "Looking in these places:",
    rcv_looked_in: "It looked in these places:",
    rcv_put_together: "Put sendmecongo-recv.app and the recording in the same folder and it appears here.",
    rcv_scanning: "Looking…",
    rcv_rescan: "Look again",
    rcv_list_title: "Recordings nearby ({} found, newest first)",
    rcv_filter_hint: "Filter by name or path…",
    rcv_no_match: "No recording matches",
    rcv_count_scroll: "{} in total · scroll to see them all",
    rcv_search_note: "Searched: the folder the app lives in · Downloads · Desktop · Movies. \
        A recording in any of these shows up in the list.",
    rcv_row_action: "Recover ›",
    rcv_running: "Recovering…",
    rcv_stat_symbols: "symbols collected",
    rcv_stat_frames: "frames decoded",
    rcv_stat_codes: "QR codes found",
    rcv_stat_elapsed: "time elapsed",
    rcv_reading: "reading",
    rcv_stall_note: "The progress bar pauses now and then while missing symbols come round again \
        — it is not stuck. It stops by itself once complete, and extra recording time costs nothing.",
    rcv_done_title: "Recovered",
    rcv_verify_default: "the container CRC matches the original length / CRC-32",
    rcv_reveal: "Reveal in Finder",
    rcv_again: "Recover another",
    rcv_failed_title: "The file could not be recovered",
    rcv_advice_shortfall: "Not enough symbols arrived, or the shot lost focus or was blocked \
        partway.\nRecord a longer clip and try again — recording too long never goes wrong.",
    rcv_advice_other: "This recording could not be read or processed.\n\
        The details below can be sent to the sender as they are.",
    rcv_retry: "Try this recording again",
    rcv_other_video: "Use another recording",
    rcv_status_waiting: "Waiting for a recording",
    rcv_status_progress: "{} / {} symbols collected",
    rcv_status_first_frame: "Reading the first frame…",
    rcv_status_done: "Done",
    rcv_status_failed: "Failed",
    rcv_thread_died: "The worker thread ended unexpectedly.",
    rcv_summary: "{} · {}×{} · {} · {} frames · {} fps · {} threads",
    rcv_counters_line: "{} frames decoded · {} codes found · {} distinct symbols ({} received, {} duplicates)",
    rcv_symbols_line: "symbol payload {} B · container {} B · source symbols K≈{}",
    rcv_timing_line: "{} s · {} throughput",
    rcv_verify_identical: "IDENTICAL ✓  byte-for-byte with {}",
    rcv_verify_mismatch: "MISMATCH ✗  differs from {}",
    rcv_verify_mismatch_warn: "The recovered file does not match {} (length {} vs {}) — keep this recording",
    rcv_open_failed: "Could not open the recording: {}",
    rcv_no_frames: "The recording contains no frames at all.",
    rcv_mkdir_failed: "Could not create the output directory: {}",
    rcv_write_failed: "Could not write {}: {}",
    rcv_read_failed: "Could not read {}: {}",
    rcv_dump_failed: "Could not write the symbol dump: {}",
    rcv_json_failed: "Could not write the JSON: {}",

    // ── Receiver: the command line ─────────────────────────────────────────────
    cli_banner: "sendmecongo-recv — recover a file from a recording",
    cli_progress: "  {} frames decoded · {} codes · {}/{} symbols · {} elapsed",
    cli_found_videos: "Found these recordings (newest first):",
    cli_choose: "Which one? [1] ",
    cli_read_choice_failed: "Could not read the choice: {}",
    cli_not_a_number: "{} is not a number",
    cli_out_of_range: "The number has to be between 1 and {}",
    cli_usage: concat!(
        "sendmecongo-recv — recover a file that SendMeCongo sent, from a phone recording\n",
        "\n",
        "Usage\n",
        "    sendmecongo-recv <recording.mov> [options]\n",
        "    sendmecongo-recv                     with no arguments, opens a window (macOS; set\n",
        "                                     SENDMECONGO_NO_DIALOG=1 to force the command line,\n",
        "                                     which finds recordings nearby and asks which one)\n",
        "\n",
        "Options\n",
        "    --out <dir>         where to write the recovered file (default: next to the recording)\n",
        "    --compare <file>    compare the result with the original, byte for byte\n",
        "    --dump <file>       export the symbols received, for an independent\n",
        "                        `sendmecongo-bench decode` cross-check\n",
        "    --json <file>       write machine-readable statistics\n",
        "    --threads <N>       decode threads (default: derived from the CPU core count)\n",
        "    --lang <code>       interface language: zh-Hans / zh-Hant / en (default: the locale)\n",
        "    --quiet, -q         print no progress and no report\n",
        "    --gui               open the window (what double-clicking the .app does on macOS)\n",
        "\n",
        "It needs nothing else installed: no Python, no OpenCV, no Rust toolchain.\n",
        "\n",
        "macOS ships sendmecongo-recv.app. Double-clicking it opens a window: drag the recording into\n",
        "it, or pick one from the list it shows (it looks in its own folder, in Downloads /\n",
        "Desktop / Movies, and at the top level of every mounted volume). **The quickest way is to\n",
        "put sendmecongo-recv.app and the recording in the same folder and double-click the app.**\n",
        "\n",
        "The window shows a progress bar, the frames decoded and the symbols collected (how many\n",
        "are needed is read out of the stream itself); press Stop at any time, and reveal the\n",
        "recovered file in Finder when it is done.\n",
        "\n",
        "Notes\n",
        "    The recording has to capture the whole QR stream played on the screen. Recording for\n",
        "    too long is harmless — every step of the stream carries its own number and checksum,\n",
        "    repeats are ignored, and anything missed comes round again in later passes.",
    ),
    cli_err_threads: "{} needs a number, got {}",
    cli_err_unknown_option: "Unrecognised option {} (see --help)",
    cli_err_one_video: "Only one recording can be given; the extra one is {}",
    cli_err_needs_value: "{} needs a value after it",
    cli_producer_failed: "Error while reading the recording: {}",

    // ── Report rows, shared by the window and the command line ─────────────────
    label_video: "Video",
    label_decode: "Decode",
    label_recover: "Recover",
    label_verify: "Verify",
    label_warning: "Warning",
    label_timing: "Elapsed",
    label_size: "Size",
    label_stats: "Stats",
    label_symbols: "Symbols",

    // ── Failures raised below the UI ───────────────────────────────────────────
    err_symbols_short: "Not enough symbols to recover the file: {} distinct symbols arrived and \
        about {} are needed.\n\
        {} frames decoded · {} QR codes found · {} symbols failed their checksum · {} frames \
        failed to decode\n\
        The usual causes: the recording is too short, focus was not locked, the phone moved, or \
        screen brightness / a night filter blurred the codes.\n\
        Record again and record longer (twice as long is fine — the fountain code simply absorbs \
        the repeats).",
    err_cancelled: "Stopped.",
    err_h264_init: "Could not initialise the H.264 decoder: {}",
    err_h264_missing: "This recording is H.264, but this build of sendmecongo-recv has no H.264 \
        support compiled in.\n\
        Use the official full build instead; or have the other side switch the phone camera \
        format to “High Efficiency” (HEVC) and record again.",
    err_iso_not_isobmff: "Not an MP4/MOV file (no moov box found)",
    err_iso_fragmented: "This is a fragmented MP4 (moof), which is not supported. Use a .mov/.mp4 \
        recorded by the system camera.",
    err_iso_no_video_track: "The file has no video track",
    err_iso_missing_table: "the required {} table is missing",
    err_iso_bad_structure: "The file structure is malformed: {}",
    err_iso_unsupported_codec: "Unsupported video codec {} (only HEVC / H.264 are supported)",
};
