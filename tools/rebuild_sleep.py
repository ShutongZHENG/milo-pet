#!/usr/bin/env python3
"""源 mp4 → 去水印/去绿幕/去白边/稳定地面对齐 → sleep.bin + 逐帧 PNG。

与 mp4_to_frames.py 的差异：
- PNG 存直通 RGBA（正确预览）；sleep.bin 存预乘 BGRA（渲染用）
- defringe：半透明边缘向实心区取色，消白边
- isolate_main_subject：去掉抠像碎点
- 地面对齐用 alpha>=128 的最低点，忽略软阴影，避免逐帧抖动
- 无循环修复 / 无 pingpong / 无淡化

用法: python rebuild_sleep.py [src.mp4]
依赖: numpy, pillow, zstandard, ffmpeg
"""
import struct
import subprocess
import sys
from collections import deque
from pathlib import Path

import numpy as np
import zstandard
from PIL import Image

FFMPEG = r"C:\Users\Administrator\Developer\milo\.ffmpeg\ffmpeg-9.0.2-essentials_build\bin\ffmpeg.exe"
FFPROBE = FFMPEG.replace("ffmpeg.exe", "ffprobe.exe")
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
    r"C:\Users\Administrator\Developer\milo\video\qwen_sleep.mp4"
)
DST_BIN = Path("assets/sleep.bin")
FRAME_DIR = Path("assets/sleep_frames")
FPS, OUT_W, OUT_H = 30, 640, 360
TARGET_BOTTOM = 352
WATERMARK_RECT = (1026, 604, 1251, 692)
GROUND_ALPHA = 128

GREEN_HUE_MIN, GREEN_HUE_MAX = 70.0, 170.0
SAT_MIN, VAL_MIN = 0.15, 0.15
HUE_FEATHER = 6.0


def rgb_to_hsv(frame):
    r = frame[:, :, 0].astype(np.float32) / 255.0
    g = frame[:, :, 1].astype(np.float32) / 255.0
    b = frame[:, :, 2].astype(np.float32) / 255.0
    mx = np.maximum(np.maximum(r, g), b)
    mn = np.minimum(np.minimum(r, g), b)
    delta = mx - mn
    nz = delta > 0
    with np.errstate(divide="ignore", invalid="ignore"):
        hr = 60.0 * (((g - b) / np.where(nz, delta, 1)) % 6.0)
        hg = 60.0 * (((b - r) / np.where(nz, delta, 1)) + 2.0)
        hb = 60.0 * (((r - g) / np.where(nz, delta, 1)) + 4.0)
    hue = np.where(
        (mx == r) & nz, hr, np.where((mx == g) & nz, hg, np.where((mx == b) & nz, hb, 0.0))
    )
    sat = np.where(mx > 0, delta / np.maximum(mx, 1e-6), 0.0)
    return hue, sat, mx


def key_frame(frame, lo, hi):
    """直通 RGBA + 去绿溢 + 水印清除。"""
    im = frame.astype(np.float64).copy()
    x0, y0, x1, y1 = WATERMARK_RECT
    im[y0:y1, x0:x1] = (0, 0, 0)
    hue, sat, val = rgb_to_hsv(im)
    in_hue = np.clip((hue - lo) / HUE_FEATHER, 0, 1) * np.clip(
        (hi - hue) / HUE_FEATHER, 0, 1
    )
    bg = (sat >= SAT_MIN) & (val >= VAL_MIN) & (hue >= lo) & (hue <= hi)
    alpha = np.where(bg, (1.0 - in_hue) * 255, 255)
    alpha[y0:y1, x0:x1] = 0.0
    r, g, b = im[:, :, 0], im[:, :, 1], im[:, :, 2]
    im[:, :, 1] = np.minimum(g, np.maximum(r, b))
    return np.dstack([im, alpha])


