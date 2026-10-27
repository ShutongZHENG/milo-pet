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
mod score;
mod score_popup;
mod win32;

use std::ffi::c_void;
use std::time::Instant;

use config::{Config, Corner};
use frames::FramePack;
use physics::{PhysicsParams, Pet, ThrowSpace};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW,
    PeekMessageW, SetForegroundWindow, SetWindowPos, TrackPopupMenu, TranslateMessage, MSG,
    PM_REMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_NOACTIVATE, MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, WM_COMMAND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_RBUTTONUP, WM_QUIT,
};

struct App {
    pack: FramePack,
    pet: Pet,
    home_x: i32,
    home_y: i32,
    squash_at: Option<Instant>,
    /// Q 弹下压深度（点击 = SQ_SQUASH；落地 = landing_squash(冲击速度)）
    squash_depth: f64,
    drag_dx: i32,
    drag_dy: i32,
    /// 拖拽弹簧速度（px/s），spring_step 每帧推进
    drag_vel: (f64, f64),
    /// 轨迹采样（t, x, y），供松手初速估算；保留最近 TRAIL_KEEP_MS
    cursor: Vec<(Instant, i32, i32)>,
    /// 上次按下时刻（弹弓双击判定）；命中后清零
    last_press_at: Option<Instant>,
    /// 本次拖拽是否弹弓模式（按下时判定，松手时结算）
    sling: bool,
    /// 弹弓锚点 = 按下时刻宠物中心
    sling_ax: f64,
    sling_ay: f64,
    /// 空中→地面转换标记（落地 Q 弹只在转换帧触发一次）
    prev_grounded: bool,
    /// 锁定屏索引（confine_to_screen 时抛掷全程锁定；-1 = 不锁）
    lock_screen: i32,
    /// 锁定当前屏开关（config 根字段）
    confine: bool,
    /// 系统 DPI 缩放（物理/逻辑），计分与弹窗布局用
    dpi_scale: f64,
    /// 活跃的得分弹窗（2.2s 生命周期）
    popups: Vec<score_popup::ScorePopup>,
    physics: PhysicsParams,
    space: ThrowSpace,
    frame_buf: Vec<u8>,
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).ok();
        // Windows 默认定时器粒度 15.6ms：sleep(3~4ms) 会被拖到下一个 tick（≈18ms/循环），
        // 主循环只剩 ~55Hz；启动时把全局粒度设为 1ms，退出时还原。
        let _ = windows::Win32::Media::timeBeginPeriod(1);
    }
    let result = run();
    unsafe {
        let _ = windows::Win32::Media::timeEndPeriod(1);
    }
    if let Err(e) = result {
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

/// MILO_DEBUG=1 时开启诊断输出（启动配置、循环频率），默认完全静默。
fn debug_on() -> bool {
    std::env::var("MILO_DEBUG").map(|v| v == "1").unwrap_or(false)
}

fn run() -> Result<(), String> {
    let cfg = Config::load("assets/config.toml");
    let pack = FramePack::load("assets/frames.bin")?;
    let dpi = unsafe { GetDpiForSystem() } as f64;
    let scale = dpi / 96.0;
    let debug = debug_on();
    if debug {
        eprintln!("DEBUG cfg.width={} dpi={} scale={}", cfg.width, dpi, scale);
    }

    // 窗口 = 素材 16:9 画幅按配置宽度缩放
    let w = (cfg.width * scale).round() as i32;
    let h = (w as f64 * pack.height as f64 / pack.width as f64).round() as i32;

    let (work, _) = work_area();
    let (home_x, home_y) = corner_pos(&cfg, w, h, &work);

    // 抛掷空间：逐屏 AABB（工作区反弹 + 完整面板做邻屏探测）
    let mons = win32::monitors();
    let to_rect = |r: &RECT| physics::Rect {
        x: r.left as f64,
        y: r.top as f64,
        w: (r.right - r.left) as f64,
        h: (r.bottom - r.top) as f64,
    };
    let areas: Vec<physics::Rect> = mons.iter().map(|m| to_rect(&m.work)).collect();
    let panels: Vec<physics::Rect> = mons.iter().map(|m| to_rect(&m.panel)).collect();
    let space = physics::throw_space(areas, Some(panels), w as f64, physics::SIDE_ALLOW_RATIO * w as f64);

    let frame_bytes = (pack.width * pack.height * 4) as usize;
    let (pw, ph) = (pack.width as i32, pack.height as i32);
    let mut app = App {
        pet: Pet { x: home_x as f64, y: home_y as f64, w, h, ..Default::default() },
        pack,
        home_x,
        home_y,
        squash_at: None,
        squash_depth: physics::SQ_SQUASH,
        drag_dx: 0,
        drag_dy: 0,
        drag_vel: (0.0, 0.0),
        cursor: Vec::new(),
        last_press_at: None,
        sling: false,
        sling_ax: 0.0,
        sling_ay: 0.0,
        prev_grounded: false,
        lock_screen: -1,
        confine: cfg.confine_to_screen,
        dpi_scale: scale,
        popups: Vec::new(),
        physics: cfg.physics,
        space,
        frame_buf: vec![0u8; frame_bytes],
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
    let mut prev_lbtn = false;
    // 频率自测：MILO_DEBUG=1 时启动 3 秒后打印一次主循环 Hz
    let dbg_start = Instant::now();
    let mut dbg_frames: u64 = 0;
    let mut dbg_printed = false;
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
        // 拖拽跟踪在主循环做：不依赖 WM_MOUSEMOVE 投递（快拖时光标会逃出窗口，
        // 消息链即断），直接全局轮询光标位置与左键状态；位置由过阻尼弹簧追赶目标。
        if app.pet.dragging {
            let (cx, cy) = cursor_pos();
            let target_x = (cx - app.drag_dx) as f64;
            let target_y = (cy - app.drag_dy) as f64;
            // 记录轨迹供松手初速估算（保留 TRAIL_KEEP_MS）
            app.cursor.push((now, cx, cy));
            let keep_s = physics::TRAIL_KEEP_MS / 1000.0;
            app.cursor.retain(|(t, _, _)| t.elapsed().as_secs_f64() < keep_s);
            // 过阻尼弹簧追赶（dt 上限 1/30，与 dsh 拖拽跟随同源）
            let sdt = dt.min(1.0 / 30.0);
            let p = app.physics.throw_power;
            app.drag_vel.0 = physics::spring_step(app.drag_vel.0, app.pet.x, target_x, sdt, p);
            app.drag_vel.1 = physics::spring_step(app.drag_vel.1, app.pet.y, target_y, sdt, p);
            app.pet.x += app.drag_vel.0 * sdt;
            app.pet.y += app.drag_vel.1 * sdt;
            let lbtn_down = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16) & 0x8000 != 0 };
            if !lbtn_down {
                finish_release(&mut app);
            }
        } else {
            // 飞行/静止：子步积分防高速穿地，落地 Q 弹只在空中→地面转换帧触发一次
            let falling_vy = app.pet.vy;
            let substeps = physics::throw_substeps(app.pet.vx, app.pet.vy, dt);
            let step_dt = dt / substeps as f64;
            let mut outcome = physics::throw_step_region(
                physics::ThrowState { x: app.pet.x, y: app.pet.y, vx: app.pet.vx, vy: app.pet.vy },
                step_dt,
                &app.space,
                &app.physics,
                app.lock_screen,
            );
            for _ in 1..substeps {
                if outcome.at_rest {
                    break;
                }
                outcome = physics::throw_step_region(
                    outcome.state,
                    step_dt,
                    &app.space,
                    &app.physics,
                    app.lock_screen,
                );
            }
            app.pet.x = outcome.state.x;
            app.pet.y = outcome.state.y;
            app.pet.vx = outcome.state.vx;
            app.pet.vy = outcome.state.vy;
            let grounded = outcome.screen >= 0
                && outcome.state.y >= app.space.bounds[outcome.screen as usize].max_y - 1.0;
            if outcome.bounced && grounded && !app.prev_grounded {
                app.squash_at = Some(now);
                app.squash_depth = physics::landing_squash(falling_vy);
            }
            app.prev_grounded = grounded;
        }

        // 得分弹窗：点外/Esc 关闭（按下用全局轮询——桌面点击到不了本窗口）+ 生命周期推进
        let lbtn_down = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16) & 0x8000 != 0 };
        if lbtn_down && !prev_lbtn && !app.popups.is_empty() {
            let (cx, cy) = cursor_pos();
            for p in &mut app.popups {
                // 挂载宽限（同 dsh：吞掉触发挂载的那次按下）
                if p.age_ms(now) >= score_popup::GRACE_MS && !p.contains(cx, cy) {
                    p.close();
                }
            }
        }
        prev_lbtn = lbtn_down;
        if !app.popups.is_empty() {
            let esc = unsafe { (GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16) & 0x8000 != 0 };
            if esc {
                app.popups.iter_mut().for_each(|p| p.close());
            }
            app.popups.retain_mut(|p| p.tick(now));
        }

        anim_t = (anim_t + dt) % anim_dur;
        let frame_idx = (anim_t / frame_secs) as i64 % app.pack.count as i64;

        if frame_idx != last_frame || first_paint {
            app.pack.decode_frame(frame_idx as usize, &mut app.frame_buf)?;
            last_frame = frame_idx;
            first_paint = false;
        }

        // Q 弹形变：仅 scaleY、底边锚定（ease-in 下压 + easeOutBack 回弹）
        let sy = squash_factors(app.squash_at, app.squash_depth, now);
        let dw = app.pet.w;
        let dh = (app.pet.h as f64 * sy).round() as i32;
        let dx = 0;
        let dy = app.pet.h - dh;
        window.blit(
            &app.frame_buf,
            app.pet.x as i32 + dx,
            app.pet.y as i32 + dy,
            dw,
            dh,
        )?;

        // 频率自测（改动八配套诊断）
        dbg_frames += 1;
        if debug && !dbg_printed {
            let el = now.duration_since(dbg_start).as_secs_f64();
            if el >= 3.0 {
                eprintln!(
                    "DEBUG loop {:.0} Hz ({} frames / {:.2}s)",
                    dbg_frames as f64 / el,
                    dbg_frames,
                    el
                );
                dbg_printed = true;
            }
        }

        // 粒度 1ms 下 3ms 睡眠 ≈ 真 3ms（改动八：4ms → 3ms）
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    Ok(())
}

