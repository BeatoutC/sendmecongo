# SendMeCongo

[![CI](https://github.com/BeatoutC/sendmecongo/actions/workflows/ci.yml/badge.svg)](https://github.com/BeatoutC/sendmecongo/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

English · [简体中文](README.zh-CN.md)

**One-way optical file transfer for air-gapped machines.** The sender renders a
fountain-coded stream of QR codes on screen; any phone camera can record it, and a
single zero-dependency executable rebuilds the file — byte-for-byte identical.

No network. No USB. No Bluetooth. Nothing leaves the room except light.

```
file → compression (gzip/brotli, whichever is smaller) → SMC1 container
     → RaptorQ fountain coding (RFC 6330) → self-describing SMQ frames
     → QR codes, full-screen or dual-lane
                    ↓  one-way, lossy optical channel, no ACK
phone camera → recording → sendmecongo-recv → RaptorQ recovery
     → four layers of CRC → file on disk, verified identical
```

Built for getting documents, keys, configuration, logs, and small evidence
packages **out of** a physically isolated environment — in the direction the
screen is already allowed to face. See the [protocol spec](docs/PROTOCOL.md)
(SMQ1 v0.1) for the container and frame formats.

## Why not an existing project

Three closest open projects were evaluated: `deedy/qr-data-transfer` (no
LICENSE file at all), its MIT-tagged fork `qrferry` (inherits the unlicensed
code), and `decimen-optical-transfer` (conflicting MIT/AGPL claims, LT codes).
Protocol ideas were borrowed; **not a single line of their code is used** —
this codebase is clean-room and all-Rust.

## The three binaries

| Binary | Runs on | What it is |
|---|---|---|
| `sendmecongo-send` | the isolated machine | GUI + fullscreen player, one exe, two modes |
| `sendmecongo-recv` | the receiver's machine | standalone recovery tool, **zero dependencies** |
| `sendmecongo-bench` | dev box | capacity calibration, frame export, erasure simulation |

`sendmecongo-recv` needs nothing installed — no Python, no FFmpeg, no codec
pack, no internet. The macOS build is a 2.6 MB single-architecture (arm64)
`.app` whose only runtime dependencies are the system libraries
(`otool -L`: CoreFoundation, Cocoa, Metal, libc++, libSystem). It decodes the
HEVC or H.264 recording itself, recognizes the QR stream, and rebuilds the
file; it stops as soon as the fountain code has enough symbols, so you never
need to watch the whole recording.

Both GUIs and the CLI speak **Simplified Chinese (default), Traditional
Chinese, and English** — switchable live from the menu bar (macOS) or in-window
menu (Windows/Linux), or with `--lang` / `SENDMECONGO_LANG`.

## Quick start

**Sender** (the isolated machine):

```bash
cargo build --release -p sendmecongo-send
./target/release/sendmecongo-send          # GUI: drag file in, pick a preset, press play
```

**Receiver** (any other machine, e.g. where you copied the recording):

```bash
cargo build --release -p sendmecongo-recv
./target/release/sendmecongo-recv recording.mov --compare original-file
```

or on macOS just double-click the `.app` from the DMG and drag the recording
into the window. Progress, decoded frames, and symbol counts are visible the
whole time; the progress denominator is known from the first frame header, not
estimated.

Recommended recording setup (measured, not guessed): **turbo60 preset,
`--size 1600`, 4K60 recording, AE/AF locked, phone on a stand**. At the
resulting 100 KB/s: 1 MB ≈ 10 s, 10 MB ≈ 1.7 min, 64 MB ≈ 10 min. Files above
64 MB are rejected outright — the optical link and preparation memory both
make that a bad idea.

## Presets and measured throughput

Fountain coding means losses only cost time, never correctness: RaptorQ needs
`K'` symbols (slightly above K, fixed by RFC 6330 parameters) and the stream
loops until the camera has them. Overhead measured at 1.00x — collect K
symbols, get the file back, `IDENTICAL`, four CRC layers verified end to end.

| Preset | QR | Lanes | sym/s | Nominal | Effective (w/ 20% repair) |
|---|---|---|---|---|---|
| robust | V15-L | 1 | 10 | 4.9 KB/s | ~4 KB/s |
| balanced | V20-L | 1 | 15 | 12.2 KB/s | ~10 KB/s |
| turbo15 | V30-L | 1 | 15 | 25.0 KB/s | ~21 KB/s |
| turbo30 | V30-L | 1 | 30 | 50.1 KB/s | ~42 KB/s |
| **turbo60** | V30-L | 2 | 60 | 100.2 KB/s | ~84 KB/s |
| megabit | V40-L | 2 | 60 | **171.7 KB/s** | ~143 KB/s |

Measured on real hardware (incompressible random data, iPhone, AE/AF locked,
zero blackout; all runs `IDENTICAL ✓`):

| Preset | `--size` | Recording | goodput | % of nominal |
|---|---|---|---|---|
| turbo30 | 1200 | 4K30 | 49.9 KB/s | 99% |
| turbo60 | 1600 | 4K60 | **100.2 KB/s** | 100% |
| megabit | 1400 | 4K30 | 105.2 KB/s | 61% |

Notes from the measurement campaign:

- **turbo30 and turbo60 are saturated** — at their theoretical limit, because
  the nominal "effective" figure assumes a full loop including repair symbols.
- **turbo60 is the recommended default**: same throughput as megabit with
  larger modules (137 vs 177 per side), i.e. much more robust.
- **Dual-lane presets need high-framerate recording.** Camera fps must be ≥ 2×
  the per-lane update rate, or a phase beat between camera and lane switching
  periodically kills one lane (observed: 74.6 KB/s and `codes/frame` oscillating
  1.0↔2.0 at 4K30; back to 100.2 KB/s and a steady 2.0 at 4K60).
- Erasure simulation (256 KB, turbo30, K=154): 10–40% frame drops finish
  with overhead 1.00–1.55x and always intact.

## How receiving works

```
recording.mov/mp4
  ├─ in-house ISO-BMFF demuxer   hvc1 / hev1 / avc1, moov at head or tail
  ├─ pure-Rust decode            HEVC → rusty_h265    H.264 → openh264 (vendored, no cmake)
  ├─ QR recognition              rxing (pure-Rust ZXing port), QR only
  ├─ RaptorQ recovery            the shared sendmecongo-core Receiver, protocol untouched
  └─ stops when complete         the whole recording is never needed
```

Recordings are split at CRA keyframes (~0.9 s apart on phone HEVC) into
independent GOPs decoded in parallel. A GOP boundary is open (RASL frames
after it are dropped) — which is exactly what the fountain code's repair
symbols exist to absorb.

The GUI shows candidate recordings found next to the app and in the usual
places (home, Downloads, Desktop, Movies, mounted volumes), sorted newest
first; drag-and-drop into the window works too.

## Building

```bash
cargo build --release            # everything
```

- Playback **must** be release builds — debug rendering can't keep up and
  halves the symbol rate (measured).
- The toolchain is pinned in `rust-toolchain.toml` (1.98.1; `raptorq 2.0.1`
  needs rustc ≥ 1.89 for x86 targets).
- `sendmecongo-recv` defaults to including H.264 via vendored `openh264` C++
  (no cmake needed; NASM failure is soft — no SIMD, still works). Machines
  without a C++ compiler can build `--no-default-features` for HEVC-only,
  which is what phones actually record.
- Windows builds produce static-CRT single-file exes (see `.cargo/config.toml`).

Packaging (macOS): `./tools/build-macos.sh` builds the `.app`s and DMGs.
The DMG includes a first-open note and a one-liner quarantine-clearing
`.command`, because the app is ad-hoc signed and Gatekeeper will challenge it
after download (right-click → Open on older macOS; Settings → Privacy &
Security → Open Anyway on 15+; no challenge when copied from USB).

## Documentation

- [docs/PROTOCOL.md](docs/PROTOCOL.md) — SMC1 container and SMQ frame formats
- [docs/TESTING.md](docs/TESTING.md) — how to test the optical link on your
  own screen + phone, and how to read the metrics

## Known limitations

- macOS builds are Apple Silicon only (intentional — cross-compiling openh264
  for x86_64 isn't worth the complexity).
- Night Shift / blue-light filters shift the colors and break decoding; turn
  them off before playing.
- `rxing`'s multi-code detection occasionally reports only one of two
  side-by-side codes per frame; the receiver compensates with overlapping
  band rescans, but marginal recordings suffer first.
- `rusty_h265` supports Main / Main 10 / Main Still Picture, 4:2:0, ≤ 10-bit —
  exactly what phones record; anything outside is rejected loudly, never
  mis-decoded.

## Roadmap

- [ ] H.264 path end-to-end verification (all test recordings so far are HEVC;
      the openh264 branch passed compile + logic review only)
- [ ] Real-Windows-machine run of the packaged `.exe`
- [ ] megabit re-measured at `--size 1600` + 4K60 (105.2 KB/s was at 700 px
      per code and 4K30; headroom expected)
- [ ] M2 (v0.5.0, in progress): resumable transfer — receiver-side progress
      manifests merged across recordings, per-block symbol grid, and a
      type-in resume code so the sender replays only the deficit — plus an
      agent skill (SKILL.md + CLI/JSON contract) so an AI agent can drive
      send/receive end to end. See [docs/M2-DESIGN.md](docs/M2-DESIGN.md)
- [ ] M3: AES-256-GCM encryption envelope + audit log, multi-file batches,
      automated acceptance matrix

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option — the Rust ecosystem default.

---

The name is a nod to *"send me to the Congo"*: if you're stuck inside an
air gap, this is the boat out.
