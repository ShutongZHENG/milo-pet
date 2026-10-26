"""把透明动画打包成 milo-pet 的帧包素材 assets/frames.bin。

输入可以是：
- 目录：直接读取其中全部 PNG（RGBA，任意尺寸，统一缩放为 640x360）
- webm：用 ffmpeg 抽帧（注意：本机 ffmpeg 解码 VP9-alpha 有 bug 会丢 alpha，
  推荐先走 pet-build 的 key_v2/fix_flash 管线得到 PNG 目录再打包）

帧包格式（小端）：
  magic  8B  = b"MPETFB01"
  u32        帧数 N
  u32        宽 W
  u32        高 H
  u32        帧率 fps
  N × u32    偏移表（每帧压缩数据相对文件头的偏移）
  N × (u32 压缩后字节数 + zstd 压缩的预乘 BGRA 像素)

像素为预乘 BGRA（UpdateLayeredWindow 直接可用），字节序 BGRA。

用法: python pack_frames.py <输入.webm|输入目录> [输出.bin]
依赖: numpy、Pillow、zstandard（目录输入无需 ffmpeg）
"""
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
import zstandard
from PIL import Image

FFMPEG = r"C:\Users\shutong\Developer\milo\.ffmpeg\901\ffmpeg-9.0.1-essentials_build\bin\ffmpeg.exe"
OUT_W, OUT_H = 640, 360


def main():
    src = Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\Users\shutong\.dsh\dsh-pet\pet\milo-animation\idle.webm")
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else r"C:\Users\shutong\Developer\milo\milo-pet\assets\frames.bin")
    cctx = zstandard.ZstdCompressor(level=3)
    offsets = []
    blobs = []
    frame_paths = []

    if src.is_dir():
        frame_paths = sorted(src.glob("*.png"))
    else:
        tmp = tempfile.TemporaryDirectory()
        td = Path(tmp.name)
        subprocess.run(
            [FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-i", str(src),
             str(td / "f_%04d.png")],
            check=True,
        )
        frame_paths = sorted(td.glob("*.png"))

    if not frame_paths:
        raise SystemExit("no frames found")

    for p in frame_paths:
        im = Image.open(p).convert("RGBA")
        arr = np.asarray(im, dtype=np.float32)
        r, g, b, a = arr[:, :, 0], arr[:, :, 1], arr[:, :, 2], arr[:, :, 3]
        af = a / 255.0
        # 去绿溢：所有半透明边缘像素的绿色通道压到红蓝最大值（狗的配色无绿色，安全）
        semi = (a > 0) & (a < 255)
        g = np.where(semi, np.minimum(g, np.maximum(r, b)), g)
        # 先预乘再缩小：透明像素 RGB 归零，LANCZOS 缩小时不会把绿幕残留色混进边缘
        pre = np.dstack([r * af, g * af, b * af, a])
        pre_img = Image.fromarray(pre.astype(np.uint8), "RGBA")
        if pre_img.size != (OUT_W, OUT_H):
            pre_img = pre_img.resize((OUT_W, OUT_H), Image.LANCZOS)
        r2 = np.asarray(pre_img, dtype=np.float32)
        # LANCZOS 震荡可能让 color > alpha（非法预乘）→ 钳制到 alpha，消除合成爆白
        a2 = r2[:, :, 3, None]
        r2[:, :, :3] = np.minimum(r2[:, :, :3], a2)
        r2 = r2.astype(np.uint8)
        # 预乘 RGBA -> 预乘 BGRA（UpdateLayeredWindow 直接可用）
        pm = np.dstack([r2[:, :, 2], r2[:, :, 1], r2[:, :, 0], r2[:, :, 3]])
        blobs.append(cctx.compress(pm.tobytes()))

    header_size = 8 + 16 + 4 * len(blobs)
    pos = header_size
    for blob in blobs:
        offsets.append(pos)
        pos += 4 + len(blob)

    with open(dst, "wb") as f:
        f.write(b"MPETFB01")
        f.write(struct.pack("<IIII", len(blobs), OUT_W, OUT_H, 30))
        for off in offsets:
            f.write(struct.pack("<I", off))
        for blob in blobs:
            f.write(struct.pack("<I", len(blob)))
            f.write(blob)

    total = sum(len(b) for b in blobs)
    print(f"frames={len(blobs)} size={OUT_W}x{OUT_H} fps=30")
    print(f"packed {dst} : {dst.stat().st_size / 1024 / 1024:.1f} MB (raw would be {len(blobs) * OUT_W * OUT_H * 4 / 1024 / 1024:.0f} MB)")


main()