def defringe(rgba, rounds=3):
    """边缘色污染清除：半透明像素 RGB 用最近实心区颜色替换，并压掉过亮毛边。"""
    out = rgba.astype(np.float32).copy()
    alpha = out[:, :, 3].copy()
    # 极低 alpha 噪声直接去掉（闪点）
    alpha[alpha < 6] = 0
    out[:, :, 3] = alpha
    edge = (alpha > 0) & (alpha < 255)
    if not edge.any():
        return out
    rgb = out[:, :, :3]
    h, w = alpha.shape
    for _ in range(rounds):
        acc = np.zeros_like(rgb)
        cnt = np.zeros_like(alpha)
        for dy, dx in (
            (0, 1),
            (0, -1),
            (1, 0),
            (-1, 0),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ):
            sy0, sy1 = max(0, dy), h + min(0, dy)
            sx0, sx1 = max(0, dx), w + min(0, dx)
            ty0, ty1 = max(0, -dy), h + min(0, -dy)
            tx0, tx1 = max(0, -dx), w + min(0, -dx)
            na = alpha[sy0:sy1, sx0:sx1]
            nr = rgb[sy0:sy1, sx0:sx1]
            solid = na >= 248
            acc[ty0:ty1, tx0:tx1] += nr * solid[..., None]
            cnt[ty0:ty1, tx0:tx1] += solid
        fill = edge & (cnt > 0)
        if fill.any():
            rgb[fill] = acc[fill] / cnt[fill][:, None]
    # 软边亮度钳制：防止反预乘/振铃导致的白圈
    edge = (out[:, :, 3] > 0) & (out[:, :, 3] < 200)
    if edge.any():
        # 用邻域实心均值做上限
        acc = np.zeros_like(rgb)
        cnt = np.zeros_like(alpha)
        a = out[:, :, 3]
        for dy, dx in ((0, 1), (0, -1), (1, 0), (-1, 0), (1, 1), (1, -1), (-1, 1), (-1, -1)):
            sy0, sy1 = max(0, dy), h + min(0, dy)
            sx0, sx1 = max(0, dx), w + min(0, dx)
            ty0, ty1 = max(0, -dy), h + min(0, -dy)
            tx0, tx1 = max(0, -dx), w + min(0, -dx)
            na = a[sy0:sy1, sx0:sx1]
            nr = rgb[sy0:sy1, sx0:sx1]
            solid = na >= 248
            acc[ty0:ty1, tx0:tx1] += nr * solid[..., None]
            cnt[ty0:ty1, tx0:tx1] += solid
        has = edge & (cnt > 0)
        if has.any():
            ref = acc[has] / cnt[has][:, None]
            # 不超过参考色 + 小余量，避免白圈
            rgb[has] = np.minimum(rgb[has], ref + 12)
    return out


def isolate_main_subject(arr, tiny=24):
    """去掉抠像噪点：只删除面积 < tiny 的碎点（保留 Z 气泡等大块）。"""
    alpha = arr[:, :, 3]
    mask = alpha > 24
    h, w = mask.shape
    visited = mask.copy()
    out = arr.copy()
    ys, xs = np.where(mask)
    for y0, x0 in zip(ys, xs):
        if not visited[y0, x0]:
            continue
        q = deque([(y0, x0)])
        visited[y0, x0] = False
        pix = []
        while q:
            y, x = q.popleft()
            pix.append((y, x))
            for ny, nx in ((y - 1, x), (y + 1, x), (y, x - 1), (y, x + 1)):
                if 0 <= ny < h and 0 <= nx < w and visited[ny, nx]:
                    visited[ny, nx] = False
                    q.append((ny, nx))
        if len(pix) < tiny:
            for y, x in pix:
                out[y, x] = 0
    return out


def align_to_ground(straight):
    a = straight[:, :, 3]
    ys, _ = np.where(a >= GROUND_ALPHA)
    if not len(ys):
        return straight, 0
    dy = TARGET_BOTTOM - int(ys.max())
    if dy == 0:
        return straight, 0
    out = np.zeros_like(straight)
    h = straight.shape[0]
    if dy > 0:
        out[dy:] = straight[: h - dy]
    else:
        k = -dy
        out[: h - k] = straight[k:]
    return out, dy


def to_premul_bgra(straight):
    a = straight[:, :, 3:4].astype(np.float32) / 255.0
    rgb = straight[:, :, :3].astype(np.float32) * a
    rgb = np.minimum(rgb, straight[:, :, 3:4].astype(np.float32))
    return np.dstack(
        [rgb[:, :, 2], rgb[:, :, 1], rgb[:, :, 0], straight[:, :, 3]]
    ).astype(np.uint8)


