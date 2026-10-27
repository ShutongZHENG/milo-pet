//! 透明置顶分层窗口 + UpdateLayeredWindow 逐像素 alpha 渲染（预乘 BGRA）。

use std::ffi::c_void;
use std::mem;
use std::ptr;

use windows::core::w;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC,
    GetMonitorInfoW, GetStockObject, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLENDFUNCTION, DIB_RGB_COLORS, HBRUSH, HBITMAP, HDC, HMONITOR, MONITORINFOEXW, NULL_BRUSH,
    AC_SRC_ALPHA, AC_SRC_OVER,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetWindowPos, UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW,
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, ULW_ALPHA, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub const CLASS_NAME: PCWSTR = w!("MiloPetWindow");

/// 一块显示器：工作区（反弹与归属判定，任务栏挖掉的条带不在此列）+ 完整面板（含任务栏区）。
pub struct Monitor {
    pub work: RECT,
    pub panel: RECT,
}

/// EnumDisplayMonitors 回调：收集 MONITORINFOEXW 的 rcWork / rcMonitor。
unsafe extern "system" fn enum_mon_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _lprc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<Monitor>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut mi as *mut MONITORINFOEXW as *mut _).as_bool() {
        out.push(Monitor { work: mi.monitorInfo.rcWork, panel: mi.monitorInfo.rcMonitor });
    }
    BOOL(1) // 继续枚举
}

/// 枚举全部显示器（工作区 + 面板）。失败或为空时退化为系统工作区单屏。
pub fn monitors() -> Vec<Monitor> {
    unsafe {
        let mut out: Vec<Monitor> = Vec::new();
        let ptr = &mut out as *mut Vec<Monitor>;
        let _ = EnumDisplayMonitors(None, None, Some(enum_mon_proc), LPARAM(ptr as isize));
        if out.is_empty() {
            let mut rect = RECT::default();
            windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
                windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
                0,
                Some(&mut rect as *mut RECT as *mut c_void),
                windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok();
            out.push(Monitor { work: rect, panel: rect });
        }
        out
    }
}

pub struct PetWindow {
    pub hwnd: HWND,
    blitter: Blitter,
}

pub struct Blitter {
    mem_dc: HDC,
    bitmap: HBITMAP,
    bits: *mut c_void,
    /// DIB 尺寸 = 最大显示尺寸（含 Q 弹放大余量）
    w: i32,
    h: i32,
    /// 帧素材原始尺寸（640x360）
    frame_w: i32,
    frame_h: i32,
}

/// 双线性缩放预乘 BGRA 帧到目标尺寸，写入 dst（行主序，dw*dh*4）。
/// 预乘空间是线性的，插值结果对合成正确（不产生颜色越界）。
fn scale_bilinear(src: &[u8], sw: i32, sh: i32, dst: &mut [u8], dw: i32, dh: i32) {
    let sxr = sw as f32 / dw as f32;
    let syr = sh as f32 / dh as f32;
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) * syr - 0.5).clamp(0.0, sh as f32 - 1.0);
        let y0 = sy.floor() as i32;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f32;
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) * sxr - 0.5).clamp(0.0, sw as f32 - 1.0);
            let x0 = sx.floor() as i32;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f32;
            let i00 = ((y0 * sw + x0) * 4) as usize;
            let i10 = ((y0 * sw + x1) * 4) as usize;
            let i01 = ((y1 * sw + x0) * 4) as usize;
            let i11 = ((y1 * sw + x1) * 4) as usize;
            let di = ((y * dw + x) * 4) as usize;
            for c in 0..4 {
                let a = src[i00 + c] as f32;
                let b = src[i10 + c] as f32;
                let c2 = src[i01 + c] as f32;
                let d = src[i11 + c] as f32;
                let v = (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c2 * (1.0 - fx) + d * fx) * fy;
                dst[di + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

impl Blitter {
    pub fn new(display_w: i32, display_h: i32, frame_w: i32, frame_h: i32) -> Result<Self, String> {
        let w = display_w;
        let h = display_h;
        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(screen_dc);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // 负值 = 自顶向下，行序与内存一致
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = ptr::null_mut();
            let bitmap = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .map_err(|e| format!("CreateDIBSection 失败: {e}"))?;
            SelectObject(mem_dc, bitmap);
            ReleaseDC(None, screen_dc);
            Ok(Blitter { mem_dc, bitmap, bits, w, h, frame_w, frame_h })
        }
    }

    /// 把帧（frame_w×frame_h 预乘 BGRA）缩放到 dst_w×dst_h 后贴到屏幕 (dst_x, dst_y)。
    /// UpdateLayeredWindow 不会拉伸源位图——必须先缩放再贴，否则只显示左上角裁切。
    pub fn blit(
        &self,
        hwnd: HWND,
        bgra: &[u8],
        dst_x: i32,
        dst_y: i32,
        dst_w: i32,
        dst_h: i32,
    ) -> Result<(), String> {
        unsafe {
            let expect = (self.frame_w * self.frame_h * 4) as usize;
            if bgra.len() != expect {
                return Err(format!("帧尺寸不匹配: {} != {expect}", bgra.len()));
            }
            if dst_w > self.w || dst_h > self.h {
                return Err(format!("显示尺寸 {dst_w}x{dst_h} 超出 DIB {}x{}", self.w, self.h));
            }
            // 缩放帧到目标尺寸，写入 DIB 左上角 (dst_w × dst_h) 区域
            let dib = std::slice::from_raw_parts_mut(self.bits as *mut u8, (self.w * self.h * 4) as usize);
            if dst_w == self.frame_w && dst_h == self.frame_h {
                // 1:1 快路径
                dib[..expect].copy_from_slice(bgra);
            } else {
                // 只写目标行区（行距 self.w*4，目标宽 dst_w*4）
                let mut tmp = vec![0u8; (dst_w * dst_h * 4) as usize];
                scale_bilinear(bgra, self.frame_w, self.frame_h, &mut tmp, dst_w, dst_h);
                let row_dst = (self.w * 4) as usize;
                let row_src = (dst_w * 4) as usize;
                for y in 0..dst_h as usize {
                    let s = y * row_src;
                    let d = y * row_dst;
                    dib[d..d + row_src].copy_from_slice(&tmp[s..s + row_src]);
                }
            }

            let dst_dc = GetDC(None);
            let pt = POINT { x: dst_x, y: dst_y };
            let size = SIZE { cx: dst_w, cy: dst_h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                dst_dc,
                Some(&pt),
                Some(&size),
                self.mem_dc,
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .map_err(|e| format!("UpdateLayeredWindow 失败: {e}"))?;
            ReleaseDC(None, dst_dc);
            Ok(())
        }
    }
}

impl Drop for Blitter {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.bitmap);
            DeleteDC(self.mem_dc);
        }
    }
}

