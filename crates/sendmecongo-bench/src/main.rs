//! sendmecongo-bench — M0 measurement tooling.
//!
//!   capacity                            calibrate QR byte-mode capacity per version
//!   encode   --in F --preset P --out D  render one full cycle of frames as PNGs
//!   simulate --in F --preset P          erasure simulation: how many frames until done
//!   play     --in F --preset P          full-screen-ish endless QR stream (ESC to stop)
//!   decode   --in DUMP --out D          rebuild a file from symbols dumped by tools/analyze.py

use sendmecongo_core::preset::{self, Preset};
use sendmecongo_core::{Receiver, Sender};
use qrcode::EcLevel;
use std::collections::HashMap;
use std::path::Path;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BoxResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "capacity" => cmd_capacity(),
        "encode" => cmd_encode(parse_flags(&args[2..])),
        "simulate" => cmd_simulate(parse_flags(&args[2..])),
        "play" => cmd_play(parse_flags(&args[2..])),
        "decode" => cmd_decode(parse_flags(&args[2..])),
        "icon" => cmd_icon(parse_flags(&args[2..])),
        _ => {
            usage();
            Ok(())
        }
    }
}

fn usage() {
    println!("sendmecongo-bench");
    println!("  capacity");
    println!("  encode   --in <file> --preset <name> --out <dir> [--scale 6] [--max-frames N]");
    println!("  simulate --in <file> --preset <name> [--drops 0,10,20,30,40]");
    println!("  play     --in <file> --preset <name> [--size 1600] [--lanes N] [--cycles N]");
    println!("             [--borderless] [--at-x N --at-y N]");
    println!("             --size = 窗口总宽度；每个二维码边长 = size / lanes（保持正方形）");
    println!("             --borderless + --at-x/--at-y = 铺满指定显示器（多屏时用）");
    println!("  decode   --in <dump.bin> --out <dir> [--compare <original file>]");
    println!("  icon     --out <icon.png> --variant <send|recv> [--size 1024]");
    println!("\npresets: robust balanced turbo15 turbo30 turbo60 megabit");
}

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            if let Some(value) = args.get(i + 1) {
                map.insert(key.to_string(), value.clone());
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    map
}

fn flag<'a>(map: &'a HashMap<String, String>, key: &str, default: &'a str) -> &'a str {
    map.get(key).map(|s| s.as_str()).unwrap_or(default)
}

fn cmd_capacity() -> BoxResult<()> {
    println!("byte-mode capacity (bytes per QR symbol)");
    println!("{:>8} {:>8} {:>8} {:>8} {:>8}", "version", "L", "M", "Q", "H");
    for version in [5i16, 10, 15, 20, 25, 30, 35, 40] {
        let cells = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H]
            .map(|ec| sendmecongo_core::qr::capacity(version, ec).to_string());
        println!(
            "{:>8} {:>8} {:>8} {:>8} {:>8}",
            version, cells[0], cells[1], cells[2], cells[3]
        );
    }

    println!("\npresets (byte mode, ECC L, {}B frame overhead)", sendmecongo_core::frame::OVERHEAD);
    println!(
        "{:<10} {:>6} {:>3} {:>7} {:>6} {:>10} {:>10}",
        "preset", "ver", "lane", "sym/s", "symB", "KB/s", "Mbps"
    );
    for p in preset::ALL {
        println!(
            "{:<10} {:>6} {:>3} {:>7} {:>6} {:>10.1} {:>10.3}",
            p.name,
            p.version,
            p.lanes,
            p.fps,
            p.symbol_size(),
            p.nominal_bps() / 1024.0,
            p.nominal_mbps()
        );
    }
    Ok(())
}

/// Which end of the optical link an icon stands for.
///
/// The two apps are two halves of one link, so the two icons are one drawing: the
/// receiver is the sender mirrored about the vertical axis, with the accent hue moved
/// from cyan to mint. Nothing else differs — that is the whole point of the pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IconVariant {
    Send,
    Recv,
}