def main():
    lo = GREEN_HUE_MIN - HUE_FEATHER
    hi = GREEN_HUE_MAX + HUE_FEATHER
    cctx = zstandard.ZstdCompressor(level=3)

    probe = subprocess.run(
        [
            FFPROBE,
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=s=x:p=0",
            str(SRC),
        ],
        capture_output=True,
        text=True,
    ).stdout.strip().split("x")
    W, H = int(probe[0]), int(probe[1])
    print(f"source {W}x{H} <- {SRC}")

    proc = subprocess.Popen(
        [
            FFMPEG,
            "-loglevel",
            "error",
            "-i",
            str(SRC),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ],
        stdout=subprocess.PIPE,
    )

    FRAME_DIR.mkdir(parents=True, exist_ok=True)
    for p in FRAME_DIR.glob("*.png"):
        p.unlink()

    blobs = []
    shifts = []
    i = 0
    frame_size = W * H * 3
    while True:
        buf = proc.stdout.read(frame_size)
        if len(buf) < frame_size:
            break
        frame = np.frombuffer(buf, np.uint8).reshape(H, W, 3)
        rgba = key_frame(frame, lo, hi)
        # 预乘后缩放（直通 RGBA 直接 LANCZOS 会产生白边振铃）
        a = rgba[:, :, 3] / 255.0
        premul = np.dstack(
            [rgba[:, :, 0] * a, rgba[:, :, 1] * a, rgba[:, :, 2] * a, rgba[:, :, 3]]
        )
        img = Image.fromarray(np.clip(premul, 0, 255).astype(np.uint8), "RGBA")
        if img.size != (OUT_W, OUT_H):
            img = img.resize((OUT_W, OUT_H), Image.LANCZOS)
        pm = np.asarray(img).astype(np.float32)
        # 钳制预乘合法性，再转回直通做 defringe / 对齐 / PNG
        pm[:, :, :3] = np.minimum(pm[:, :, :3], pm[:, :, 3:4])
        alpha = pm[:, :, 3]
        straight = pm.copy()
        straight[:, :, :3] = np.where(
            alpha[..., None] > 0,
            pm[:, :, :3] * 255.0 / np.maximum(alpha, 1.0)[..., None],
            0,
        )
        straight = defringe(straight, rounds=2)
        straight = isolate_main_subject(straight.astype(np.uint8)).astype(np.float32)
        straight, dy = align_to_ground(straight)
        shifts.append(dy)
        arr = np.clip(straight, 0, 255).astype(np.uint8)

        Image.fromarray(arr, "RGBA").save(FRAME_DIR / f"f{i:03d}.png")
        blobs.append(cctx.compress(to_premul_bgra(arr).tobytes()))
        i += 1
        if i % 60 == 0:
            print(f"{i} frames (last dy={dy:+d})")

    proc.stdout.close()
    proc.wait()
    N = len(blobs)

    header_size = 8 + 16 + 4 * N
    pos = header_size
    offsets = []
    for b in blobs:
        offsets.append(pos)
        pos += 4 + len(b)
    with open(DST_BIN, "wb") as f:
        f.write(b"MPETFB01")
        f.write(struct.pack("<IIII", N, OUT_W, OUT_H, FPS))
        for off in offsets:
            f.write(struct.pack("<I", off))
        for b in blobs:
            f.write(struct.pack("<I", len(b)))
            f.write(b)

    shifts = np.array(shifts)
    d = np.abs(np.diff(shifts)) if N > 1 else np.array([0])
    print(
        f"frames={N} dy min/max/mean={shifts.min()}/{shifts.max()}/{shifts.mean():.1f} "
        f"step_max={d.max()} step>2={(d > 2).sum()}"
    )
    print(f"sleep.bin -> {DST_BIN} ({DST_BIN.stat().st_size / 1024 / 1024:.1f} MB)")
    print(f"PNG -> {FRAME_DIR}/ ({N} files)")

    bad_ground = 0
    bad_edge = 0
    for j in range(N):
        arr = np.array(Image.open(FRAME_DIR / f"f{j:03d}.png"))
        a = arr[:, :, 3].astype(np.int32)
        ys, _ = np.where(a >= GROUND_ALPHA)
        if len(ys) and abs(int(ys.max()) - TARGET_BOTTOM) > 2:
            bad_ground += 1
        edge = (a > 0) & (a < 40)
        if edge.any():
            white_low = (
                edge
                & (arr[:, :, 0] > 200)
                & (arr[:, :, 1] > 200)
                & (arr[:, :, 2] > 200)
            )
            if white_low.sum() > 30:
                bad_edge += 1
    print(f"ground misaligned: {bad_ground}/{N}, white-fringe frames: {bad_edge}/{N}")


if __name__ == "__main__":
    main()
