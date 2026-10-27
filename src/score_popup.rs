//! 得分弹窗 + 粒子爆发：独立分层窗口，纯软件渲染（预乘 BGRA + GDI 灰度文字遮罩）。
//! 视觉 1:1 对标 dsh-pet `src/shared/score-popup.ts`：卡片 CSS、入场动画（CSS ease
//! 二分求逆）、阴影（圆角遮罩 + 三趟盒式模糊）、粒子参数、关闭语义逐项一致。

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, DeleteDC,
    DeleteObject, DrawTextW, GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject, SetBkMode,
    SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FW_BOLD, HBITMAP, HDC, HFONT, LOGFONTW, TRANSPARENT,
};

use crate::win32;

// ---- score-popup.ts 常量（逻辑 px / ms）----
/// 弹窗展示时长。
const SCORE_POPUP_DURATION_MS: f64 = 2200.0;
/// 入场动画时长（.28s ease）。
const ENTRANCE_MS: f64 = 280.0;
/// 挂载宽限期（ms）：吞掉触发本次挂载的按下。主循环点外关闭判定用。
pub const GRACE_MS: f64 = 300.0;
const MIN_CONTENT_W: f64 = 120.0;
const PAD_TOP: f64 = 8.0;
const PAD_LR: f64 = 16.0;
const PAD_BOT: f64 = 9.0;
const BORDER_W: f64 = 1.0;
const RADIUS: f64 = 12.0;
const VAL_FS: f64 = 22.0;
const VAL_LH: f64 = 27.5; // 22 * 1.25
const SUB_FS: f64 = 11.0;
const SUB_LH: f64 = 15.4; // 11 * 1.4
const SUB_MT: f64 = 2.0;
const SHADOW_DY: f64 = 10.0;
const SHADOW_BLUR: f64 = 32.0;
const SHADOW_A: f64 = 0.22;
const BG_A: f64 = 0.97;
const BORDER_R: u8 = 255;
const BORDER_G: u8 = 179;
const BORDER_B: u8 = 0;
const BORDER_A: f64 = 0.35;
const VAL_R: u8 = 255;
const VAL_G: u8 = 143;
const VAL_B: u8 = 0;
const SUB_R: u8 = 43;
const SUB_G: u8 = 43;
const SUB_B: u8 = 43;
const SUB_A: f64 = 0.6;
const CLICK_ABOVE: f64 = 14.0; // 弹窗底边距点击点
const VIEW_PAD: f64 = 4.0; // 贴边余量

// ---- 粒子（spawnScoreBurst）----
const BURST_COUNT: usize = 20;
const BURST_SPEED_MIN: f64 = 120.0;
const BURST_SPEED_MAX: f64 = 460.0;
const BURST_GRAVITY: f64 = 700.0;
const BURST_LIFE_MIN: f64 = 0.500; // 秒
const BURST_LIFE_MAX: f64 = 0.900;
const BURST_RADIUS_MIN: f64 = 3.0;
const BURST_RADIUS_MAX: f64 = 7.0;
const BURST_COLORS: [(u8, u8, u8); 7] = [
    (255, 179, 0),   // #ffb300
    (255, 143, 0),   // #ff8f00
    (255, 112, 67),  // #ff7043
    (244, 81, 30),   // #f4511e
    (255, 196, 0),   // #ffc400
    (255, 213, 79),  // #ffd54f
    (239, 83, 80),   // #ef5350
];

/// CSS cubic-bezier(0.25, 0.1, 0.25, 1)（`ease`）在 x 处的 y：x(u) 单调，二分求 u。
fn ease_css(x: f64) -> f64 {
    let bez = |u: f64, p1: f64, p2: f64| -> f64 {
        let v = 1.0 - u;
        3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u
    };
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if bez(mid, 0.25, 0.25) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez((lo + hi) * 0.5, 0.1, 1.0)
}

/// xorshift64*：无依赖伪随机（粒子参数）。
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Rng(nanos | 1)
    }
    fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Particle {
    ox: f64,
    oy: f64,
    vx: f64,
    vy: f64,
    r: f64,
    color: (u8, u8, u8),
    life: f64,
}

/// 加载私有字体（上首软糖体 / SSRuanTangTi）到进程，句柄故意保持到进程结束。
fn ensure_font_loaded() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        let bytes: &'static [u8] = include_bytes!("../assets/上首软糖体.ttf");
        let mut num = 0u32;
        let _ = AddFontMemResourceEx(
            bytes.as_ptr() as *const std::ffi::c_void,
            bytes.len() as u32,
            None,
            &mut num as *mut u32 as *const u32,
        );
    });
}

