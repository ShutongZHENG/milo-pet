"""ping-pong 往返循环重排：frames.bin → 602 帧无缝循环包（零转码，zstd 块原样复用）。

播放顺序 f0→f(n-1)→f(n-2)→…→f1→(循环回 f0)：两处接缝都是原始相邻帧过渡，
消除首尾姿势不一致导致的循环跳变。302 帧原始包 → 602 帧 @30fps（循环约 20 秒）。

用法: python pingpong_pack.py <输入.bin> <输出.bin>
依赖: 无第三方（纯字节操作，不解压）
"""
import struct
import sys
from pathlib import Path


def main():
    src = Path(sys.argv[1])
    dst = Path(sys.argv[2])
    data = src.read_bytes()
    assert data[:8] == b"MPETFB01", "不是 milo-pet 帧包"
    n, w, h, fps = struct.unpack("<IIII", data[8:24])
    offs = struct.unpack(f"<{n}I", data[24 : 24 + 4 * n])

    # 逐帧完整块 = u32 长度前缀 + zstd 载荷（偏移表给出块起点）
    blobs = []
    for i, off in enumerate(offs):
        end = offs[i + 1] if i + 1 < n else len(data)
        blobs.append(data[off:end])

    # ping-pong：正放全片 + 倒放去首尾（首尾各去重一次）
    order = list(range(n)) + list(range(n - 2, 0, -1))

    header_size = 8 + 16 + 4 * len(order)
    pos = header_size
    new_offs = []
    for idx in order:
        new_offs.append(pos)
        pos += len(blobs[idx])

    with open(dst, "wb") as f:
        f.write(b"MPETFB01")
        f.write(struct.pack("<IIII", len(order), w, h, fps))
        for o in new_offs:
            f.write(struct.pack("<I", o))
        for idx in order:
            f.write(blobs[idx])

    print(f"frames {n} -> {len(order)} {w}x{h} @{fps}fps -> {dst} ({dst.stat().st_size / 1024 / 1024:.1f} MB)")
    print(f"order head: {order[:3]} ... turn: {order[n - 1:n + 2]} ... tail: {order[-3:]}")


main()
