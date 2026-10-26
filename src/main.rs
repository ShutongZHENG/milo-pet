//! Milo 桌宠（Rust 重写版，纯离线骨架）。
//!
//! 结构：
//! - 透明置顶分层窗口（win32::PetWindow），UpdateLayeredWindow 逐像素 alpha 渲染
//! - 动画：assets/frames.bin（idle.webm 预打包的预乘 BGRA 帧序列 + zstd），30fps 循环播放
//! - 交互：左键按住拖拽、甩抛（重力/反弹/摩擦）、点击 Q 弹、右键菜单（回位/退出）
//! - 配置：assets/config.toml

mod config;
mod frames;
mod physics;
mod win32;

use std::ffi::c_void;
use std::time::Instant;

use config::{Config, Corner};
use frames::FramePack;
use physics::Pet;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW,
    PeekMessageW, SetForegroundWindow, SetWindowPos, TrackPopupMenu, TranslateMessage, MSG,
    PM_REMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_NOACTIVATE, MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, WM_COMMAND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_RBUTTONUP, WM_QUIT,
};

const SQUASH_SECS: f64 = 0.22; // Q 弹时长
const THROW_GAIN: f64 = 1.6; // 甩抛初速增益
const THROW_WINDOW_SECS: f64 = 0.08; // 取最近 80ms 位移算甩抛速度

struct App {
    pack: FramePack,
    pet: Pet,
    home_x: i32,
    home_y: i32,
    squash_at: Option<Instant>,
    drag_dx: i32,
    drag_dy: i32,
    cursor: Vec<(f64, i32, i32)>, // (t, x, y) 采样，用于甩抛测速
    frame_buf: Vec<u8>,
    ground_y: i32,
    screen_w: i32,
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).ok();
    }
    if let Err(e) = run() {
        let msg = format!("Milo 启动失败: {e}");
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                &windows::core::HSTRING::from(msg.as_str()),
                w!("Milo"),
                windows::Win32::UI::WindowsAndMessaging::MB_OK | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}

fn run() -> Result<(), String> {
    let cfg = Config::load("assets/config.toml");
    let pack = FramePack::load("assets/frames.bin")?;
    let dpi = unsafe { GetDpiForSystem() } as f64;
    let scale = dpi / 96.0;
    eprintln!("DEBUG cfg.width={} dpi={} scale={}", cfg.width, dpi, scale);

    // 窗口 = 素材 16:9 画幅按配置宽度缩放
    let w = (cfg.width * scale).round() as i32;
    let h = (w as f64 * pack.height as f64 / pack.width as f64).round() as i32;

    let (work, screen_w) = work_area();
    let (home_x, home_y) = corner_pos(&cfg, w, h, &work);

    let frame_bytes = (pack.width * pack.height * 4) as usize;
    let (pw, ph) = (pack.width as i32, pack.height as i32);
    let mut app = App {
        pet: Pet { x: home_x as f64, y: home_y as f64, w, h, ..Default::default() },
        pack,
        home_x,
        home_y,
        squash_at: None,
        drag_dx: 0,
        drag_dy: 0,
        cursor: Vec::new(),
        frame_buf: vec![0u8; frame_bytes],
        ground_y: work.bottom,
        screen_w,
    };

    // UpdateLayeredWindow 不拉伸源位图，blit 内部把帧缩放到显示尺寸后再贴
    let window = win32::PetWindow::create(w, h, pw, ph)?;
    let hwnd = window.hwnd;

    // 首帧 + 动画循环
    let anim_dur = app.pack.count as f64 / app.pack.fps as f64;
    let frame_secs = 1.0 / app.pack.fps as f64;
    let mut anim_t = 0.0f64;
    let mut last_frame: i64 = -1;
    let mut last = Instant::now();
    let mut first_paint = true;

    let mut msg = MSG::default();
    'outer: loop {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                handle_message(hwnd, msg, &mut app);
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.05);
        last = now;
        anim_t = (anim_t + dt) % anim_dur;
        let frame_idx = (anim_t / frame_secs) as i64 % app.pack.count as i64;

        if frame_idx != last_frame || first_paint {
            app.pack.decode_frame(frame_idx as usize, &mut app.frame_buf)?;
            last_frame = frame_idx;
            first_paint = false;
        }

        // 物理步进（拖拽时停）；窗口位置与尺寸由 UpdateLayeredWindow 每帧设定
        app.pet.step(dt, app.ground_y as f64, app.screen_w as f64);

        // Q 弹形变：底边锚定，宽涨高缩
        let (sx, sy) = squash_factors(app.squash_at, now);
        let dw = (app.pet.w as f64 * sx).round() as i32;
        let dh = (app.pet.h as f64 * sy).round() as i32;
        let dx = (app.pet.w - dw) / 2;
        let dy = app.pet.h - dh;
        window.blit(
            &app.frame_buf,
            app.pet.x as i32 + dx,
            app.pet.y as i32 + dy,
            dw,
            dh,
        )?;

        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    Ok(())
}

