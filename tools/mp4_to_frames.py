#!/usr/bin/env python3
"""mp4 绿幕视频 → milo-pet frames.bin（自包含）。

用法: python mp4_to_frames.py <输入.mp4> <输出 frames.bin> [fps=24] [W=640] [H=360]

依赖: numpy, pillow, zstandard, ffmpeg（PATH 或修改 FFMPEG 变量）
原理:
  1) ffmpeg 解出全部帧（rgb24，无损管道）
  2) HSV 色相抠像（只认"绿色方向"，蓝/白/红/棕永不误抠）+ 去绿溢
  3) 预乘 BGRA → LANCZOS 缩放 → 钳制 color<=alpha
  4) §2.3 循环点扫描（320x180 缩略帧两两差）+ 尾部 K=9 混合帧
  5) zstd 逐帧压缩，按 MPETFB01 格式写盘
"""
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
import zstandard
from PIL import Image

FFMPEG = r"C:\Users\Administrator\Developer\milo\.ffmpeg\ffmpeg-9.0.2-essentials_build\bin\ffmpeg.exe"
# 注意：不能用 FFMPEG.replace("ffmpeg", "ffprobe")——父目录名 ".ffmpeg" 也会被替换成
# ".ffprobe" 导致路径错误；只替换文件名 "ffmpeg.exe" 才安全
FFPROBE = FFMPEG.replace("ffmpeg.exe", "ffprobe.exe")

# ---- 抠像参数（HSV 色相判定，源自 dsh-pet scripts/chroma_step02.py）----
GREEN_HUE_MIN, GREEN_HUE_MAX = 70.0, 170.0   # 绿色色相带
SAT_MIN, VAL_MIN = 0.15, 0.15                # 饱和/明度下限（防灰白斑误抠）
HUE_FEATHER = 6.0                            # 色相边界羽化（度）
WATERMARK_RECT = (1064, 656, 1262, 696)      # 水印矩形 (x0, y0, x1, y1)，无水印填 None
                                              # 例: (1055, 650, 1270, 702)

def rgb_to_hsv(frame):
    """rgb24 (H,W,3) -> hue(0-360), sat(0-1), val(0-1)，numpy 向量化。"""
    r = frame[:, :, 0].astype(np.float32) / 255.0
    g = frame[:, :, 1].astype(np.float32) / 255.0
    b = frame[:, :, 2].astype(np.float32) / 255.0
    mx = np.maximum(np.maximum(r, g), b)
    mn = np.minimum(np.minimum(r, g), b)
    delta = mx - mn
    hue = np.zeros_like(mx)
    nz = delta > 0
    with np.errstate(divide="ignore", invalid="ignore"):
        hr = 60.0 * (((g - b) / np.where(nz, delta, 1)) % 6.0)
        hg = 60.0 * (((b - r) / np.where(nz, delta, 1)) + 2.0)
        hb = 60.0 * (((r - g) / np.where(nz, delta, 1)) + 4.0)
    hue = np.where((mx == r) & nz, hr,
                   np.where((mx == g) & nz, hg,
                            np.where((mx == b) & nz, hb, 0.0)))
    sat = np.where(mx > 0, delta / np.maximum(mx, 1e-6), 0.0)
    return hue, sat, mx