unsafe fn make_font(px: i32, bold: bool) -> Result<HFONT, String> {
    let mut lf = LOGFONTW::default();
    lf.lfHeight = -px;
    lf.lfWeight = if bold { FW_BOLD.0 as i32 } else { 400 };
    let name: Vec<u16> = "SSRuanTangTi".encode_utf16().collect();
    let n = name.len().min(lf.lfFaceName.len() - 1);
    lf.lfFaceName[..n].copy_from_slice(&name[..n]);
    let font = CreateFontIndirectW(&lf);
    if font.is_invalid() {
        return Err("CreateFontIndirectW 失败".into());
    }
    Ok(font)
}

/// 预乘 source-over：dst = src + dst*(1-sa)。
fn over(buf: &mut [u8], i: usize, sr: u32, sg: u32, sb: u32, sa: u32) {
    let inv = 255 - sa;
    buf[i] = (sr + (buf[i] as u32 * inv + 127) / 255).min(255) as u8;
    buf[i + 1] = (sg + (buf[i + 1] as u32 * inv + 127) / 255).min(255) as u8;
    buf[i + 2] = (sb + (buf[i + 2] as u32 * inv + 127) / 255).min(255) as u8;
    buf[i + 3] = (sa + (buf[i + 3] as u32 * inv + 127) / 255).min(255) as u8;
}

/// 圆角矩形 SDF 覆盖率写入 plane（0..1），仅光栅化 bbox。
fn rr_cov(plane: &mut [f32], w: i32, h: i32, cx: f64, cy: f64, hw: f64, hh: f64, radius: f64) {
    let r = radius.max(0.0);
    let x0 = ((cx - hw - r - 1.0).floor().max(0.0)) as i32;
    let x1 = ((cx + hw + r + 1.0).ceil().min(w as f64)) as i32;
    let y0 = ((cy - hh - r - 1.0).floor().max(0.0)) as i32;
    let y1 = ((cy + hh + r + 1.0).ceil().min(h as f64)) as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let qx = (px - cx).abs() - hw + r;
            let qy = (py - cy).abs() - hh + r;
            let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt()
                + qx.max(qy).min(0.0)
                - r;
            let cov = (0.5 - d).clamp(0.0, 1.0);
            plane[(y * w + x) as usize] = cov as f32;
        }
    }
}

/// 一趟盒式模糊（零填充，窗口均值）。horizontal: 按行；否则按列。
fn box_pass(src: &[f32], dst: &mut [f32], w: i32, h: i32, b: i32, horizontal: bool) {
    let half = b / 2;
    if horizontal {
        for y in 0..h {
            let row = (y * w) as usize;
            for x in 0..w {
                let mut sum = 0.0f32;
                for k in -half..=half {
                    let xx = x + k;
                    if xx >= 0 && xx < w {
                        sum += src[row + xx as usize];
                    }
                }
                dst[row + x as usize] = sum / b as f32;
            }
        }
    } else {
        for x in 0..w {
            for y in 0..h {
                let mut sum = 0.0f32;
                for k in -half..=half {
                    let yy = y + k;
                    if yy >= 0 && yy < h {
                        sum += src[(yy * w + x) as usize];
                    }
                }
                dst[(y * w + x) as usize] = sum / b as f32;
            }
        }
    }
}

/// 三趟盒式模糊 ≈ 高斯（σ ≈ b/2）。
fn box_blur3(plane: &mut [f32], tmp: &mut [f32], w: i32, h: i32, b: i32) {
    for _ in 0..3 {
        box_pass(plane, tmp, w, h, b, true);
        box_pass(tmp, plane, w, h, b, false);
    }
}

pub struct ScorePopup {
    window: win32::PopupWindow,
    mounted: Instant,
    closed: bool,
    layer_done: bool,
    // 文案
    val: String,
    sub: String,
    content_w: i32,
    // 物理屏幕坐标（卡片与窗口）
    card_x: i32,
    card_y: i32,
    card_w: i32,
    card_h: i32,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    layer_ox: i32,
    layer_oy: i32,
    layer_w: i32,
    layer_h: i32,
    dpi: f64,
    // 渲染缓冲
    layer: Vec<u8>,
    plane_a: Vec<f32>,
    plane_b: Vec<f32>,
    buf: Vec<u8>,
    // GDI 文字遮罩
    text_dc: HDC,
    text_dib: HBITMAP,
    text_bits: *mut u8,
    text_w: i32,
    text_h: i32,
    particles: Vec<Particle>,
}

