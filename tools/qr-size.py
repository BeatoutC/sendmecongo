#!/usr/bin/env python3
"""量出录像画面里二维码的实际像素边长，据此反推发送端当时用的窗口尺寸。

    .venv/bin/python3 tools/qr-size.py recordings/VID01mp4.mp4 60 200 400

二维码符号本体的边长 = inner × (模块数 / 含静默区的模块数)。v40 = 177 / 185，
v30 = 137 / 145。反推：inner ≈ 量到的边长 × 185 / 177，再由 inner 和 pad = tile/20
回到 --size（megabit/turbo60 是双通道，tile = size / 2）。
"""
import math
import sys

import cv2
import zxingcpp


def corners(position):
    for name in ("top_left", "top_right", "bottom_right", "bottom_left"):
        point = getattr(position, name)
        yield (point.x, point.y)


def main() -> int:
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    path = sys.argv[1]
    wanted = [int(v) for v in sys.argv[2:]] or [60, 200, 400]

    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise SystemExit(f"打不开录像: {path}")
    width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
    print(f"{path}\n画面 {width}×{height} · {total} 帧")

    for index in wanted:
        cap.set(cv2.CAP_PROP_POS_FRAMES, index)
        ok, frame = cap.read()
        if not ok:
            print(f"  第 {index} 帧读不到")
            continue
        sizes = []
        for result in zxingcpp.read_barcodes(frame):
            points = list(corners(result.position))
            sizes.append((math.dist(points[0], points[1]), math.dist(points[0], points[3])))
        pretty = " · ".join(f"{w:.0f}×{h:.0f}" for w, h in sizes)
        print(f"  第 {index} 帧: {len(sizes)} 个码   {pretty}")
    cap.release()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