fn squash_factors(at: Option<Instant>, now: Instant) -> (f64, f64) {
    match at {
        None => (1.0, 1.0),
        Some(t0) => {
            let t = now.duration_since(t0).as_secs_f64();
            if t >= SQUASH_SECS {
                (1.0, 1.0)
            } else {
                let s = (t / SQUASH_SECS * std::f64::consts::PI).sin();
                (1.0 + 0.15 * s, 1.0 - 0.12 * s)
            }
        }
    }
}

fn work_area() -> (RECT, i32) {
    unsafe {
        let mut rect = RECT::default();
        windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok();
        (rect, rect.right - rect.left)
    }
}

fn corner_pos(cfg: &Config, w: i32, h: i32, work: &RECT) -> (i32, i32) {
    let mx = (cfg.margin_x).round() as i32;
    let my = (cfg.margin_y).round() as i32;
    // 宠物站在"地面"（工作区底部）上；corner 只决定水平侧
    let y = work.bottom - h - my;
    let x = match cfg.corner {
        Corner::TopRight | Corner::BottomRight => work.right - w - mx,
        Corner::TopLeft | Corner::BottomLeft => work.left + mx,
    };
    (x, y)
}

unsafe fn handle_message(hwnd: HWND, msg: MSG, app: &mut App) {
    let now = Instant::now();
    match msg.message {
        WM_LBUTTONDOWN => {
            let (cx, cy) = cursor_pos();
            let inside = cx >= app.pet.x as i32
                && cx < app.pet.x as i32 + app.pet.w
                && cy >= app.pet.y as i32
                && cy < app.pet.y as i32 + app.pet.h;
            if inside {
                app.pet.dragging = true;
                app.drag_dx = cx - app.pet.x as i32;
                app.drag_dy = cy - app.pet.y as i32;
                app.cursor.clear();
                app.cursor.push((0.0, cx, cy));
                app.squash_at = Some(now);
            }
        }
        WM_MOUSEMOVE => {
            if app.pet.dragging {
                let (cx, cy) = cursor_pos();
                app.pet.x = (cx - app.drag_dx) as f64;
                app.pet.y = (cy - app.drag_dy) as f64;
                // 采样测速（保留 120ms）
                app.cursor.push((now.elapsed().as_secs_f64(), cx, cy));
                app.cursor.retain(|(t, _, _)| now.elapsed().as_secs_f64() - t < 0.12);
            }
        }
        WM_LBUTTONUP => {
            if app.pet.dragging {
                let keep: Vec<_> = app
                    .cursor
                    .iter()
                    .filter(|(t, _, _)| now.elapsed().as_secs_f64() - t < THROW_WINDOW_SECS)
                    .cloned()
                    .collect();
                if keep.len() >= 2 {
                    let (t0, x0, y0) = keep[0];
                    let (t1, x1, y1) = keep[keep.len() - 1];
                    let dt = (t1 - t0).max(0.016);
                    app.pet.throw((x1 - x0) as f64 / dt * THROW_GAIN, (y1 - y0) as f64 / dt * THROW_GAIN);
                } else {
                    app.pet.dragging = false;
                }
                app.cursor.clear();
            }
        }
        WM_RBUTTONUP => {
            show_menu(hwnd, app);
        }
        _ => {}
    }
}

unsafe fn show_menu(hwnd: HWND, app: &mut App) {
    SetForegroundWindow(hwnd);
    let menu = CreatePopupMenu().unwrap_or_default();
    AppendMenuW(menu, MF_STRING, 1, w!("回到初始位置"));
    AppendMenuW(menu, MF_SEPARATOR, 0, None);
    AppendMenuW(menu, MF_STRING, 2, w!("退出"));
    let mut pt = POINT::default();
    GetCursorPos(&mut pt);
    let chosen = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    match chosen.0 as u32 {
        1 => {
            app.pet.x = app.home_x as f64;
            app.pet.y = app.home_y as f64;
            app.pet.vx = 0.0;
            app.pet.vy = 0.0;
        }
        2 => {
            DestroyMenu(menu);
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            }
            return;
        }
        _ => {}
    }
    DestroyMenu(menu);
}

fn cursor_pos() -> (i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }
}

// 让 wparam/lparam 参与匹配编译期不告警
#[allow(dead_code)]
fn _msg_types(_: WPARAM, _: LPARAM, _: LRESULT) {}
