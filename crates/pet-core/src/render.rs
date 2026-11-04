//! 渲染抽象：把一张预乘 BGRA 帧贴到屏幕。
//! 内核只依赖这个 trait；具体实现（分层窗口 / DirectComposition / GPU）放在 pet-render。

/// 渲染器：负责把帧素材画到桌面。
pub trait Renderer {
    /// 把一张预乘 BGRA 帧，按显示尺寸 (w, h) 贴到屏幕 (x, y)。
    ///
    /// * `bgra` —— 帧原始尺寸的预乘 BGRA 像素（构造渲染器时给定 frame_w×frame_h）。
    /// * `x`, `y`, `w`, `h` —— 目标位置与显示尺寸（物理像素，含 squash 缩放）。
    /// 缩放由实现负责（软件双线性 / GPU 采样）。
    fn present(&mut self, bgra: &[u8], x: i32, y: i32, w: i32, h: i32) -> Result<(), String>;
}