impl ScorePopup {
    /// 在光标处创建弹窗（坐标为物理像素；speed/size 为逻辑 px——dsh 口径）。
    pub fn create(
        cx: i32,
        cy: i32,
        score: i32,
        speed: f64,
        size: f64,
        dpi: f64,
    ) -> Result<Self, String> {
        ensure_font_loaded();
        let val = format!("+{}", score);
        let sub = format!("速度 {} · 大小 {}", speed.round() as i64, size.round() as i64);

        // ---- 测量（scale=1 物理字体）----
        let val_px = (VAL_FS * dpi).round().max(1.0) as i32;
        let sub_px = (SUB_FS * dpi).round().max(1.0) as i32;
        let (w1, w2) = unsafe {
            let m1 = measure(&val, val_px, true)?;
            let m2 = measure(&sub, sub_px, false)?;
            (m1, m2)
        };
        let content_w = w1.max(w2).max((MIN_CONTENT_W * dpi).round() as i32);
        let line1_h = (VAL_LH * dpi).round() as i32;
        let line2_h = (SUB_LH * dpi).round() as i32;
        let gap = (SUB_MT * dpi).round() as i32;
        let content_h = line1_h + gap + line2_h;
        let card_w =
            content_w + ((PAD_LR * 2.0 + BORDER_W * 2.0) * dpi).round() as i32;
        let card_h = content_h + ((PAD_TOP + PAD_BOT + BORDER_W * 2.0) * dpi).round() as i32;

        // ---- 定位（dsh：点击上方，贴边夹回所在屏）----
        let mon = cursor_monitor(cx, cy);
        let vp = VIEW_PAD * dpi;
        let card_x = ((cx - card_w / 2) as f64)
            .max(mon.0 as f64 + vp)
            .min(mon.2 as f64 - card_w as f64 - vp) as i32;
        let card_y = (cy as f64 - card_h as f64 - CLICK_ABOVE * dpi).max(mon.1 as f64 + vp) as i32;

        // ---- 图层与窗口区域 ----
        let pad = (64.0 * dpi).ceil() as i32;
        let layer_w = card_w + pad * 2;
        let layer_h = card_h + pad * 2;
        // 粒子活动范围（初速/重力/寿命解析上界 + 余量）
        let reach = (430.0 * dpi) as i32;
        let up = (220.0 * dpi) as i32;
        let down = (640.0 * dpi) as i32;
        let win_x = card_x - pad;
        let win_y = (card_y - pad).min(cy - up);
        let win_x2 = (card_x + card_w + pad).max(cx + reach + (14.0 * dpi) as i32);
        let win_y2 = (card_y + card_h + pad).max(cy + down);
        let win_w = (win_x2 - win_x).max(1).min(4096);
        let win_h = (win_y2 - win_y).max(1).min(4096);
        let layer_ox = card_x - pad - win_x;
        let layer_oy = card_y - pad - win_y;

        // ---- 粒子 ----
        let mut rng = Rng::new();
        let mut particles = Vec::with_capacity(BURST_COUNT);
        for _ in 0..BURST_COUNT {
            let angle = rng.next() * std::f64::consts::PI * 2.0;
            let speed = BURST_SPEED_MIN + rng.next() * (BURST_SPEED_MAX - BURST_SPEED_MIN);
            let r = BURST_RADIUS_MIN + rng.next() * (BURST_RADIUS_MAX - BURST_RADIUS_MIN);
            let color = BURST_COLORS[(rng.next() * BURST_COLORS.len() as f64) as usize];
            let life = BURST_LIFE_MIN + rng.next() * (BURST_LIFE_MAX - BURST_LIFE_MIN);
            particles.push(Particle {
                ox: cx as f64 + r * dpi, // CSS left/top = 点击点，圆心偏移 r
                oy: cy as f64 + r * dpi,
                vx: angle.cos() * speed * dpi,
                vy: (angle.sin() * speed - 80.0) * dpi,
                r: r * dpi,
                color,
                life,
            });
        }

        // ---- GDI 文字遮罩 DC ----
        let text_w = content_w + 4;
        let text_h = line1_h.max(line2_h) + 4;
        let (text_dc, text_dib, text_bits) = unsafe { create_text_surface(text_w, text_h)? };

        let window = win32::PopupWindow::create(win_w, win_h)?;

        Ok(ScorePopup {
            window,
            mounted: Instant::now(),
            closed: false,
            layer_done: false,
            val,
            sub,
            content_w,
            card_x,
            card_y,
            card_w,
            card_h,
            win_x,
            win_y,
            win_w,
            win_h,
            layer_ox,
            layer_oy,
            layer_w,
            layer_h,
            dpi,
            layer: vec![0u8; (layer_w * layer_h * 4) as usize],
            plane_a: vec![0f32; (layer_w * layer_h) as usize],
            plane_b: vec![0f32; (layer_w * layer_h) as usize],
            buf: vec![0u8; (win_w * win_h * 4) as usize],
            text_dc,
            text_dib,
            text_bits,
            text_w,
            text_h,
            particles,
        })
    }