/// Q 弹进度 → scaleY（dsh squashScale：ease-in 下压 + easeOutBack 回弹，220ms）。
fn squash_factors(at: Option<Instant>, depth: f64, now: Instant) -> f64 {
    match at {
        None => 1.0,
        Some(t0) => {
            let u = now.duration_since(t0).as_secs_f64() * 1000.0 / physics::SQ_DURATION_MS;
            if u >= 1.0 {
                1.0
            } else {
                physics::squash_scale(u, depth)
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
                // 空中截击：按下瞬间飞行速度 ≥ 400（逻辑 px/s）即命中，弹得分卡；
                // 命中的按下不再播点击 Q 弹（一次按下只走一条路，dsh 同语义）
                let grab_speed =
                    (app.pet.vx * app.pet.vx + app.pet.vy * app.pet.vy).sqrt() / app.dpi_scale;
                let scored = grab_speed >= score::SCORE_MIN_SPEED;
                if scored {
                    let sc = score::click_score(grab_speed, app.pet.w as f64 / app.dpi_scale);
                    match score_popup::ScorePopup::create(
                        cx,
                        cy,
                        sc,
                        grab_speed,
                        app.pet.w as f64 / app.dpi_scale,
                        app.dpi_scale,
                    ) {
                        Ok(p) => app.popups.push(p),
                        Err(e) => eprintln!("score popup 创建失败: {e}"),
                    }
                }
                app.pet.dragging = true;
                app.drag_vel = (0.0, 0.0);
                app.drag_dx = cx - app.pet.x as i32;
                app.drag_dy = cy - app.pet.y as i32;
                app.cursor.clear();
                app.cursor.push((now, cx, cy));
                // 弹弓：两次按下间隔 < SLINGSHOT_DOUBLE_MS 即进弹弓模式；
                // 命中后清零，免得连点三下弹两次。
                let sling = match app.last_press_at {
                    Some(t) => {
                        now.duration_since(t).as_secs_f64() * 1000.0 < physics::SLINGSHOT_DOUBLE_MS
                    }
                    None => false,
                };
                app.last_press_at = if sling { None } else { Some(now) };
                app.sling = sling;
                if sling {
                    app.sling_ax = app.pet.x + app.pet.w as f64 / 2.0;
                    app.sling_ay = app.pet.y + app.pet.h as f64 / 2.0;
                }
                if !scored {
                    app.squash_at = Some(now);
                    app.squash_depth = physics::SQ_SQUASH;
                }
            }
        }
        WM_LBUTTONUP => {
            if app.pet.dragging {
                finish_release(app);
            }
        }
        WM_RBUTTONUP => {
            show_menu(hwnd, app);
        }
        _ => {}
    }
}