impl IconVariant {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "send" => Some(Self::Send),
            "recv" => Some(Self::Recv),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Recv => "recv",
        }
    }

    fn mirrored(self) -> bool {
        matches!(self, Self::Recv)
    }

    /// The three tones, lightest first: the source disc, the core of the beam, and the
    /// spill around it. Three steps rather than two so the disc does not merge with the
    /// core into one arrow — that reads as a play button, not as light.
    fn colors(self) -> IconPalette {
        match self {
            Self::Send => IconPalette {
                spill: rgb(0x1B, 0x5E, 0x75),
                core: rgb(0x2F, 0xC3, 0xD8),
                source: rgb(0x7F, 0xEB, 0xF7),
            },
            Self::Recv => IconPalette {
                spill: rgb(0x1E, 0x5F, 0x4C),
                core: rgb(0x46, 0xD1, 0x9A),
                source: rgb(0xA8, 0xF5, 0xD2),
            },
        }
    }
}

struct IconPalette {
    spill: [f32; 3],
    core: [f32; 3],
    source: [f32; 3],
}

/// Everything below is laid out in a 100×100 space so the geometry is resolution-
/// independent: `--size 1024` for the app icon and `--size 16` for the small end of the
/// iconset run through the same code.
const ICON_UNITS: f32 = 100.0;
/// Apple's continuous-corner proportion (0.2237 × edge), as a plain corner radius.
const ICON_RADIUS: f32 = 22.37;
const ICON_BASE: [f32; 3] = rgb(0x12, 0x2A, 0x3A);
/// The light source, in the sender's orientation: a disc at x 24, y 50, radius 11.
///
/// Deliberately round. A standing rectangular bar in this position plus a solid cone
/// opening away from it is the system volume glyph — two apps sitting side by side in the
/// Dock would read as "volume" and "mute". A disc has no such reading, and it is what a
/// bare emitter actually looks like.
const SOURCE: (f32, f32, f32) = (25.0, 50.0, 12.0);
/// The beam, apex just inside the source, opening away from it to x = 86.
///
/// Sized to leave about a quarter of the canvas as slate above and below: a beam that
/// fills the icon reads as a flat glyph, not as an object sitting on a surface.
const SPILL: ((f32, f32), (f32, f32), (f32, f32)) = ((29.0, 50.0), (86.0, 26.0), (86.0, 74.0));
/// The bright core inside the spill: same apex, narrower cone.
const CORE: ((f32, f32), (f32, f32), (f32, f32)) = ((29.0, 50.0), (86.0, 41.0), (86.0, 59.0));

const fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32, g as f32, b as f32]
}

/// Render the source PNG for one of the two application icons: a slate squircle carrying
/// a block that emits (or receives) a cone of light.
///
/// `tools/build-macos.sh` feeds each variant through `sips` and `iconutil` to make an
/// `AppIcon.icns`. Drawing it in code rather than shipping a hand-made asset keeps the
/// build hermetic and lets both icons come out of one geometry.
fn cmd_icon(map: HashMap<String, String>) -> BoxResult<()> {
    let out = flag(&map, "out", "icon.png");
    let size: u32 = flag(&map, "size", "1024").parse()?;
    let variant =
        IconVariant::parse(flag(&map, "variant", "send")).ok_or("--variant 只能是 send 或 recv")?;
    if size == 0 {
        return Err("--size 不能是 0".into());
    }

    // 4×4 sub-samples per pixel. The only place coverage ever drops below 1 is the outer
    // squircle edge, so averaging the hit samples and pushing the rest into alpha gives a
    // clean silhouette without a second pass over the interior edges.
    const SUBSAMPLES: u32 = 4;
    let per_pixel = (SUBSAMPLES * SUBSAMPLES) as f32;
    let scale = ICON_UNITS / size as f32;

    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let (mut acc, mut hits) = ([0.0f32; 3], 0u32);
            for sy in 0..SUBSAMPLES {
                for sx in 0..SUBSAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SUBSAMPLES as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SUBSAMPLES as f32) * scale;
                    if let Some(c) = icon_color(variant, x, y) {
                        for channel in 0..3 {
                            acc[channel] += c[channel];
                        }
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                continue; // outside the squircle: leave the pixel transparent
            }

            let mut color = [acc[0] / hits as f32, acc[1] / hits as f32, acc[2] / hits as f32];
            apply_highlight(
                &mut color,
                (px as f32 + 0.5) * scale,
                (py as f32 + 0.5) * scale,
            );

            let index = ((py * size + px) * 4) as usize;
            for channel in 0..3 {
                rgba[index + channel] = color[channel].round().clamp(0.0, 255.0) as u8;
            }
            rgba[index + 3] = (hits as f32 / per_pixel * 255.0).round() as u8;
        }
    }

    let image = image::RgbaImage::from_raw(size, size, rgba)
        .ok_or("icon buffer size does not match its dimensions")?;
    image.save(out)?;

    println!(
        "icon        {out}  ({size}×{size}, variant {})",
        variant.name()
    );
    Ok(())
}

