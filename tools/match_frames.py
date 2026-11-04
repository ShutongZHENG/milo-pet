#!/usr/bin/env python3
"""找出 sleep.bin 各帧在 frames.bin 中最接近的帧（供切换对齐用）。

比较在 160x90 缩略图上进行：alpha 轮廓 + 预乘 RGB。
"""
import struct
from pathlib import Path

import numpy as np
import zstandard

PET = Path(__file__).resolve().parent.parent


def load(path):
    d = Path(path).read_bytes()
    n, w, h, fps = struct.unpack_from("<IIII", d, 8)
    offs = [struct.unpack_from("<I", d, 24 + i * 4)[0] for i in range(n)]
    return d, n, w, h, fps, offs


def thumbs(path, tw=160, th=90):
    d, n, w, h, fps, offs = load(path)
    out = np.empty((n, th, tw, 4), np.float32)
    dec = zstandard.ZstdDecompressor()
    sx, sy = w // tw, h // th
    for i in range(n):
        ln = struct.unpack_from("<I", d, offs[i])[0]
        raw = dec.decompress(d[offs[i] + 4: offs[i] + 4 + ln], w * h * 4)
        a = np.frombuffer(raw, np.uint8).reshape(h, w, 4).astype(np.float32)
        out[i] = a.reshape(th, sy, tw, sx, 4).mean(axis=(1, 3))
    return out


def main():
    print("解码缩略图…")
    ref = thumbs(PET / "assets" / "frames.bin")
    slp = thumbs(PET / "assets" / "sleep.bin")
    print(f"frames.bin {ref.shape}  sleep.bin {slp.shape}")

    # 归一化到 0..1
    ref /= 255.0
    slp /= 255.0

    rid = ref.reshape(ref.shape[0], -1)
    sid = slp.reshape(slp.shape[0], -1)

    # 每帧 sleep -> 最近的 frames 帧（欧氏距离，按像素平均）
    print("\nsleep 帧 -> frames.bin 最近帧:")
    best_all = (1e9, -1, -1)
    for i in range(0, slp.shape[0], 25):
        d = np.sqrt(((rid - sid[i]) ** 2).mean(axis=1))
        j = int(d.argmin())
        print(f"  sleep[{i:3d}] ↔ frames[{j:3d}]  rms={d[j]:.4f}")
        if d[j] < best_all[0]:
            best_all = (float(d[j]), i, j)

    # 全局最近的一对（分块计算，避免一次性分配 300x421x57600 的大数组）
    best = (1e9, -1, -1)
    total = 0.0
    cnt = 0
    for i in range(sid.shape[0]):
        d = np.sqrt(((rid - sid[i]) ** 2).mean(axis=1))
        j = int(d.argmin())
        if d[j] < best[0]:
            best = (float(d[j]), i, j)
        total += float(d.sum())
        cnt += d.size
    d_mean = total / cnt
    si, fi = best[1], best[2]
    print(f"\n全局最接近: sleep[{si}] ↔ frames[{fi}]  rms={best[0]:.4f}")
    print(f"（随机两帧的典型 rms ≈ {d_mean:.4f}）")
    d0 = np.sqrt(((rid - sid[0]) ** 2).mean(axis=1))
    order = np.argsort(d0)[:5]
    print(f"\nsleep[0] 在 frames.bin 中最接近的 5 帧: "
          + ", ".join(f"frames[{j}]({d0[j]:.4f})" for j in order))


main()
