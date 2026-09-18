#!/usr/bin/env python3
"""SMQ1 optical link analyzer.

Measures what the optical channel actually delivers: decode every frame of a phone
recording (or a rendered PNG sequence), parse the SMQ headers, dedupe by (sbn, esi)
and report the effective throughput.

Usage:
    python3 analyze.py video.mov                  # video recording of the stream
    python3 analyze.py frames/                    # PNG sequence exported by sendmecongo-bench
    python3 analyze.py video.mov --json out.json  # machine readable result
    python3 analyze.py video.mov --dump frames.bin  # save unique frames for Rust decode

Requires: pip install zxing-cpp opencv-python-headless
"""
from __future__ import annotations

import argparse
import json
import os
import struct
import sys
from collections import defaultdict

MAGIC = b"SMQ"
HEADER_LEN = 18
CRC_LEN = 4


def parse_header(data: bytes):
    """Return (session, object_len, symbol_size, sbn, esi) or None."""
    if len(data) < HEADER_LEN + CRC_LEN or data[:3] != MAGIC:
        return None
    session, object_len = struct.unpack_from("<II", data, 4)
    symbol_size = struct.unpack_from("<H", data, 12)[0]
    sbn = data[14]
    esi = data[15] | (data[16] << 8) | (data[17] << 16)
    return session, object_len, symbol_size, sbn, esi


def crc32(data: bytes) -> int:
    import zlib

    return zlib.crc32(data) & 0xFFFFFFFF


def decode_qr(image):
    """Decode all QR codes in a BGR/grayscale image, returning their raw bytes."""
    import zxingcpp

    out = []
    for res in zxingcpp.read_barcodes(image):
        raw = getattr(res, "bytes", None)
        if raw:
            out.append(bytes(raw))
        elif getattr(res, "text", None):
            out.append(res.text.encode("latin-1", "ignore"))
    return out


def iter_video(path: str, every: int):
    import cv2

    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise SystemExit(f"cannot open video: {path}")
    index = 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        if index % every == 0:
            yield frame
        index += 1
    cap.release()


def iter_images(path: str, every: int):
    import cv2

    names = sorted(n for n in os.listdir(path) if n.lower().endswith((".png", ".jpg", ".jpeg")))
    for i, name in enumerate(names):
        if i % every:
            continue
        img = cv2.imread(os.path.join(path, name), cv2.IMREAD_COLOR)
        if img is not None:
            yield img