    /// 挂载至今的毫秒数。
    pub fn age_ms(&self, now: Instant) -> f64 {
        now.duration_since(self.mounted).as_secs_f64() * 1000.0
    }

    /// 物理屏幕点是否落在卡片内（点卡不关闭，同 dsh root.contains）。
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.card_x
            && x < self.card_x + self.card_w
            && y >= self.card_y
            && y < self.card_y + self.card_h
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    /// 推进一帧：超时/已关闭返回 false（本体被 drop，窗口随之销毁）。
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.closed {
            return false;
        }
        if self.age_ms(now) > SCORE_POPUP_DURATION_MS {
            return false;
        }
        let u = (self.age_ms(now) / ENTRANCE_MS).min(1.0);
        let e = ease_css(u);
        if !self.layer_done {
            self.render_layer(e);
            if u >= 1.0 {
                self.layer_done = true;
            }
        }
        self.render_window(now);
        true
    }

    /// 卡片图层：阴影 + 卡片 + 文字（入场期间随 scale/opacity 重绘，完成后静态）。
    fn render_layer(&mut self, e: f64) {
        let s = 0.6 + 0.4 * e;
        let dpi = self.dpi;
        let (lw, lh) = (self.layer_w, self.layer_h);
        let area = (lw * lh) as usize;
        self.layer.fill(0);

        let hw = self.card_w as f64 * s / 2.0;
        let hh = self.card_h as f64 * s / 2.0;
        let cx_l = lw as f64 / 2.0;
        let cy_l = lh as f64 / 2.0;
        let radius = RADIUS * dpi * s;

        // 阴影：圆角遮罩下移 + 三趟盒式模糊，黑 alpha 0.22
        self.plane_a.fill(0.0);
        rr_cov(
            &mut self.plane_a,
            lw,
            lh,
            cx_l,
            cy_l + SHADOW_DY * dpi * s,
            hw,
            hh,
            radius,
        );
        let b = ((SHADOW_BLUR * dpi * s).round() as i32).max(1);
        box_blur3(&mut self.plane_a, &mut self.plane_b, lw, lh, b);
        for i in 0..area {
            let a = self.plane_a[i] as f64 * SHADOW_A;
            if a > 0.002 {
                self.layer[i * 4 + 3] = (a * 255.0).min(255.0) as u8; // 预乘黑：rgb=0
            }
        }

        // 卡片填充 rgba(255,255,255,.97)
        self.plane_a.fill(0.0);
        rr_cov(&mut self.plane_a, lw, lh, cx_l, cy_l, hw, hh, radius);
        for i in 0..area {
            let a = self.plane_a[i] as f64 * BG_A;
            if a > 0.002 {
                let sa = (a * 255.0).round().min(255.0) as u32;
                let sc = (255.0 * a).round().min(255.0) as u32;
                over(&mut self.layer, i * 4, sc, sc, sc, sa);
            }
        }

        // 边框 rgba(255,179,0,.35)，位于卡片外缘 1px 内
        let bw = (BORDER_W * dpi * s).max(1.0);
        self.plane_b.fill(0.0);
        rr_cov(
            &mut self.plane_b,
            lw,
            lh,
            cx_l,
            cy_l,
            (hw - bw).max(0.0),
            (hh - bw).max(0.0),
            (radius - bw).max(0.0),
        );
        for i in 0..area {
            let ring = (self.plane_a[i] - self.plane_b[i]).max(0.0) as f64 * BORDER_A;
            if ring > 0.002 {
                let sa = (ring * 255.0).round().min(255.0) as u32;
                let sr = (BORDER_R as f64 * ring).round().min(255.0) as u32;
                let sg = (BORDER_G as f64 * ring).round().min(255.0) as u32;
                let sb = (BORDER_B as f64 * ring).round().min(255.0) as u32;
                over(&mut self.layer, i * 4, sr, sg, sb, sa);
            }
        }

        // 文字（GDI 灰度遮罩 + 着色；DT_CENTER|DT_VCENTER 居中）
        let card_x0 = cx_l - hw;
        let card_y0 = cy_l - hh;
        let content_x = card_x0 + (BORDER_W + PAD_LR) * dpi * s;
        let content_w_s = self.content_w as f64 * s;
        let line1_h_s = (VAL_LH * dpi * s).round().max(1.0) as i32;
        let line2_h_s = (SUB_LH * dpi * s).round().max(1.0) as i32;
        let gap_s = (SUB_MT * dpi * s).round() as i32;
        let y1 = (card_y0 + (BORDER_W + PAD_TOP) * dpi * s).round() as i32;
        let y2 = y1 + line1_h_s + gap_s;
        let x = content_x.round() as i32;
        let w = content_w_s.round().max(1.0) as i32;
        let val_px = ((VAL_FS * dpi * s).round() as i32).max(1);
        let sub_px = ((SUB_FS * dpi * s).round() as i32).max(1);
        let val_text = self.val.clone();
        let sub_text = self.sub.clone();
        unsafe {
            self.composite_line(x, y1, w, line1_h_s, &val_text, val_px, true, (VAL_R, VAL_G, VAL_B), 1.0);
            self.composite_line(x, y2, w, line2_h_s, &sub_text, sub_px, false, (SUB_R, SUB_G, SUB_B), SUB_A);
        }

        // 入场不透明度（整层等比淡入，CSS opacity 0→1）
        let o = (e * 255.0).round() as u32;
        if o < 255 {
            for c in self.layer.iter_mut() {
                *c = ((*c as u32 * o + 127) / 255) as u8;
            }
        }
    }

    /// 把一行文字画进 GDI 遮罩并着色合成到 layer 的 (x, y) 处。
    unsafe fn composite_line(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        text: &str,
        font_px: i32,
        bold: bool,
        color: (u8, u8, u8),
        alpha: f64,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let tw = self.text_w.min(w);
        let th = self.text_h.min(h);
        let text_stride = self.text_w;
        let surface =
            std::slice::from_raw_parts_mut(self.text_bits, (text_stride * self.text_h * 4) as usize);
        // 清空遮罩区
        for yy in 0..th {
            let base = ((yy * text_stride) as usize) * 4;
            surface[base..base + (tw as usize) * 4].fill(0);
        }
        let font = match make_font(font_px, bold) {
            Ok(f) => f,
            Err(_) => return,
        };
        let old = SelectObject(self.text_dc, font);
        // COLORREF = 0x00BBGGRR：白字
        SetTextColor(self.text_dc, COLORREF(0x00FF_FFFF));
        SetBkMode(self.text_dc, TRANSPARENT);
        let mut u16s: Vec<u16> = text.encode_utf16().collect();
        let text_len = u16s.len();
        u16s.push(0);
        let mut rc = RECT { left: 0, top: 0, right: tw, bottom: th };
        let _ = DrawTextW(
            self.text_dc,
            &mut u16s[..text_len],
            &mut rc as *mut RECT,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(self.text_dc, old);
        let _ = DeleteObject(font);

        // 遮罩 → 着色 → source-over
        let lw = self.layer_w;
        let layer_h = self.layer_h;
        for yy in 0..th {
            for xx in 0..tw {
                let mi = ((yy * text_stride + xx) as usize) * 4;
                let mask = surface[mi] as u32; // 白字灰度（BGRA 任一通道同值）
                if mask == 0 {
                    continue;
                }
                let cov = mask as f64 / 255.0 * alpha;
                if cov <= 0.0 {
                    continue;
                }
                let lx = x + xx;
                let ly = y + yy;
                if lx < 0 || ly < 0 || lx >= lw || ly >= layer_h {
                    continue;
                }
                let di = ((ly * lw + lx) as usize) * 4;
                let sa = (cov * 255.0).round().min(255.0) as u32;
                let sr = (color.0 as f64 * cov).round().min(255.0) as u32;
                let sg = (color.1 as f64 * cov).round().min(255.0) as u32;
                let sb = (color.2 as f64 * cov).round().min(255.0) as u32;
                over(&mut self.layer, di, sr, sg, sb, sa);
            }
        }
    }

    /// 窗口缓冲：清空 + 贴卡片图层 + 粒子，UpdateLayeredWindow。
    fn render_window(&mut self, now: Instant) {
        self.buf.fill(0);
        // 卡片图层
        let (lw, lh) = (self.layer_w, self.layer_h);
        let row_src = (lw * 4) as usize;
        let row_dst = (self.win_w * 4) as usize;
        let ox = self.layer_ox as usize * 4;
        let oy = self.layer_oy as usize;
        for y in 0..lh as usize {
            let s = y * row_src;
            let d = (y + oy) * row_dst + ox;
            self.buf[d..d + row_src].copy_from_slice(&self.layer[s..s + row_src]);
        }
        // 粒子（独立于入场动画）
        let t = now.duration_since(self.mounted).as_secs_f64();
        for p in &self.particles {
            let lr = t / p.life;
            if lr >= 1.0 {
                continue;
            }
            let cx = p.ox + p.vx * t - self.win_x as f64;
            let cy = p.oy + p.vy * t + 0.5 * BURST_GRAVITY * t * t - self.win_y as f64;
            let alpha = 1.0 - lr;
            let x0 = ((cx - p.r - 1.0).floor() as i32).max(0);
            let x1 = ((cx + p.r + 1.0).ceil() as i32).min(self.win_w - 1);
            let y0 = ((cy - p.r - 1.0).floor() as i32).max(0);
            let y1 = ((cy + p.r + 1.0).ceil() as i32).min(self.win_h - 1);
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let dx = px as f64 + 0.5 - cx;
                    let dy = py as f64 + 0.5 - cy;
                    let d = (dx * dx + dy * dy).sqrt();
                    let aa = (p.r + 0.5 - d).clamp(0.0, 1.0);
                    let cov = aa * alpha;
                    if cov <= 0.0 {
                        continue;
                    }
                    let i = ((py as usize) * self.win_w as usize + px as usize) * 4;
                    let sa = (cov * 255.0).round().min(255.0) as u32;
                    let sr = (p.color.0 as f64 * cov).round().min(255.0) as u32;
                    let sg = (p.color.1 as f64 * cov).round().min(255.0) as u32;
                    let sb = (p.color.2 as f64 * cov).round().min(255.0) as u32;
                    over(&mut self.buf, i, sr, sg, sb, sa);
                }
            }
        }
        let _ = self.window.update(&self.buf, self.win_x, self.win_y);
    }
}

