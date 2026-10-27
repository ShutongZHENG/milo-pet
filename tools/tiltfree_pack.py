"""pong 段去除歪头：421 帧无缝循环包（正放全片 + 去歪头倒放 + 6 帧交叉淡化）。

帧序（源 = 302 帧原始包，f000..f301）：
  正放  f000 → f301            全片，含歪头（需求只要求倒放段不重播）
  倒放  f300 → f246            坐定段倒放
  混合  6 帧 f246 ⇝ f058       预乘 BGRA 全通道线性混合（t = 1/7..6/7，端点不重复）
  倒放  f058 → f001            微晃段倒放，全程正头
  循环  → f000

歪头量化（本素材实测）：左歪 f060~f164（峰值 -23°）、右歪 f168~f237（峰值 +14.5°）；
倒放段跳过 f165~f245（歪头区），故 pong 段不重播歪头。

零转码：源帧 zstd 块原样复用，仅 6 张混合帧新压缩（教训：新帧必须写 4 字节长度前缀；
通道序保持 BGRA 不翻转，否则狗变蓝）。

用法: python tiltfree_pack.py <原始302帧.bin> <输出.bin>
依赖: numpy, zstandard
"""
import struct
import sys
from pathlib import Path

import numpy as np
import zstandard

# 混合过渡的两个端点（去歪头分割点，源自逐帧歪头量化）
BLEND_FROM = 246  # 坐定段倒放的末帧
BLEND_TO = 58  # 微晃段倒放的首帧
BLEND_N = 6  # 交叉淡化帧数


def main():
    src = Path(sys.argv[1])
    dst = Path(sys.argv[2])
    data = src.read_bytes()
    assert data[:8] == b"MPETFB01", "不是 milo-pet 帧包"
    n, w, h, fps = struct.unpack("<IIII", data[8:24])
    offs = struct.unpack(f"<{n}I", data[24 : 24 + 4 * n])
    assert n == 302 and (w, h) == (640, 360), f"预期 302 帧 640x360，实际 {n} {w}x{h}"

    blobs = []
    for i, off in enumerate(offs):
        end = offs[i + 1] if i + 1 < n else len(data)
        blobs.append(data[off:end])

    dctx = zstandard.ZstdDecompressor()
    cctx = zstandard.ZstdCompressor(level=3)

    def decode(i):
        raw = dctx.decompress(blobs[i][4:], w * h * 4)
        return np.frombuffer(raw, np.uint8).reshape(h, w, 4).copy()

    # 6 张混合帧：预乘 BGRA 全通道线性混合（不动通道序，绝不转 RGBA）
    a = decode(BLEND_FROM)
    b = decode(BLEND_TO)
    blend_blobs = []
    for i in range(BLEND_N):
        t = (i + 1) / (BLEND_N + 1)
        mixed = np.rint((1.0 - t) * a.astype(np.float32) + t * b.astype(np.float32)).astype(np.uint8)
        # 预乘不变式防御：color ≤ alpha（线性混合理论上保持，取整后兜底钳制）
        mixed[:, :, :3] = np.minimum(mixed[:, :, :3], mixed[:, :, 3:4])
        # 教训：新合成帧必须写 4 字节长度前缀，与源帧块格式一致（否则偏移解析错位）
        payload = cctx.compress(mixed.tobytes())
        blend_blobs.append(struct.pack("<I", len(payload)) + payload)

    # 帧序：正放全片 + 坐定倒放 + 6 混合 + 微晃倒放
    order = []
    order += list(range(0, n))  # f000..f301
    order += list(range(n - 2, BLEND_FROM - 1, -1))  # f300..f246
    blend_slot = len(order)  # 混合帧在输出中的起始下标
    order += [None] * BLEND_N  # 占位，写盘时用 blend_blobs
    order += list(range(BLEND_TO, 0, -1))  # f058..f001（含 f058 本身，58 帧）

    # 组装输出块（每帧 = u32 长度前缀 + 载荷；混合帧是新压的，源帧原样复用）
    out_blobs = []
    for idx in order:
        if idx is None:
            out_blobs.append(None)  # 占位，下面填
        else:
            out_blobs.append(blobs[idx])
    for j in range(BLEND_N):
        out_blobs[blend_slot + j] = blend_blobs[j]

    header_size = 8 + 16 + 4 * len(out_blobs)
    pos = header_size
    new_offs = []
    for blob in out_blobs:
        new_offs.append(pos)
        pos += len(blob)

    with open(dst, "wb") as f:
        f.write(b"MPETFB01")
        f.write(struct.pack("<IIII", len(out_blobs), w, h, fps))
        for o in new_offs:
            f.write(struct.pack("<I", o))
        for blob in out_blobs:
            f.write(blob)

    print(
        f"frames {n} -> {len(out_blobs)} {w}x{h} @{fps}fps -> {dst} "
        f"({dst.stat().st_size / 1024 / 1024:.1f} MB); blend slots [{blend_slot}..{blend_slot + BLEND_N - 1}]"
    )


main()