def key_frame(frame, lo, hi):
    """HSV 色相抠像 + 去绿溢 + 水印清除，返回直通 RGBA (H,W,4) float64。"""
    im = frame.astype(np.float64).copy()
    if WATERMARK_RECT:
        x0, y0, x1, y1 = WATERMARK_RECT
        im[y0:y1, x0:x1] = (0, 0, 0)  # 水印区涂黑（抹掉白色文字的颜色）
    hue, sat, val = rgb_to_hsv(im)
    in_hue = np.clip((hue - lo) / HUE_FEATHER, 0, 1) * np.clip((hi - hue) / HUE_FEATHER, 0, 1)
    bg = (sat >= SAT_MIN) & (val >= VAL_MIN) & (hue >= lo) & (hue <= hi)
    alpha = np.where(bg, (1.0 - in_hue) * 255, 255)
    if WATERMARK_RECT:
        # 修正（相对 video2frame §2.2 原文）：涂黑后 sat=0、val=0 不满足 bg 判定，
        # 原式会把水印区算成 alpha=255 的不透明黑块（与"无信息即透明"注释矛盾，
        # §4 验证第 4 条也会失败）。水印区无条件强制透明。
        alpha[y0:y1, x0:x1] = 0.0
    # 去绿溢：狗的配色绿色永非主导通道，此操作只清反绿光不伤本色
    r, g, b = im[:, :, 0], im[:, :, 1], im[:, :, 2]
    im[:, :, 1] = np.minimum(g, np.maximum(r, b))
    return np.dstack([im, alpha])

def encode_bgra(arr, cctx):
    """钳制 color<=alpha → 预乘 RGBA 转 BGRA → zstd（混合帧与主循环同一路径）。"""
    a = np.clip(arr.astype(np.float32), 0.0, 255.0)
    alpha_plane = a[:, :, 3, None]
    a[:, :, :3] = np.minimum(a[:, :, :3], alpha_plane)  # 钳制预乘合法性
    a = a.astype(np.uint8)
    # 预乘 RGBA -> 预乘 BGRA（帧包存储序）
    bgra = np.dstack([a[:, :, 2], a[:, :, 1], a[:, :, 0], a[:, :, 3]])
    return cctx.compress(bgra.tobytes())