impl PetWindow {
    pub fn create(display_w: i32, display_h: i32, frame_w: i32, frame_h: i32) -> Result<Self, String> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?.0);
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(GetStockObject(NULL_BRUSH).0),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("Milo"),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                display_w,
                display_h,
                None,
                None,
                instance,
                None,
            )
            .map_err(|e| format!("CreateWindowExW 失败: {e}"))?;
            // 分层窗口必须先显式 ShowWindow，否则 DWM 不建立重定向表面（内容全透明）
            windows::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .ok();
            // DIB 预留 Q 弹放大余量：scaleY 回弹段过冲最大 1.12（easeOutBack），宽高都要 6/5 余量
            let blitter = Blitter::new(display_w * 6 / 5 + 8, display_h * 6 / 5 + 8, frame_w, frame_h)?;
            Ok(PetWindow { hwnd, blitter })
        }
    }

    pub fn blit(&self, bgra: &[u8], x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        self.blitter.blit(self.hwnd, bgra, x, y, w, h)
    }
}

pub unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

const POPUP_CLASS_NAME: PCWSTR = w!("MiloScorePopupWindow");

/// 弹窗专用 wndproc：销毁只关自己，不 PostQuitMessage（区别于桌宠主窗口）。
unsafe extern "system" fn popup_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 一次性缓冲的分层窗口（得分弹窗等）：1:1 贴预乘 BGRA，无缩放。
pub struct PopupWindow {
    pub hwnd: HWND,
    blitter: Blitter,
    w: i32,
    h: i32,
}

impl PopupWindow {
    pub fn create(w: i32, h: i32) -> Result<Self, String> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?.0);
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(popup_wndproc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(GetStockObject(NULL_BRUSH).0),
                lpszClassName: POPUP_CLASS_NAME,
                ..Default::default()
            };
            RegisterClassW(&wc); // 已注册时报错可忽略
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                POPUP_CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                w,
                h,
                None,
                None,
                instance,
                None,
            )
            .map_err(|e| format!("CreateWindowExW(popup) 失败: {e}"))?;
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .ok();
            let blitter = Blitter::new(w, h, w, h)?;
            Ok(PopupWindow { hwnd, blitter, w, h })
        }
    }

    /// 1:1 贴整块预乘 BGRA（长度必须为 w*h*4）并更新到屏幕 (x, y)。
    pub fn update(&self, bgra: &[u8], x: i32, y: i32) -> Result<(), String> {
        self.blitter.blit(self.hwnd, bgra, x, y, self.w, self.h)
    }
}

impl Drop for PopupWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
