"""frames.bin 解包/预览工具。

用法：
  python extract_frames.py [frames.bin] [输出目录]
    - 把全部帧解出为 PNG（RGBA 透明），存到输出目录
  python extract_frames.py --gif [frames.bin] [输出.gif]
    - 导出 GIF 预览（合成浅色背景，320x180）
  python extract_frames.py --mp4 [frames.bin] [输出.mp4]
    - 导出 MP4 预览（合成白色背景，640x360，需 ffmpeg）

帧包格式见 pack_frames.py 头部注释。
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


def load(path: Path):
    data = path.read_bytes()
    assert data[:8] == b"MPETFB01", "不是 milo-pet 帧包"
    count, w, h, fps = struct.unpack("<IIII", data[8:24])
    offsets = struct.unpack(f"<{count}I", data[24 : 24 + count * 4])
    dctx = zstandard.ZstdDecompressor()
    frames = []
    for off in offsets:
        ln = struct.unpack("<I", data[off : off + 4])[0]
        raw = dctx.decompress(data[off + 4 : off + 4 + ln], w * h * 4)
        arr = np.frombuffer(raw, np.uint8).reshape(h, w, 4)
        # 预乘 BGRA -> 非预乘 RGBA
        a = arr[:, :, 3].astype(np.float32)
        out = np.zeros_like(arr)
        nz = a > 0
        for c in range(3):
            ch = arr[:, :, 2 - c].astype(np.float32)
            out[:, :, c] = np.where(nz, np.clip(ch * 255.0 / np.maximum(a, 1), 0, 255), 0)
        out[:, :, 3] = a.astype(np.uint8)
        frames.append(Image.fromarray(out, "RGBA"))
    return frames, fps


def main():
    args = sys.argv[1:]
    mode = "png"
    if args and args[0] in ("--gif", "--mp4"):
        mode = args.pop(0)[2:]
    src = Path(args[0]) if args else Path(r"C:\Users\shutong\Developer\milo\milo-pet\assets\frames.bin")
    dst = Path(args[1]) if len(args) > 1 else src.with_suffix("")

    frames, fps = load(src)
    print(f"帧包：{len(frames)} 帧 {frames[0].size[0]}x{frames[0].size[1]} @ {fps}fps")

    if mode == "png":
        dst.mkdir(parents=True, exist_ok=True)
        for i, im in enumerate(frames):
            im.save(dst / f"f_{i:04d}.png")
        print(f"已解出 {len(frames)} 张 PNG -> {dst}")
        return

    # gif / mp4：合成浅色背景
    bg = Image.new("RGB", frames[0].size, (245, 245, 248))
    composited = []
    for im in frames:
        c = bg.copy()
        c.paste(im, (0, 0), im)
        composited.append(c)

    if mode == "gif":
        small = [c.resize((320, 180), Image.LANCZOS) for c in composited]
        small[0].save(
            dst,
            save_all=True,
            append_images=small[1:],
            duration=int(1000 / fps),
            loop=0,
            optimize=True,
        )
        print(f"GIF 预览 -> {dst}")
    else:
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            for i, c in enumerate(composited):
                c.save(td / f"f_{i:04d}.png")
            subprocess.run(
                [FFMPEG, "-hide_banner", "-loglevel", "error", "-y",
                 "-framerate", str(fps), "-i", str(td / "f_%04d.png"),
                 "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20",
                 "-movflags", "+faststart", str(dst)],
                check=True,
            )
        print(f"MP4 预览 -> {dst}")


main()