def main():
    src = Path(sys.argv[1])
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else "frames.bin")
    fps = int(sys.argv[3]) if len(sys.argv) > 3 else 24
    out_w = int(sys.argv[4]) if len(sys.argv) > 4 else 640
    out_h = int(sys.argv[5]) if len(sys.argv) > 5 else 360

    lo, hi = GREEN_HUE_MIN - HUE_FEATHER, GREEN_HUE_MAX + HUE_FEATHER
    cctx = zstandard.ZstdCompressor(level=3)

    with tempfile.TemporaryDirectory() as td:
        # ① ffmpeg 抽帧（逐帧顺序，无损）
        proc = subprocess.Popen(
            [FFMPEG, "-loglevel", "error", "-i", str(src),
             "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
            stdout=subprocess.PIPE)
        # 用 ffprobe 拿分辨率（此处按 1280x720 例；通用做法自行 probe）
        probe = subprocess.run(
            [FFPROBE, "-v", "error", "-select_streams", "v:0",
             "-show_entries", "stream=width,height", "-of", "csv=s=x:p=0", str(src)],
            capture_output=True, text=True).stdout.strip().split("x")
        W, H = int(probe[0]), int(probe[1])

        blobs, offsets = [], []
        # §2.3 循环修复用：完整帧与缩略帧（与 blobs 同序）
        frames_full = []   # 预乘钳制后的 out_w x out_h RGBA uint8
        frames_thumb = []  # 320x180 预乘 RGBA float32（循环点扫描用）
        i = 0
        frame_size = W * H * 3
        while True:
            buf = proc.stdout.read(frame_size)
            if len(buf) < frame_size:
                break
            frame = np.frombuffer(buf, np.uint8).reshape(H, W, 3)

            # ② HSV 抠像（直通 RGBA）
            rgba = key_frame(frame, lo, hi)

            # ③ 预乘 → 缩放 → 钳制（顺序关键，见"坑"清单）
            a = rgba[:, :, 3] / 255.0
            pre = np.dstack([rgba[:, :, 0] * a, rgba[:, :, 1] * a,
                             rgba[:, :, 2] * a, rgba[:, :, 3]])
            img = Image.fromarray(pre.astype(np.uint8), "RGBA")
            if img.size != (out_w, out_h):
                img = img.resize((out_w, out_h), Image.LANCZOS)
            arr = np.asarray(img, dtype=np.float32)
            alpha_plane = arr[:, :, 3, None]
            arr[:, :, :3] = np.minimum(arr[:, :, :3], alpha_plane)  # 钳制预乘合法性
            arr = arr.astype(np.uint8)
            # 循环修复：保留完整帧与缩略帧（此后 arr 不再被修改）
            frames_full.append(arr)
            frames_thumb.append(np.asarray(
                Image.fromarray(arr, "RGBA").resize((320, 180), Image.LANCZOS),
                dtype=np.float32))
            # 预乘 RGBA -> 预乘 BGRA（帧包存储序）
            bgra = np.dstack([arr[:, :, 2], arr[:, :, 1], arr[:, :, 0], arr[:, :, 3]])
            blobs.append(cctx.compress(bgra.tobytes()))
            i += 1
            if i % 60 == 0:
                print(f"{i} frames")
        proc.stdout.close()
        proc.wait()

    # ---- §2.3 循环点扫描：|i-j| ∈ [180, min(281, N-1)] 内找差异最小的帧对 ----
    # 对 frames_thumb 两两求平均绝对差，裁出 frames_full[i..=j] 作为循环段，
    # 尾部追加 K=9 帧：t = k/(K+1)，premul RGB 线性混合（末帧→首帧），
    #   alpha = np.maximum(末帧 alpha, 首帧 alpha)（并集，防半透明斑块）
    N_src = len(frames_thumb)
    d_lo, d_hi = 180, min(281, N_src - 1)
    best = None  # (diff, i, j)
    if N_src > d_lo:
        stacked = np.stack(frames_thumb, axis=0)  # (N, 180, 320, 4) float32
        for d in range(d_lo, d_hi + 1):
            diffs = np.mean(np.abs(stacked[d:] - stacked[:-d]), axis=(1, 2, 3))
            k = int(np.argmin(diffs))
            cand = (float(diffs[k]), k, k + d)
            if best is None or cand[0] < best[0]:
                best = cand

    K = 9  # 尾部混合帧数
    if best is None:
        print(f"WARNING: |i-j| in [{d_lo}, {d_hi}] 内无合格帧对（N={N_src}），"
              f"回退：直接用全段、不做循环修复")
        i_loop, j_loop = 0, N_src - 1
        blend_blobs = []
    else:
        diff, i_loop, j_loop = best
        end = frames_full[j_loop].astype(np.float32)
        start = frames_full[i_loop].astype(np.float32)
        blend_blobs = []
        for k in range(1, K + 1):
            t = k / (K + 1)
            mixed = np.empty_like(end)
            mixed[:, :, :3] = (1.0 - t) * end[:, :, :3] + t * start[:, :, :3]
            mixed[:, :, 3] = np.maximum(end[:, :, 3], start[:, :, 3])
            blend_blobs.append(encode_bgra(mixed, cctx))
        print(f"loop point: (i, j)=({i_loop}, {j_loop}) diff={diff:.3f} "
              f"segment={j_loop - i_loop + 1} + {len(blend_blobs)} blend frames")
    # 循环段帧复用已压缩 blob（zstd 确定性，与重压缩一致），混合帧走同一路径
    blobs = blobs[i_loop:j_loop + 1] + blend_blobs

    # ④ 按 MPETFB01 格式写盘
    N = len(blobs)
    header_size = 8 + 16 + 4 * N
    pos = header_size
    offsets = []
    for b in blobs:
        offsets.append(pos)
        pos += 4 + len(b)

    with open(dst, "wb") as f:
        f.write(b"MPETFB01")
        f.write(struct.pack("<IIII", N, out_w, out_h, fps))
        for off in offsets:
            f.write(struct.pack("<I", off))
        for b in blobs:
            f.write(struct.pack("<I", len(b)))
            f.write(b)

    print(f"frames={N} {out_w}x{out_h} fps={fps} -> {dst} ({dst.stat().st_size/1024/1024:.1f} MB)")

main()
