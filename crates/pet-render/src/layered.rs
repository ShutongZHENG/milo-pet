//! UpdateLayeredWindow 渲染器（档1 基线）：实现 pet_core::render::Renderer。
//! 后续档3 DirectComposition + D3D11 渲染器与本模块并列，共用同一 trait。

use pet_core::render::Renderer;
use windows::Win32::Foundation::HWND;

use crate::win32::PetWindow;

pub struct LayeredRenderer {
    window: PetWindow,
}

impl LayeredRenderer {
    pub fn new(
        display_w: i32,
        display_h: i32,
        frame_w: i32,
        frame_h: i32,
    ) -> Result<Self, String> {
        Ok(Self { window: PetWindow::create(display_w, display_h, frame_w, frame_h)? })
    }

    /// 原始窗口句柄（菜单、输入、生命周期管理用）。
    pub fn hwnd(&self) -> HWND {
        self.window.hwnd
    }
}

impl Renderer for LayeredRenderer {
    fn present(&mut self, bgra: &[u8], x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        self.window.blit(bgra, x, y, w, h)
    }
}
