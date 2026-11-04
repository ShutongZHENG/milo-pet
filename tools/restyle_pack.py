#!/usr/bin/env python3
"""把目标帧包的色调对齐到参考帧包（不经过视频源，直接改每一帧）。

用法:
    python restyle_pack.py [--ref assets/frames.bin] [--dst assets/sleep.bin]
                           [--out assets/sleep.bin] [--mode gain|hsl] [--dry-run]

原理:
  1) 从参考包与目标包各采样若干帧，只取 alpha>=250 的主体像素，
     统计每通道 mean/std（mode=gain）或亮度/饱和度（mode=hsl）。
  2) 逐帧：解压 → 反预乘成直通 RGBA → 应用色彩变换 → 钳制 → 重新预乘 BGRA
     → zstd(level 3) 打包，格式与 milo-pet 的 MPETFB01 完全一致。
  3) 只改像素，不动 count/width/height/fps，所以程序侧无需任何改动。
"""
import argparse
import shutil
import struct
import sys
from pathlib import Path

import numpy as np
import zstandard

MAGIC = b"MPETFB01"
SUBJECT_ALPHA = 250


# ---------------------------------------------------------------- pack I/O


def read_pack(path):
    d = Path(path).read_bytes()
    if len(d) < 24 or d[:8] != MAGIC:
        raise SystemExit(f"{path}: 不是 MPETFB01 帧包")
    count = struct.unpack_from("<I", d, 8)[0]
    width = struct.unpack_from("<I", d, 12)[0]
    height = struct.unpack_from("<I", d, 16)[0]
    fps = struct.unpack_from("<I", d, 20)[0]
    offs = [struct.unpack_from("<I", d, 24 + i * 4)[0] for i in range(count)]
    return d, count, width, height, fps, offs


def read_frame(data, off, width, height):
    """解压成预乘 BGRA (h,w,4) uint8。"""
    ln = struct.unpack_from("<I", data, off)[0]
    raw = zstandard.ZstdDecompressor().decompress(
        data[off + 4: off + 4 + ln], width * height * 4)
    return np.frombuffer(raw, np.uint8).reshape(height, width, 4)


def premul_to_straight(bgra):
    """预乘 BGRA -> 直通 RGBA float32（alpha==0 处 RGB 置 0）。"""
    a = bgra[:, :, 3].astype(np.float32)
    b, g, r = (bgra[:, :, i].astype(np.float32) for i in range(3))
    rgb = np.dstack([r, g, b])
    m = a > 0
    out = np.zeros_like(rgb)
    out[m] = rgb[m] * 255.0 / a[m][:, None]
    return np.clip(out, 0.0, 255.0), a


def straight_to_premul(rgba, a):
    """直通 RGBA float32 -> 预乘 BGRA uint8（保证 color <= alpha）。"""
    af = a / 255.0
    pm = rgba * af[:, :, None]
    pm = np.minimum(pm, a[:, :, None])          # 预乘合法性
    pm[a == 0] = 0.0                             # 全透明像素必须为 0
    out = np.stack([pm[:, :, 2], pm[:, :, 1], pm[:, :, 0], a], axis=2)
    return np.clip(out, 0, 255).astype(np.uint8)


def write_pack(path, frames, width, height, fps):
    cctx = zstandard.ZstdCompressor(level=3)
    blobs = [cctx.compress(f.tobytes()) for f in frames]
    count = len(blobs)
    pos = 8 + 16 + 4 * count
    offs = []
    for b in blobs:
        offs.append(pos)
        pos += 4 + len(b)
    with open(path, "wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<IIII", count, width, height, fps))
        for o in offs:
            f.write(struct.pack("<I", o))
        for b in blobs:
            f.write(struct.pack("<I", len(b)))
            f.write(b)


# ---------------------------------------------------------------- statistics


def collect(data, count, width, height, offs, sample):
    """采样主体像素，返回 (直通RGB列表)。"""
    chunks = []
    for i in range(0, count, sample):
        bgra = read_frame(data, offs[i], width, height)
        m = bgra[:, :, 3] >= SUBJECT_ALPHA
        if m.sum() < 500:
            continue
        rgba, _ = premul_to_straight(bgra)
        chunks.append(rgba[m])
    if not chunks:
        raise SystemExit("采样不到主体像素")
    return np.concatenate(chunks)


def gain_transform(src_px, ref_px):
    """每通道 mean/std 匹配 -> (gains, offsets)。"""
    gains, offs = [], []
    for c in range(3):
        ms, ss = src_px[:, c].mean(), src_px[:, c].std()
        mr, sr = ref_px[:, c].mean(), ref_px[:, c].std()
        k = (sr / ss) if ss > 1e-6 else 1.0
        k = float(np.clip(k, 0.5, 2.0))          # 防病态增益
        gains.append(k)
        offs.append(float(mr - k * ms))
    return np.array(gains, np.float32), np.array(offs, np.float32)


def luma_sat_params(src_px, ref_px):
    """亮度偏移 + 饱和度倍率（保持色相）。"""
    def ls(px):
        r, g, b = px[:, 0], px[:, 1], px[:, 2]
        y = 0.299 * r + 0.587 * g + 0.114 * b
        mx = px.max(axis=1)
        mn = px.min(axis=1)
        sat = np.where(mx > 1e-6, (mx - mn) / np.maximum(mx, 1e-6), 0.0)
        return y, sat
    ys, ss = ls(src_px)
    yr, sr = ls(ref_px)
    return float(yr.mean() - ys.mean()), float(sr.mean() / max(ss.mean(), 1e-6))


def apply_gain(rgba, gains, offs):
    return np.clip(rgba * gains + offs, 0.0, 255.0)