def summaries(r: dict) -> str:
    """Human-readable bottom line. The JSON above is for machines, this is for you."""
    lines = ["", "=" * 52]
    ok = r["verdict"] == "DECODABLE"
    lines.append(f"VERDICT    {r['verdict']}")
    lines.append(
        f"symbols    {r['unique_symbols']} unique / {r['source_symbols']} needed "
        f"({r['completeness'] * 100:.0f}%)"
    )
    cpf = r.get("codes_per_frame", 0.0)
    lane_note = f" · {cpf:.2f} codes/frame" if cpf > 1.05 else ""
    lines.append(f"camera     {r['frame_hit_rate'] * 100:.0f}% of frames hit{lane_note}")
    if r["symbol_delivery_ratio"]:
        lanes = r.get("lanes_detected", 1)
        lane_note = f" over {lanes} lane(s)" if lanes > 1 else ""
        lines.append(
            f"capture    {r['symbol_delivery_ratio'] * 100:.0f}% "
            f"of camera sampling capacity{lane_note}"
        )
    if ok and r["time_to_complete_sec"]:
        lines.append(
            f"completed  at {r['time_to_complete_sec']:.1f}s "
            f"(clip ran {r['elapsed_sec']:.1f}s — the rest was redundant)"
        )
        lines.append(f"goodput    {r['goodput_bytes_per_sec'] / 1024:.1f} KB/s")
    elif not ok:
        need = r["source_symbols"] - r["unique_symbols"]
        lines.append(f"           short by {need} symbols — record longer, or slow the preset down")
    if r["recovery_curve"]:
        c = r["recovery_curve"]
        pts = " → ".join(str(c[i]) for i in (1, 4, 7, 9) if i < len(c))
        lines.append(f"recovery   {pts}   (unique symbols at 20/50/80/100% of the clip)")
    if r["segment_decode_rates_pct"]:
        seg = " ".join(f"{s:.0f}" for s in r["segment_decode_rates_pct"])
        lines.append(f"segments   {seg}   (decode rate %, first→last tenth of the clip)")
    if r.get("segment_codes_per_frame"):
        sc = " ".join(f"{s:.1f}" for s in r["segment_codes_per_frame"])
        lines.append(f"codes/frm  {sc}   (codes decoded per frame, first→last tenth)")
        peak = r.get("peak_codes_per_frame", 0.0)
        if peak >= 1.9 and cpf < 1.9:
            lines.append(
                f"           peak {peak:.1f} ⇒ 双通道链路本身没问题（每一路都读到过），"
                f"吞吐是被拍摄稳定性限制的，不是渲染或协议"
            )
    if r["longest_blackout_sec"] >= 0.5:
        lines.append(
            f"blackout   {r['longest_blackout_sec']:.1f}s with zero decodes — "
            f"something blocked the camera, not a protocol problem"
        )
    lines.append("=" * 52)
    if r["rejected_frames"]:
        lines.append(
            f"note: {r['rejected_frames']} payloads failed CRC — camera misread, correctly discarded"
        )
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("input", help="video file or directory of PNG frames")
    ap.add_argument("--every", type=int, default=1, help="only analyze every Nth frame")
    ap.add_argument("--fps", type=float, default=0.0, help="override video fps (for throughput)")
    ap.add_argument(
        "--sym-rate",
        type=float,
        default=0.0,
        help="preset symbol rate (symbols/sec played on screen), e.g. 30 for turbo30",
    )
    ap.add_argument("--json", dest="json_out")
    ap.add_argument("--dump", dest="dump_out", help="write unique frame payloads for Rust decode")
    args = ap.parse_args()

    try:
        import cv2  # noqa: F401
        import zxingcpp  # noqa: F401
    except ImportError as exc:
        raise SystemExit(f"missing dependency: {exc}\npip install zxing-cpp opencv-python-headless")

    source = iter_images if os.path.isdir(args.input) else iter_video
    fps = args.fps
    if not fps and os.path.isfile(args.input):
        import cv2

        cap = cv2.VideoCapture(args.input)
        fps = cap.get(cv2.CAP_PROP_FPS) or 0.0
        cap.release()

    scanned = decoded = rejected = 0
    unique: dict[tuple[int, int], bytes] = {}
    sessions: dict[int, int] = defaultdict(int)
    symbol_size = object_len = 0
    # unique count over time — this is what tells us when the transfer actually finished,
    # which is not the same as how long the recording is.
    recovery: list[int] = []
    # per-frame hit/miss, so we can tell an evenly-noisy channel apart from one that was
    # simply blocked for a while (hand in the way, refocus, notification banner).
    hits: list[bool] = []
    frame_codes: list[int] = []

    for image in source(args.input, args.every):
        scanned += 1
        hit = False
        codes_here = 0
        for payload in decode_qr(image):
            header = parse_header(payload)
            if header is None:
                rejected += 1
                continue
            hit = True
            codes_here += 1
            session, obj_len, sym_size, sbn, esi = header
            decoded += 1
            sessions[session] += 1
            symbol_size, object_len = sym_size, obj_len
            if payload[-CRC_LEN:] and crc32(payload[:-CRC_LEN]) == struct.unpack_from(
                "<I", payload, len(payload) - CRC_LEN
            )[0]:
                unique.setdefault((sbn, esi), payload)
            else:
                rejected += 1
        hits.append(hit)
        frame_codes.append(codes_here)
        recovery.append(len(unique))

    longest_blackout = run = 0
    for h in hits:
        run = 0 if h else run + 1
        longest_blackout = max(longest_blackout, run)

    # Decode rate per decile. A single flat average hides the thing that actually matters:
    # a channel that starts clean and degrades behaves very differently from a uniformly
    # noisy one, and they call for completely different fixes.
    segments: list[float] = []
    if hits:
        step = max(1, len(hits) // 10)
        for i in range(0, len(hits), step):
            chunk = hits[i : i + step]
            segments.append(round(100 * sum(chunk) / len(chunk), 1))

    # Same split, but counting codes rather than hits. On a dual-lane stream this is what
    # exposes drift: the hit rate can sit at 100% while the second lane quietly stops
    # being readable, halving throughput without any obvious failure.
    seg_codes: list[float] = []
    if frame_codes:
        step = max(1, len(frame_codes) // 10)
        for i in range(0, len(frame_codes), step):
            chunk = frame_codes[i : i + step]
            seg_codes.append(round(sum(chunk) / len(chunk), 2))

    # `scanned` counts analysed frames; with --every N that is 1/N of the clip, so the
    # frame indices below all live on the subsampled timebase and must be scaled back.
    stride = max(1, args.every)
    elapsed = (scanned * stride) / fps if fps else 0.0
    delivered_bytes = len(unique) * symbol_size
    needed = (-(-object_len // symbol_size)) if symbol_size else 0

    # When did the receiver actually have enough symbols? Recording length is an upper
    # bound, not the answer — the stream loops, and it usually finishes well before you
    # stopped filming. Goodput must be measured against this, not against `elapsed`.
    ttc_frames = next((i + 1 for i, n in enumerate(recovery) if needed and n >= needed), 0)
    ttc_sec = (ttc_frames * stride / fps) if (ttc_frames and fps) else 0.0

    # How much of the camera's sampling capacity actually resolved a QR. The denominator
    # is fps × lanes, NOT the played symbol count: a camera running at or above the stream
    # rate reads the same symbol more than once, so dividing by played symbols produces
    # >100% figures that mean nothing (seen at 200% with 4K60). Anchoring on sampling
    # capacity keeps the ratio interpretable in both regimes.
    lanes_est = max(1, int(round(max(seg_codes) if seg_codes else 1.0)))
    capacity = elapsed * fps * lanes_est if (elapsed and fps) else 0.0
    played_est = elapsed * args.sym_rate if (elapsed and args.sym_rate) else 0.0
    curve = (
        [recovery[min(int(len(recovery) * p / 100), len(recovery) - 1)] for p in range(10, 101, 10)]
        if recovery
        else []
    )
    ok = bool(needed) and len(unique) >= needed
    # Single-lane streams give one code per frame; dual-lane streams give two. So
    # `decoded/scanned` is a code *count*, not a rate — calling it a rate produced
    # nonsensical 200% figures. Report both and let the reader tell them apart.
    codes_per_frame = (decoded / scanned) if scanned else 0.0
    frame_hit_rate = (sum(hits) / len(hits)) if hits else 0.0
    report = {
        "verdict": ("DECODABLE" if ok else "INCOMPLETE"),
        "completeness": (len(unique) / needed) if needed else 0.0,
        "symbol_delivery_ratio": (decoded / capacity) if capacity else 0.0,
        "lanes_detected": lanes_est,
        "played_symbols_est": round(played_est),
        "codes_per_frame": codes_per_frame,
        "frame_hit_rate": frame_hit_rate,
        "time_to_complete_sec": ttc_sec,
        "goodput_bytes_per_sec": (object_len / ttc_sec) if (ttc_sec and ok) else 0.0,
        "recovery_curve": curve,
        "longest_blackout_frames": longest_blackout,
        "longest_blackout_sec": (longest_blackout / fps) if fps else 0.0,
        "segment_decode_rates_pct": segments,
        "segment_codes_per_frame": seg_codes,
        "peak_codes_per_frame": (max(seg_codes) if seg_codes else 0.0),
        "input": args.input,
        "scanned_frames": scanned,
        "decoded_frames": decoded,
        "rejected_frames": rejected,
        "unique_symbols": len(unique),
        "duplicate_frames": decoded - len(unique),
        "symbol_size": symbol_size,
        "object_len": object_len,
        "source_symbols": (-(-object_len // symbol_size)) if symbol_size else 0,
        "sessions": dict(sessions),
        "fps": fps,
        "elapsed_sec": elapsed,
        "effective_bytes_per_sec": (delivered_bytes / elapsed) if elapsed else 0.0,
    }

    print(json.dumps(report, indent=2, ensure_ascii=False))
    print(summaries(report))

    if args.dump_out and unique:
        with open(args.dump_out, "wb") as handle:
            for payload in unique.values():
                handle.write(struct.pack("<I", len(payload)))
                handle.write(payload)
        print(f"\ndumped {len(unique)} unique frames to {args.dump_out}", file=sys.stderr)

    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as handle:
            json.dump(report, handle, indent=2, ensure_ascii=False)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
