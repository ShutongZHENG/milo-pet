# milo-pet

纯离线的 Windows 桌面宠物（Rust）：透明置顶分层窗口 + 帧序列动画 + 拖拽甩抛物理。

一只 AI 生成的约克夏，以逐像素 alpha 贴在桌面上，循环播放待机动画——左键可以拎起来甩出去，会重力下落、落地反弹、摩擦停稳、撞墙折返；点一下会 Q 弹一下；右键有菜单。

## 功能

- **透明置顶分层窗口**：`UpdateLayeredWindow` 逐像素 alpha 合成；不占任务栏、不抢焦点
- **帧序列动画**：`assets/frames.bin`（预乘 BGRA + zstd 逐帧压缩），230 帧 @ 24fps 循环播放
- **交互物理**：左键拖拽、松手甩抛（重力 2600 px/s²、反弹恢复 0.55、地面摩擦）、点击 Q 弹形变、右键菜单（回到初始位置 / 退出）
- **DPI 自适应**：per-monitor-aware v2，窗口宽度按系统 DPI 缩放
- **纯离线**：无网络请求，素材本地打包

## 快速开始

要求：Windows 10/11、Rust 工具链（MSVC 主机，需 Visual Studio Build Tools C++ 工作负载）。

```powershell
git clone https://github.com/ShutongZHENG/milo-pet.git
cd milo-pet
cargo build --release
cargo run --release
```

⚠️ **`assets/frames.bin` 不随仓库分发**（~36MB，被 `.gitignore` 忽略）。程序按相对路径加载它，必须从项目根运行。请用下方素材管线从你自己的绿幕视频生成，或手动放置兼容的帧包。

## 素材管线：绿幕 MP4 → frames.bin

`tools/mp4_to_frames.py` 把一段绿幕视频抠像打包成帧包：

```powershell
python tools\mp4_to_frames.py 源视频.mp4 assets\frames.bin
```

处理流程：ffmpeg 抽帧（rgb24 无损管道）→ HSV 色相抠像（只认绿色方向，蓝/白/红永不误抠）+ 去绿溢 → 水印矩形清除 → 预乘 alpha → LANCZOS 缩放 640×360 → 循环点扫描 + 尾部 9 帧混合过渡 → 钳制非法预乘（color ≤ alpha）→ 预乘 BGRA → zstd level 3 → 写 MPETFB01 帧包。

换素材时需要调整的参数（脚本头部常量）：

| 常量 | 说明 |
|---|---|
| `FFMPEG` / ffprobe 路径 | 本机 ffmpeg 可执行文件路径 |
| `WATERMARK_RECT` | 水印矩形 `(x0, y0, x1, y1)`，无水印填 `None` |
| fps 默认值 | argv[3]，默认 24——**写素材实际帧率**，写错会变速播放 |

依赖：`numpy`、`pillow`、`zstandard`、ffmpeg（PATH 或改常量）。

## 配置：assets/config.toml

```toml
width = 350     # 窗口宽度（逻辑像素，按 DPI 缩放；高度按素材 16:9 自动计算）
corner = "top-right"   # 初始位置：top-right / top-left / bottom-right / bottom-left
margin_x = 24   # 距工作区边缘的边距
margin_y = 100
```

## 帧包格式（MPETFB01，小端）

| 偏移 | 内容 |
|---|---|
| 0 | magic `"MPETFB01"` |
| 8 | u32 帧数 N / 宽 W / 高 H / 帧率 fps |
| 24 | N × u32 偏移表（每帧数据相对文件头） |
| … | 每帧：u32 压缩长度 + zstd 压缩的预乘 BGRA 像素（W×H×4 字节） |

不变式：每个颜色通道 ≤ alpha（预乘合法性，打包时已钳制）。

## 工具

- `tools/mp4_to_frames.py` — 绿幕 MP4 → frames.bin（见上）
- `tools/extract_frames.py` — frames.bin → PNG 序列 / GIF / MP4 预览（`python tools/extract_frames.py assets\frames.bin 输出目录`）
- `tools/pack_frames.py` — PNG 目录/webm → frames.bin（早期入口）

## 项目结构

```
milo-pet/
├── src/
│   ├── main.rs      # 入口、主循环、输入、菜单
│   ├── win32.rs     # 分层窗口 + 双线性缩放 + UpdateLayeredWindow
│   ├── frames.rs    # 帧包加载/按需 zstd 解压
│   ├── physics.rs   # 抛掷物理
│   └── config.rs    # config.toml 解析（缺省回落内置默认）
├── assets/          # config.toml（入库）+ frames.bin（自备）
└── tools/           # 素材管线与解包工具（Python）
```
