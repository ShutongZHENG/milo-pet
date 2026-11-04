"""生成 Tauri 图标（占位：暖橙圆形）。用户后续可替换成真实宠物形象。"""
from PIL import Image, ImageDraw
import os

OUT = "src-tauri/icons"
os.makedirs(OUT, exist_ok=True)


def make(size):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = size / 2.0
    r = size * 0.45
    d.ellipse([m - r, m - r, m + r, m + r], fill=(255, 143, 0, 255))
    hr = size * 0.16
    d.ellipse([m - hr * 1.6, m - hr * 1.6, m - hr * 0.4, m - hr * 0.4], fill=(255, 205, 130, 255))
    return img


for s in (32, 128, 256):
    make(s).save(os.path.join(OUT, f"{s}x{s}.png"))
make(256).save(os.path.join(OUT, "icon.png"))
make(256).save(os.path.join(OUT, "icon.ico"), sizes=[(32, 32), (64, 64), (128, 128), (256, 256)])
print("icons done:", os.listdir(OUT))
