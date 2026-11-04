//! DirectComposition + D3D11 + Direct2D 渲染器（档3）。
//! GPU 合成精灵：帧上传为 D2D 位图（GPU 双线性缩放），swapchain Present（vsync），
//! DirectComposition 合成到桌面。通过 `MILO_RENDERER=dc` 启用（默认仍是分层窗口基线）。

use std::ffi::c_void;

use pet_core::render::Renderer;
use windows::core::{w, Interface};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    ID2D1Bitmap, ID2D1Factory,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, ID3D11Device,
    ID3D11DeviceContext,
};
use windows::Win32::Graphics::DirectComposition::{DCompositionCreateDevice, IDCompositionDevice};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, NULL_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadCursorW, RegisterClassW, SetWindowPos,
    ShowWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WNDCLASSW,
};

/// DirectComposition 渲染器：GPU 双线性缩放 + swapchain vsync 呈现。
pub struct DirectCompositionRenderer {
    hwnd: HWND,
    max_w: i32,
    max_h: i32,
    frame_w: u32,
    frame_h: u32,
    _device: ID3D11Device,
    _ctx: ID3D11DeviceContext,
    _dxgi_factory: IDXGIFactory2,
    swapchain: IDXGISwapChain1,
    dcomp: IDCompositionDevice,
    d2d_factory: ID2D1Factory,
}

impl DirectCompositionRenderer {
    /// 创建 DComp 窗口 + D3D11 设备 + swapchain + DComp 合成树。
    pub fn new(display_w: i32, display_h: i32, frame_w: i32, frame_h: i32) -> Result<Self, String> {
        let max_w = display_w * 6 / 5 + 8;
        let max_h = display_h * 6 / 5 + 8;
        let hwnd = unsafe { create_dcomp_window(max_w, max_h)? };

        unsafe {
            // D3D11 设备（BGRA 支持）
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut device: Option<ID3D11Device> = None;
            let mut ctx: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .map_err(|e| format!("D3D11CreateDevice 失败: {e}"))?;
            let device = device.expect("device null");
            let ctx = ctx.expect("context null");

            // DXGI 工厂 + 交换链（供 DComp 使用）
            let dxgi_factory: IDXGIFactory2 =
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))
                    .map_err(|e| format!("CreateDXGIFactory2: {e}"))?;
            let dxgi_device: IDXGIDevice =
                device.cast().map_err(|e| format!("D3D11 → IDXGIDevice 失败: {e}"))?;

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: max_w as u32,
                Height: max_h as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: Default::default(),
            };
            let swapchain: IDXGISwapChain1 = dxgi_factory
                .CreateSwapChainForComposition(&dxgi_device, &desc, None)
                .map_err(|e| format!("CreateSwapChainForComposition: {e}"))?;

            // DirectComposition 设备 + 目标 + 视觉
            let dcomp: IDCompositionDevice =
                DCompositionCreateDevice(&dxgi_device).map_err(|e| format!("DCompositionCreateDevice: {e}"))?;
            let target = dcomp
                .CreateTargetForHwnd(hwnd, true)
                .map_err(|e| format!("CreateTargetForHwnd: {e}"))?;
            let visual = dcomp.CreateVisual().map_err(|e| format!("CreateVisual: {e}"))?;
            visual
                .SetContent(&swapchain)
                .map_err(|e| format!("SetContent(swapchain): {e}"))?;
            target
                .SetRoot(&visual)
                .map_err(|e| format!("SetRoot: {e}"))?;
            dcomp.Commit().map_err(|e| format!("DComp Commit: {e}"))?;

            // Direct2D 工厂（用于在 swapchain 后缓冲上画精灵）
            let d2d_factory: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                    .map_err(|e| format!("D2D1CreateFactory: {e}"))?;

            Ok(DirectCompositionRenderer {
                hwnd,
                max_w,
                max_h,
                frame_w: frame_w as u32,
                frame_h: frame_h as u32,
                _device: device,
                _ctx: ctx,
                _dxgi_factory: dxgi_factory,
                swapchain,
                dcomp,
                d2d_factory,
            })
        }
    }

    /// 原始窗口句柄（菜单、输入、生命周期管理用）。
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn present_inner(&self, bgra: &[u8], x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        unsafe {
            // 移动/缩放窗口（swapchain 保持 max 尺寸，DComp 裁剪到窗口）
            SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE)
                .map_err(|e| format!("SetWindowPos: {e}"))?;

            // 取当前后缓冲，建 D2D 渲染目标 + 精灵位图
            let backbuffer: IDXGISurface =
                self.swapchain.GetBuffer(0).map_err(|e| format!("GetBuffer: {e}"))?;
            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                minLevel: windows::Win32::Graphics::Direct2D::D2D1_FEATURE_LEVEL_DEFAULT,
                usage: Default::default(),
            };
            let rt = self
                .d2d_factory
                .CreateDxgiSurfaceRenderTarget(&backbuffer, &rt_props)
                .map_err(|e| format!("CreateDxgiSurfaceRenderTarget: {e}"))?;

            // 帧 → D2D 位图（预乘 BGRA，D2D 同格式）
            let bmp_props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let bitmap: ID2D1Bitmap = rt
                .CreateBitmap(
                    D2D_SIZE_U { width: self.frame_w, height: self.frame_h },
                    Some(bgra.as_ptr() as *const c_void),
                    self.frame_w * 4,
                    &bmp_props,
                )
                .map_err(|e| format!("CreateBitmap: {e}"))?;

            // 画精灵：GPU 双线性缩放到 (w, h)
            rt.BeginDraw();
            let clear_color = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
            rt.Clear(Some(&clear_color as *const D2D1_COLOR_F));
            let dest = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: w as f32,
                bottom: h as f32,
            };
            rt.DrawBitmap(&bitmap, Some(&dest), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
            rt.EndDraw(None, None).map_err(|e| format!("EndDraw: {e}"))?;

            // 释放后缓冲引用后再 Present
            drop(rt);
            drop(bitmap);
            drop(backbuffer);

            // Present（vsync）→ DComp Commit（合成到桌面）
            self.swapchain
                .Present(1, DXGI_PRESENT(0))
                .ok()
                .map_err(|e| format!("Present: {e}"))?;
            self.dcomp.Commit().map_err(|e| format!("Commit: {e}"))?;
        }
        Ok(())
    }
}

impl Renderer for DirectCompositionRenderer {
    fn present(&mut self, bgra: &[u8], x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        self.present_inner(bgra, x, y, w, h)
    }
}

impl Drop for DirectCompositionRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// 创建非分层（DComp 合成）置顶窗口。
unsafe fn create_dcomp_window(w: i32, h: i32) -> Result<HWND, String> {
    let instance = windows::Win32::Foundation::HINSTANCE(
        GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?.0,
    );
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: HBRUSH(GetStockObject(NULL_BRUSH).0),
        lpszClassName: w!("MiloDCompWindow"),
        ..Default::default()
    };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        w!("MiloDCompWindow"),
        w!("Milo"),
        WS_POPUP,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        w,
        h,
        None,
        None,
        instance,
        None,
    )
    .map_err(|e| format!("CreateWindowExW: {e}"))?;
    ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
        .map_err(|e| format!("SetWindowPos: {e}"))?;
    Ok(hwnd)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