/// 松手收尾：弹弓按拉距结算初速；否则按轨迹估算（estimate_release_velocity），
/// 够快就抛掷、不够快温柔放下。confine_to_screen 时锁定当前所在屏。
fn finish_release(app: &mut App) {
    let now = Instant::now();
    if app.sling {
        // 弹弓：力度 = 拉距 /（宽×0.6），方向 = 拉的反方向；不设软上限
        let pull_x = app.pet.x + app.pet.w as f64 / 2.0 - app.sling_ax;
        let pull_y = app.pet.y + app.pet.h as f64 / 2.0 - app.sling_ay;
        let dist = (pull_x * pull_x + pull_y * pull_y).sqrt();
        let power = physics::slingshot_power(dist, app.pet.w as f64);
        let (vx, vy) = physics::slingshot_velocity(
            physics::pull_angle(pull_x, pull_y),
            power,
            app.physics.throw_power,
        );
        app.pet.throw(vx, vy);
    } else {
        match physics::estimate_release_velocity(&app.cursor, now, &app.physics) {
            Some((vx, vy)) => app.pet.throw(vx, vy),
            None => {
                // 温柔放下：原地解除拖拽，速度清零
                app.pet.dragging = false;
                app.pet.vx = 0.0;
                app.pet.vy = 0.0;
            }
        }
    }
    app.cursor.clear();
    app.drag_vel = (0.0, 0.0);
    app.sling = false;
    app.lock_screen = if app.confine {
        physics::screen_of_box(&app.space, app.pet.x, app.pet.y)
    } else {
        -1
    };
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
            app.lock_screen = -1;
            app.prev_grounded = false;
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