/// Which piece of the drawing a point lands on. Kept separate from the colours so the
/// geometry can be asserted independently of which variant is being drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IconPart {
    Outside,
    Base,
    Spill,
    Core,
    Source,
}

/// Where a point in the 100×100 layout falls. One drawing, two orientations: the receiver
/// is the sender sampled mirrored, so this is the only place the mirror is applied.
fn icon_part(variant: IconVariant, x: f32, y: f32) -> IconPart {
    if sd_squircle(x, y, ICON_RADIUS) > 0.0 {
        return IconPart::Outside;
    }
    let x = if variant.mirrored() {
        ICON_UNITS - x
    } else {
        x
    };
    // Source first, so the beam reads as emerging from behind a whole disc. Checking the
    // cones first would bite a wedge out of the disc's outer edge.
    if (x - SOURCE.0).hypot(y - SOURCE.1) <= SOURCE.2 {
        return IconPart::Source;
    }
    // Core before spill: the core is the smaller cone and must win where they overlap.
    if in_triangle((x, y), CORE.0, CORE.1, CORE.2) {
        return IconPart::Core;
    }
    if in_triangle((x, y), SPILL.0, SPILL.1, SPILL.2) {
        return IconPart::Spill;
    }
    IconPart::Base
}

/// Colour of the icon at a point in the 100×100 layout, or `None` outside the silhouette.
fn icon_color(variant: IconVariant, x: f32, y: f32) -> Option<[f32; 3]> {
    let palette = variant.colors();
    match icon_part(variant, x, y) {
        IconPart::Outside => None,
        IconPart::Base => Some(ICON_BASE),
        IconPart::Spill => Some(palette.spill),
        IconPart::Core => Some(palette.core),
        IconPart::Source => Some(palette.source),
    }
}

/// A very light top-left wash, the way Big Sur-era system icons are lit. Applied last and
/// to every colour including the accents, so the icon reads as one lit object rather than
/// a flat sticker. The floor is deliberately tiny: it must not survive downscaling to
/// 16 px as grey fringing on the corners.
fn apply_highlight(color: &mut [f32; 3], x: f32, y: f32) {
    let t = ((x + y) / (2.0 * ICON_UNITS)).clamp(0.0, 1.0);
    let lift = 0.07 * (1.0 - t).powf(2.2);
    let sink = 0.045 * t.powf(2.2);
    for channel in color.iter_mut() {
        *channel += (255.0 - *channel) * lift;
        *channel -= *channel * sink;
    }
}

/// Squircle silhouette as a signed distance: rounded rectangle with the corner radius
/// taken from the icon size.
fn sd_squircle(x: f32, y: f32, radius: f32) -> f32 {
    let half = ICON_UNITS / 2.0;
    let straight = half - radius;
    let (dx, dy) = ((x - half).abs(), (y - half).abs());
    let (ox, oy) = ((dx - straight).max(0.0), (dy - straight).max(0.0));
    ox.hypot(oy) - radius
}