def apply_luma_sat(rgba, dy, k):
    r, g, b = rgba[:, :, 0], rgba[:, :, 1], rgba[:, :, 2]
    y = 0.299 * r + 0.587 * g + 0.114 * b
    y2 = y + dy
    out = np.empty_like(rgba)
    # out = y2 + k*(c - y)：保持色相，只改亮度与饱和度
    for c, ch in enumerate((r, g, b)):
        out[:, :, c] = y2 + k * (ch - y)
    return np.clip(out, 0.0, 255.0)


# ---------------------------------------------------------------- report


def fur_stats(rgba, alpha):
    """毛发区统计（暖色相 10-50°，饱和>0.25，不透明）。"""
    r, g, b = rgba[:, :, 0], rgba[:, :, 1], rgba[:, :, 2]
    mx = rgba.max(axis=2)
    mn = rgba.min(axis=2)
    d = mx - mn
    nz = d > 1e-6
    sat = np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-6), 0.0)
    hue = np.where((mx == r) & nz, 60 * (((g - b) / np.where(nz, d, 1)) % 6), 0.0)
    hue = np.where((mx == g) & nz, 60 * (((b - r) / np.where(nz, d, 1)) + 2), hue)
    hue = np.where((mx == b) & nz, 60 * (((r - g) / np.where(nz, d, 1)) + 4), hue)
    m = (alpha >= SUBJECT_ALPHA) & (hue > 10) & (hue < 50) & (sat > 0.25)
    if m.sum() < 200:
        return None
    return (r[m].mean(), g[m].mean(), b[m].mean(), sat[m].mean(), int(m.sum()))


def report(label, px):
    if px is None:
        print(f"  {label}: 主体像素不足")
        return
    r, g, b, s, n = px
    print(f"  {label}: R={r:6.1f} G={g:6.1f} B={b:6.1f}  sat={s:.3f}  (n={n})")


# ---------------------------------------------------------------- main


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", default="assets/frames.bin")
    ap.add_argument("--dst", default="assets/sleep.bin")
    ap.add_argument("--out", default="assets/sleep.bin")
    ap.add_argument("--mode", choices=["gain", "hsl"], default="gain")
    ap.add_argument("--sample", type=int, default=5)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    ref_path, dst_path, out_path = Path(args.ref), Path(args.dst), Path(args.out)
    ref = read_pack(ref_path)
    dst = read_pack(dst_path)

    if (ref[2], ref[3]) != (dst[2], dst[3]):
        raise SystemExit(f"尺寸不一致: ref {ref[2]}x{ref[3]} vs dst {dst[2]}x{dst[3]}")
    width, height = dst[2], dst[3]
    print(f"ref {ref_path}  n={ref[1]} {width}x{height} fps={ref[4]}")
    print(f"dst {dst_path}  n={dst[1]} {width}x{height} fps={dst[4]}")

    print("\n采样主体像素…")
    ref_px = collect(ref[0], ref[1], ref[2], ref[3], ref[5], sample=args.sample)
    dst_px = collect(dst[0], dst[1], dst[2], dst[3], dst[5], sample=args.sample)
    print(f"  ref  {ref_px.shape[0]} px   dst {dst_px.shape[0]} px")

    if args.mode == "gain":
        gains, offs = gain_transform(dst_px, ref_px)
        print(f"\n每通道增益 gains={gains.round(4).tolist()} offsets={offs.round(2).tolist()}")
        xform = lambda rgba: apply_gain(rgba, gains, offs)
    else:
        dy, k = luma_sat_params(dst_px, ref_px)
        print(f"\n亮度偏移 dy={dy:+.2f} 饱和度倍率 k={k:.4f}")
        xform = lambda rgba: apply_luma_sat(rgba, dy, k)

    # 变换前/后统计（用抖动采样帧）
    print("\n主体色彩（采样帧）:")
    samples = list(range(0, dst[1], max(1, dst[1] // 6)))[:6]
    before, after = [], []
    for i in samples:
        bgra = read_frame(dst[0], dst[5][i], width, height)
        rgba, a = premul_to_straight(bgra)
        before.append(fur_stats(rgba, a))
        after.append(fur_stats(xform(rgba), a))
    ref_stats = []
    for i in list(range(0, ref[1], max(1, ref[1] // 6)))[:6]:
        bgra = read_frame(ref[0], ref[5][i], width, height)
        rgba, a = premul_to_straight(bgra)
        ref_stats.append(fur_stats(rgba, a))

    def agg(lst):
        lst = [x for x in lst if x]
        if not lst:
            return None
        arr = np.array([x[:4] for x in lst], np.float32)
        return (arr[:, 0].mean(), arr[:, 1].mean(), arr[:, 2].mean(),
                arr[:, 3].mean(), int(sum(x[4] for x in lst)))

    report("frames.bin 参考 ", agg(ref_stats))
    report("sleep.bin  改前 ", agg(before))
    report("sleep.bin  改后 ", agg(after))

    if args.dry_run:
        print("\n--dry-run：未写入")
        return

    print(f"\n重打包 {dst[1]} 帧…")
    frames = []
    for i in range(dst[1]):
        bgra = read_frame(dst[0], dst[5][i], width, height)
        rgba, a = premul_to_straight(bgra)
        frames.append(straight_to_premul(xform(rgba), a))
        if (i + 1) % 50 == 0:
            print(f"  {i + 1}/{dst[1]}")

    backup = out_path.with_suffix(out_path.suffix + ".orig")
    if out_path == dst_path and not backup.exists():
        shutil.copy2(out_path, backup)
        print(f"原文件已备份 -> {backup}")
    write_pack(out_path, frames, width, height, dst[4])
    print(f"写入 {out_path}  {out_path.stat().st_size / 1024 / 1024:.1f} MB")


if __name__ == "__main__":
    main()