impl Drop for ScorePopup {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.text_dib);
            let _ = DeleteDC(self.text_dc);
        }
        // window 随字段 drop → DestroyWindow
    }
}

/// 测量一行文字宽度（物理 px，scale=1 字体）。
unsafe fn measure(text: &str, px: i32, bold: bool) -> Result<i32, String> {
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return Err("GetDC 失败".into());
    }
    let font = make_font(px, bold)?;
    let old = SelectObject(hdc, font);
    let mut buf: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let r = GetTextExtentPoint32W(hdc, &mut buf, &mut size);
    SelectObject(hdc, old);
    let _ = DeleteObject(font);
    ReleaseDC(None, hdc);
    if !r.as_bool() {
        return Err("GetTextExtentPoint32W 失败".into());
    }
    Ok(size.cx)
}

/// 建 32bpp 顶向下 DIB 文字遮罩面。
unsafe fn create_text_surface(w: i32, h: i32) -> Result<(HDC, HBITMAP, *mut u8), String> {
    let screen = GetDC(None);
    let mem_dc = CreateCompatibleDC(screen);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut u8 = std::ptr::null_mut();
    let dib = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits as *mut _ as *mut _, None, 0)
        .map_err(|e| format!("CreateDIBSection(text): {e}"))?;
    SelectObject(mem_dc, dib);
    ReleaseDC(None, screen);
    if bits.is_null() {
        return Err("text bits null".into());
    }
    Ok((mem_dc, dib, bits))
}

/// 光标所在屏的面板 (left, top, right, bottom)；找不到退化用第一块。
fn cursor_monitor(cx: i32, cy: i32) -> (i32, i32, i32, i32) {
    let mons = win32::monitors();
    for m in &mons {
        let p = m.panel;
        if cx >= p.left && cx < p.right && cy >= p.top && cy < p.bottom {
            return (p.left, p.top, p.right, p.bottom);
        }
    }
    let p = mons[0].panel;
    (p.left, p.top, p.right, p.bottom)
}