/// Point-in-triangle by edge signs; accepts either winding.
fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), a: (f32, f32), b: (f32, f32)| {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    };
    let (d1, d2, d3) = (side(p, a, b), side(p, b, c), side(p, c, a));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

fn cmd_encode(map: HashMap<String, String>) -> BoxResult<()> {
    let input = flag(&map, "in", "");
    let out_dir = flag(&map, "out", "out");
    let preset_name = flag(&map, "preset", "turbo30");
    let scale: u32 = flag(&map, "scale", "6").parse()?;
    if input.is_empty() {
        usage();
        return Ok(());
    }
    let preset =
        preset::by_name(preset_name).ok_or_else(|| format!("unknown preset: {preset_name}"))?;

    let path = Path::new(input);
    let data = std::fs::read(path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("payload.bin");

    let sender = Sender::new(name, &data, preset.symbol_size(), preset.repair_pct)?;
    let frames = sender.frames();
    let max_frames: usize = match map.get("max-frames") {
        Some(v) => v.parse()?,
        None => frames.len(),
    };
    let count = frames.len().min(max_frames);

    std::fs::create_dir_all(out_dir)?;
    for (i, frame) in frames.iter().enumerate().take(count) {
        let png = sendmecongo_core::qr::png(frame, preset.version, preset.ec, scale)?;
        std::fs::write(format!("{out_dir}/{i:05}.png"), png)?;
    }

    // Sanity: the full cycle must be self-sufficient (independent of how many PNGs we wrote).
    let mut receiver = Receiver::new();
    let mut rebuilt = None;
    for frame in frames {
        if let Some(r) = receiver.push(frame)? {
            rebuilt = Some(r);
            break;
        }
    }
    let verified = match &rebuilt {
        Some(r) => r.data == data && r.name == name,
        None => false,
    };

    let object_len = frames
        .first()
        .and_then(|f| sendmecongo_core::frame::parse(f).ok())
        .map(|(h, _)| h.object_len)
        .unwrap_or(0);
    let source_symbols = (object_len as usize).div_ceil(preset.symbol_size() as usize).max(1);

    let manifest = format!(
        concat!(
            "{{\n",
            "  \"preset\": \"{}\",\n",
            "  \"qr_version\": {},\n",
            "  \"ec\": \"L\",\n",
            "  \"scale\": {},\n",
            "  \"symbol_size\": {},\n",
            "  \"object_len\": {},\n",
            "  \"source_symbols\": {},\n",
            "  \"frames_written\": {},\n",
            "  \"frames_total\": {},\n",
            "  \"lanes\": {},\n",
            "  \"fps\": {},\n",
            "  \"hold_refreshes\": {},\n",
            "  \"repair_pct\": {},\n",
            "  \"orig_len\": {},\n",
            "  \"name\": \"{}\",\n",
            "  \"roundtrip_ok\": {},\n",
            "  \"nominal_bytes_per_sec\": {}\n",
            "}}\n"
        ),
        preset.name,
        preset.version,
        scale,
        preset.symbol_size(),
        object_len,
        source_symbols,
        count,
        frames.len(),
        preset.lanes,
        preset.fps,
        preset.hold_refreshes,
        preset.repair_pct,
        data.len(),
        escape_json(name),
        verified,
        preset.nominal_bps() as u64,
    );
    std::fs::write(format!("{out_dir}/manifest.json"), manifest)?;

    println!("preset            {}", preset.name);
    println!("file              {} ({} bytes)", name, data.len());
    println!("symbol size       {} B", preset.symbol_size());
    println!("source symbols    {}", source_symbols);
    println!("frames            {} written / {} in one cycle", count, frames.len());
    println!("nominal rate      {:.1} KB/s ({:.3} Mbps)", preset.nominal_bps() / 1024.0, preset.nominal_mbps());
    let cycle = frames.len() as f64 / preset.fps;
    println!("one cycle         {:.1} s (需 {} 帧)", cycle, frames.len());
    println!(
        "effective         {:.1} KB/s (含 {}% 修复符号开销，未计摄像头损耗)",
        (data.len() as f64 / cycle) / 1024.0,
        preset.repair_pct
    );
    println!("roundtrip         {}", if verified { "OK" } else { "FAILED" });
    println!("output            {out_dir}/");
    Ok(())
}

fn cmd_play(map: HashMap<String, String>) -> BoxResult<()> {
    let input = flag(&map, "in", "");
    let preset_name = flag(&map, "preset", "turbo30");
    if input.is_empty() {
        usage();
        return Ok(());
    }
    let preset =
        preset::by_name(preset_name).ok_or_else(|| format!("unknown preset: {preset_name}"))?;
    let size: usize = flag(&map, "size", "900").parse()?;
    let cycles: usize = flag(&map, "cycles", "0").parse()?;
    let lanes_arg: usize = flag(&map, "lanes", "0").parse()?;
    let borderless = map.contains_key("borderless");
    // --at-x / --at-y place the window at a monitor's origin (visual fullscreen on that
    // display, since minifb has no fullscreen call of its own).
    let origin: Option<(isize, isize)> = match (map.get("at-x"), map.get("at-y")) {
        (Some(x), Some(y)) => Some((x.parse::<isize>()?, y.parse::<isize>()?)),
        _ => None,
    };

    let path = Path::new(input);
    let data = std::fs::read(path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("payload.bin");
    let opts = sendmecongo_core::PlayOptions {
        size,
        cycles,
        lanes: if lanes_arg > 0 { Some(lanes_arg) } else { None },
        origin,
        borderless,
        hide_cursor: false,
        topmost: true,
    };

    println!(
        "playing {} · {} · {} sym/s · ESC 停止",
        name, preset.name, preset.fps
    );
    let stats = sendmecongo_core::play(&data, name, &preset, &opts)?;
    println!(
        "window    {} × {} px · 每码 {}px · 留白 {}px · {} lane(s)",
        stats.window.0, stats.window.1, stats.tile, stats.pad, stats.lanes
    );
    println!(
        "emitted {} symbols in {:.1}s → {:.1} sym/s actual (target {} = {:.0}%)",
        stats.emitted,
        stats.wall_secs,
        stats.sym_per_sec,
        preset.fps,
        stats.sym_per_sec / preset.fps as f64 * 100.0
    );
    println!(
        "cycles   {:.2} · {} frames per cycle",
        stats.emitted as f64 / stats.frames_per_cycle as f64,
        stats.frames_per_cycle
    );
    if stats.kept_up(preset.fps) {
        println!(
            "WARNING  renderer fell short of the target rate — 实测吞吐不可信，\
             降低 --size 或降档后重测"
        );
    }
    Ok(())
}

/// Rebuild a file from the symbols that `tools/analyze.py --dump` recovered off a
/// camera recording. This closes the optical loop: screen -> camera -> RaptorQ -> file.
fn cmd_decode(map: HashMap<String, String>) -> BoxResult<()> {
    let input = flag(&map, "in", "");
    let out_dir = flag(&map, "out", "out/recv");
    if input.is_empty() {
        usage();
        return Ok(());
    }
    let dump = std::fs::read(input)?;

    // Dump format: repeated [u32 le length][frame payload].
    let mut symbols: Vec<Vec<u8>> = Vec::new();
    let mut cursor = 0usize;
    while cursor + 4 <= dump.len() {
        let len = u32::from_le_bytes([
            dump[cursor],
            dump[cursor + 1],
            dump[cursor + 2],
            dump[cursor + 3],
        ]) as usize;
        cursor += 4;
        if len == 0 || cursor + len > dump.len() {
            break;
        }
        symbols.push(dump[cursor..cursor + len].to_vec());
        cursor += len;
    }
    if symbols.is_empty() {
        return Err("dump contained no symbols — did the camera decode anything?".into());
    }

    let first = sendmecongo_core::frame::parse(&symbols[0])
        .map(|(h, _)| h)
        .ok();
    let source_symbols = first
        .map(|h| h.object_len.div_ceil(h.symbol_size as u32) as usize)
        .unwrap_or(0);
    println!("dump              {} ({} symbols)", input, symbols.len());
    if let Some(h) = &first {
        println!("session           0x{:08X}", h.session);
        println!("object            {} B / {} B symbols", h.object_len, h.symbol_size);
        println!("source symbols    {} (need ~{}, +repair)", source_symbols, source_symbols);
    }

    let mut receiver = Receiver::new();
    let mut rebuilt = None;
    for symbol in &symbols {
        if let Some(r) = receiver.push(symbol)? {
            rebuilt = Some(r);
            break;
        }
    }

    let Some(received) = rebuilt else {
        println!();
        println!("RESULT            INCOMPLETE");
        println!(
            "  got {} unique symbols · {} source symbols (K)",
            receiver.frames_used(),
            source_symbols
        );
        println!("  RaptorQ needs K' symbols, where K' >= K (set by RFC 6330 parameters).");
        println!("  Landing a few short of K is normal operating range, not a failure — the");
        println!("  stream loops, so the next cycle delivers the symbols that were missed.");
        println!("  → record ~2 more cycles. If it still never converges, one lane is being");
        println!("    lost entirely (framing or focus) — check codes/frame in the report.");
        return Ok(());
    };

    std::fs::create_dir_all(out_dir)?;
    let out_path = format!("{out_dir}/{}", received.name);
    std::fs::write(&out_path, &received.data)?;

    println!();
    println!("RESULT            OK");
    println!("file              {} ({} bytes)", received.name, received.data.len());
    println!("symbols used      {}", received.frames_used);
    println!(
        "overhead          {:.2}x",
        received.frames_used as f64 / source_symbols.max(1) as f64
    );
    println!("crc32             0x{:08X}", sendmecongo_core::crc32(&received.data));
    println!("written           {out_path}");

    if let Some(original) = map.get("compare") {
        let original = std::fs::read(original)?;
        println!(
            "compare           {}",
            if original == received.data {
                "IDENTICAL ✓".to_string()
            } else {
                format!("MISMATCH ({} vs {} bytes)", original.len(), received.data.len())
            }
        );
    }
    Ok(())
}

fn cmd_simulate(map: HashMap<String, String>) -> BoxResult<()> {
    let input = flag(&map, "in", "");
    let preset_name = flag(&map, "preset", "turbo30");
    if input.is_empty() {
        usage();
        return Ok(());
    }
    let preset =
        preset::by_name(preset_name).ok_or_else(|| format!("unknown preset: {preset_name}"))?;

    let path = Path::new(input);
    let data = std::fs::read(path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("payload.bin");

    let sender = Sender::new(name, &data, preset.symbol_size(), preset.repair_pct)?;
    let frames = sender.frames();
    let object_len = frames
        .first()
        .and_then(|f| sendmecongo_core::frame::parse(f).ok())
        .map(|(h, _)| h.object_len)
        .unwrap_or(0);
    let source_symbols = (object_len as usize).div_ceil(preset.symbol_size() as usize).max(1);

    let drops: Vec<u32> = flag(&map, "drops", "0,10,20,30,40")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    println!("file            {} ({} bytes)", name, data.len());
    println!("preset          {}", preset.name);
    println!("source symbols  {}", source_symbols);
    println!("frames/cycle    {}", frames.len());
    println!();
    println!(
        "{:>6} {:>10} {:>10} {:>10} {:>8} {:>10}",
        "drop%", "delivered", "displayed", "time(s)", "overhead", "result"
    );

    for drop in drops {
        let mut receiver = Receiver::new();
        let mut rng = Rng(0x2545_F491_4F6C_DD1D ^ drop as u64);
        let (mut delivered, mut displayed) = (0usize, 0usize);
        let limit = frames.len() * 40;
        let mut received = None;

        while displayed < limit && received.is_none() {
            let frame = &frames[displayed % frames.len()];
            displayed += 1;
            if rng.below(100) < drop as u64 {
                continue;
            }
            delivered += 1;
            received = receiver.push(frame)?;
        }

        let result = match &received {
            Some(r) if r.data == data && r.name == name => "OK",
            Some(_) => "MISMATCH",
            None => "TIMEOUT",
        }
        .to_string();
        let wall = displayed as f64 / preset.fps;
        let overhead = delivered as f64 / source_symbols as f64;
        println!(
            "{:>6} {:>10} {:>10} {:>10.1} {:>8.2} {:>10}",
            drop, delivered, displayed, wall, overhead, result
        );
    }
    println!("\noverhead = delivered frames / source symbols; 1.00 means zero waste.");
    Ok(())
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

#[allow(dead_code)]
fn _assert_preset_copy(_: Preset) {}

#[cfg(test)]
mod icon_tests {
    use super::*;

    /// The pair must be one drawing seen twice. The receiver at `x` has to look like the
    /// sender at `100 - x` everywhere, or the two icons drift apart over time.
    #[test]
    fn variants_are_mirrors() {
        for y in [18.0f32, 30.0, 42.0, 50.0, 58.0, 70.0, 82.0] {
            for step in 0..=100 {
                let x = step as f32;
                assert_eq!(
                    icon_part(IconVariant::Send, x, y),
                    icon_part(IconVariant::Recv, ICON_UNITS - x, y),
                    "send and recv disagree at ({x}, {y})"
                );
            }
        }
    }

    /// Where the three parts actually land, so a change to the layout constants has to be
    /// a deliberate one.
    #[test]
    fn parts_are_where_they_should_be() {
        let send = IconVariant::Send;
        assert_eq!(icon_part(send, 24.0, 50.0), IconPart::Source);
        assert_eq!(icon_part(send, 70.0, 50.0), IconPart::Core);
        // Between the core and the spill's upper edge, the spill still shows.
        assert_eq!(icon_part(send, 80.0, 35.0), IconPart::Spill);
        // Above the spill the slate base shows — that gap is what keeps the silhouette from
        // reading as "a wedge glued into a square".
        assert_eq!(icon_part(send, 70.0, 25.0), IconPart::Base);
        // Corners stay empty: the macOS silhouette is the squircle, not a square.
        assert_eq!(icon_part(send, 1.0, 1.0), IconPart::Outside);
        assert_eq!(icon_part(send, 99.0, 99.0), IconPart::Outside);
        // The source disc sits fully inside the silhouette, so it never gets clipped.
        assert!(sd_squircle(13.0, 38.0, ICON_RADIUS) < 0.0);
        assert!(sd_squircle(37.0, 62.0, ICON_RADIUS) < 0.0);
        // The two variants are told apart by hue, and only by hue.
        let (send, recv) = (send.colors(), IconVariant::Recv.colors());
        assert_ne!(send.source, recv.source);
        assert_ne!(send.core, recv.core);
        assert_ne!(send.spill, recv.spill);
    }

    /// The wash has to be asymmetric (top-left lighter than bottom-right) and small enough
    /// that it cannot show up as a grey edge when the icon is scaled to 16 px.
    #[test]
    fn highlight_is_subtle_and_directional() {
        let (mut top_left, mut bottom_right) = (ICON_BASE, ICON_BASE);
        apply_highlight(&mut top_left, 6.0, 6.0);
        apply_highlight(&mut bottom_right, 94.0, 94.0);
        assert!(top_left[2] > ICON_BASE[2]);
        assert!(bottom_right[2] < ICON_BASE[2]);
        for channel in 0..3 {
            assert!((top_left[channel] - ICON_BASE[channel]).abs() < 20.0);
            assert!((bottom_right[channel] - ICON_BASE[channel]).abs() < 20.0);
        }
    }

    #[test]
    fn variant_names_round_trip() {
        for variant in [IconVariant::Send, IconVariant::Recv] {
            assert_eq!(IconVariant::parse(variant.name()), Some(variant));
        }
        assert_eq!(IconVariant::parse("sender"), None);
        assert!(IconVariant::Send != IconVariant::Recv);
    }
}
