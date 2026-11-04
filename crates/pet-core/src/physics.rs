//! 拖拽抛掷物理：弹簧跟手、甩抛初速估算、重力与反弹、Q 弹挤压。
//! 逐常数/逐算法对标 dsh-pet `src/shared/physics.ts`（浏览器 overlay 与桌面端手感必须一致）。
//! 坐标为宠物包围盒左上角的像素；轨迹采样时间戳用 Instant（见 main.rs 的 cursor 轨迹）。

use std::time::Instant;

// ---- 拖拽弹簧 ----
/// 拖拽弹簧刚度：越大跟手越紧。
pub const SPRING_K: f64 = 200.0;
/// 拖拽弹簧阻尼：ζ = c/(2√k) ≈ 1.06，过阻尼，不 overshoot。
pub const SPRING_C: f64 = 30.0;

// ---- 初速估算 ----
/// 拖拽轨迹保留窗口（ms）：只留最近这一段做初速估算。
pub const TRAIL_KEEP_MS: f64 = 200.0;
/// 初速估算窗口（ms）。
pub const RELEASE_WINDOW_MS: f64 = 150.0;
/// 松手前停顿超过它即温柔放下（不带残余速度），即使之前甩过。
pub const RELEASE_STALE_MS: f64 = 150.0;
/// 窗口太短视为不可估算（ms）。
pub const MIN_SPAN_MS: f64 = 20.0;
/// 分段速度的最小 dt（ms）：过密采样向前合并，过小 dt 会把抖动放大成虚假峰值。
pub const SEG_MIN_DT_MS: f64 = 8.0;
/// 低于此速度即不抛（原地放下），px/s。
pub const DEAD_ZONE_SPEED: f64 = 500.0;
/// 甩出速度软上限（px/s）：cap×(1-e^(-s/cap))，任意力度仍单调可区分，渐近不超过 cap。
pub const MAX_THROW_SPEED: f64 = 3600.0;
/// 初速大小 = 端点均值×(1-w) + 窗口峰值×w，弥补快甩时位移集中在窗口内一小段的低估。
pub const PEAK_WEIGHT: f64 = 0.5;
/// 参考加速度（px/s²）：末段加速达到它即吃满增益。
pub const ACCEL_REF: f64 = 8000.0;
/// 加速度增益上限：仍在加速的甩动最多放大 60%。
pub const ACCEL_GAIN_MAX: f64 = 0.6;

// ---- 抛掷物理默认值（与内置 config 的 physics 段一致）----
pub const GRAVITY: f64 = 1400.0;
pub const RESTITUTION: f64 = 0.78;
pub const GROUND_FRICTION: f64 = 2.5;
pub const DEFAULT_THROW_POWER: f64 = 1.0;
/// 落地时 |vy| 小于它直接停竖直。
pub const REST_VY: f64 = 40.0;
/// 地面上 |vx| 小于它认为已静止。
pub const REST_VX: f64 = 15.0;
/// 单步最大 dt（s）：防卡顿恢复后的巨帧跳变。
pub const MAX_STEP_DT: f64 = 0.05;

// ---- Q 弹 ----
/// 下压幅度：高度压到 55%。
pub const SQ_SQUASH: f64 = 0.55;
/// 挤压时长（ms）。
pub const SQ_DURATION_MS: f64 = 220.0;
/// 落地冲击速度基准：低于此即轻落（按下限幅度）。
pub const SQ_SOFT_SPEED: f64 = 300.0;
/// 落地冲击速度上限：达到或超过此即重砸（吃满最大下压）。
pub const SQ_HARD_SPEED: f64 = 1500.0;
/// 落地最大下压幅度（与点击一致）；轻落下限 0.8。
pub const SQ_MAX_SQUASH: f64 = 0.55;

// ---- 弹弓 ----
/// 「一倍力度」的参考拉距 = 宠物宽 × 本比例。
pub const SLINGSHOT_REF_RATIO: f64 = 0.6;
/// 双击判定窗口（ms）。
pub const SLINGSHOT_DOUBLE_MS: f64 = 200.0;
/// 抛掷指令的力度锚点：power=1.0 对应软钳速前的 1200 px/s。
pub const THROW_SPEED_ANCHOR: f64 = 1200.0;

// ---- 子步积分 ----
/// 单步允许的最大位移（px）：一帧位移超过它就拆成多步积分。
pub const SUBSTEP_PX: f64 = 8.0;
/// 单帧最多拆几步（兜底）。
pub const SUBSTEP_MAX: u32 = 64;

// ---- 命中框 ----
/// 身体相对包围盒左右各内缩的量（HIT_BOX.x0/640，dsh constants.ts 同源）。
pub const SIDE_ALLOW_RATIO: f64 = 200.0 / 640.0;

/// 运行时物理参数（成品配置的 [physics] 段；缺失字段回落本默认）。
#[derive(Clone, Copy, Debug)]
pub struct PhysicsParams {
    pub gravity: f64,
    pub restitution: f64,
    pub ground_friction: f64,
    pub ceiling_bounce: bool,
    pub throw_power: f64,
}

impl Default for PhysicsParams {
    fn default() -> Self {
        PhysicsParams {
            gravity: GRAVITY,
            restitution: RESTITUTION,
            ground_friction: GROUND_FRICTION,
            ceiling_bounce: true,
            throw_power: DEFAULT_THROW_POWER,
        }
    }
}

/// 宠物位置状态（包围盒左上角 + 速度）。
#[derive(Default)]
pub struct Pet {
    /// 窗口左上角（物理像素）
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub w: i32,
    pub h: i32,
    pub dragging: bool,
}

impl Pet {
    pub fn throw(&mut self, vx: f64, vy: f64) {
        self.vx = vx;
        self.vy = vy;
        self.dragging = false;
    }
}

/// 矩形（屏幕坐标，右/下缘不含）。
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    /// 点是否落在矩形内（右/下缘不含）：相邻两屏缝隙为 0 时，点归左/上那块。
    fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

/// 抛掷边界：包围盒左上角的允许活动范围（身体贴边语义）。
#[derive(Clone, Copy, Debug)]
pub struct ThrowBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// 抛体当前状态（包围盒左上角 px + 速度 px/s）。
#[derive(Clone, Copy, Debug)]
pub struct ThrowState {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
}

/// throw_step_region 的一步结果。
#[derive(Clone, Copy, Debug)]
pub struct StepOutcome {
    pub state: ThrowState,
    /// 本步结束时身体中心所在的屏（锁定时恒为锁定屏）
    pub screen: i32,
    pub bounced: bool,
    pub at_rest: bool,
}

/// 抛掷空间：每块屏一套 AABB，而不是整个桌面一个大 AABB。
/// 不规则多屏布局下外接矩形含空洞，用它当边界会让宠物飞进不可见区域。
pub struct ThrowSpace {
    /// 逐屏的包围盒左上角允许范围（与 areas 同序）
    pub bounds: Vec<ThrowBounds>,
    /// 逐屏工作区（反弹与归属判定；任务栏挖掉的条带不在此列）
    pub areas: Vec<Rect>,
    /// 逐屏完整面板（含任务栏区，与 areas 同序）：越界侧有没有邻屏要用面板探测——
    /// 任务栏位于屏缝时工作区之间会空出一条带，按工作区探测会把接缝当墙。
    /// 缺省 = areas（单屏时两者等价）。
    pub panels: Vec<Rect>,
    /// 宠物包围盒宽
    pub size: f64,
    /// 身体相对包围盒左右各内缩的量（构造时已折算进 bounds；字段与 dsh ThrowSpace 对齐，暂无读取方）
    #[allow(dead_code)]
    pub side_allow: f64,
}

fn rect_right(r: &Rect) -> f64 {
    r.x + r.w
}
fn rect_bottom(r: &Rect) -> f64 {
    r.y + r.h
}

/// 点被哪块矩形覆盖；无则 −1。重叠时取列表中第一块。
fn index_at_point(rects: &[Rect], x: f64, y: f64) -> i32 {
    for (i, r) in rects.iter().enumerate() {
        if r.contains(x, y) {
            return i as i32;
        }
    }
    -1
}

fn dist_to_rect_sq(r: &Rect, x: f64, y: f64) -> f64 {
    let dx = (r.x - x).max(0.0).max(x - rect_right(r));
    let dy = (r.y - y).max(0.0).max(y - rect_bottom(r));
    dx * dx + dy * dy
}

/// 离点最近的屏下标；空列表返回 −1。
fn nearest_index(rects: &[Rect], x: f64, y: f64) -> i32 {
    let mut best = -1;
    let mut best_d = f64::INFINITY;
    for (i, r) in rects.iter().enumerate() {
        let d = dist_to_rect_sq(r, x, y);
        if d < best_d {
            best_d = d;
            best = i as i32;
        }
    }
    best
}

/// 由工作区矩形计算抛掷边界：身体（盒内缩 side_allow）贴屏幕边缘才反弹。
pub fn throw_bounds_in(area: &Rect, size: f64, side_allow: f64) -> ThrowBounds {
    let h = size * 9.0 / 16.0;
    ThrowBounds {
        min_x: area.x - side_allow,
        min_y: area.y,
        max_x: rect_right(area) - size + side_allow,
        max_y: rect_bottom(area) - h,
    }
}

/// 由工作区列表构造抛掷空间（单块屏时等价于单屏 throwBounds）。
pub fn throw_space(areas: Vec<Rect>, panels: Option<Vec<Rect>>, size: f64, side_allow: f64) -> ThrowSpace {
    let bounds = areas.iter().map(|a| throw_bounds_in(a, size, side_allow)).collect();
    let panels = match panels {
        Some(p) if p.len() == areas.len() => p,
        _ => areas.clone(), // 面板缺失或不同序：退化用工作区
    };
    ThrowSpace { bounds, areas, panels, size, side_allow }
}

/// 宠物当前该归属哪块屏：身体中心所在屏，落在空洞里时取最近的一块。
pub fn screen_of_box(space: &ThrowSpace, x: f64, y: f64) -> i32 {
    let cx = x + space.size / 2.0;
    let cy = y + (space.size * 9.0 / 16.0) / 2.0;
    let hit = index_at_point(&space.areas, cx, cy);
    if hit >= 0 {
        hit
    } else {
        nearest_index(&space.areas, cx, cy)
    }
}

/// 落地冲击速度 → 下压幅度（速度越大压得狠），结果作为 squash_scale 的 squash 参数。
pub fn landing_squash(impact_speed: f64) -> f64 {
    let t = ((impact_speed.abs() - SQ_SOFT_SPEED) / (SQ_HARD_SPEED - SQ_SOFT_SPEED)).clamp(0.0, 1.0);
    (1.0 - t * (1.0 - SQ_MAX_SQUASH)).min(0.8)
}

/// Q 弹挤压曲线：u∈[0,1] 进度 → scaleY。
/// 下压段（0~0.45）ease-in 压缩，回弹段（0.45~1）easeOutBack 带回约 4% 过冲，两端共用同一曲线。
pub fn squash_scale(u: f64, squash: f64) -> f64 {
    if u < 0.45 {
        let p = u / 0.45;
        1.0 - (1.0 - squash) * p * p
    } else {
        let p = (u - 0.45) / 0.55;
        // easeOutBack 的标准过冲系数
        let c1 = 1.70158;
        let c3 = c1 + 1.0;
        let f = 1.0 + c3 * (p - 1.0).powi(3) + c1 * (p - 1.0).powi(2);
        (squash + (1.0 - squash) * f.max(0.0)).min(1.12)
    }
}

/// 过阻尼弹簧单轴速度步进：调用方随后 x += v*dt。
/// power = 总力度（K/C 同乘：系统形态不变，收敛速度快 p 倍）。
pub fn spring_step(v: f64, x: f64, target: f64, dt: f64, power: f64) -> f64 {
    v + ((target - x) * SPRING_K - v * SPRING_C) * power * dt
}

/// 软钳速：cap×(1-e^(-s/cap))，任意速度下单调可区分、渐近不超过 cap。
pub fn soft_clamp_speed(speed: f64) -> f64 {
    if speed <= 0.0 {
        return 0.0;
    }
    MAX_THROW_SPEED * (1.0 - (-speed / MAX_THROW_SPEED).exp())
}

/// 拉距 → 力度倍率（px / 参考拉距）：线性、单调、不夹取；0 拉距 = 0 力度。
pub fn slingshot_power(dist: f64, size: f64) -> f64 {
    let ref_dist = if size > 0.0 { size * SLINGSHOT_REF_RATIO } else { 0.0 };
    if !(ref_dist > 0.0) || !(dist > 0.0) || !dist.is_finite() {
        return 0.0;
    }
    dist / ref_dist
}

/// 拉向量（起点 → 松手点，屏幕坐标，y 向下）→ 弹射角度（度）：弹的是拉的反方向。
pub fn pull_angle(pull_x: f64, pull_y: f64) -> f64 {
    pull_y.atan2(-pull_x).to_degrees()
}

/// 弹弓初速：角度约定与力度锚点同抛掷指令，但没有软上限（任意 power 都单调可区分）。
pub fn slingshot_velocity(angle_deg: f64, power: f64, throw_power: f64) -> (f64, f64) {
    let speed = THROW_SPEED_ANCHOR * power.max(0.0) * throw_power.max(0.0);
    if speed == 0.0 {
        return (0.0, 0.0);
    }
    let rad = angle_deg.to_radians();
    ((rad.cos()) * speed, -(rad.sin()) * speed)
}

/// 一次轨迹采样（t = 按下/采样时刻；x/y = 指针绝对坐标 px）。
pub type DragSample = (Instant, i32, i32);

/// 由拖拽轨迹估算松手初速 (vx, vy)，px/s。
/// 方向取窗口首末端点的位移（抗抖）；大小 = 端点平均与窗口峰值按 PEAK_WEIGHT 加权，
/// 末段仍在加速时按 ACCEL_REF 给增益（最多 ACCEL_GAIN_MAX），再过软钳速。
/// 软钳速之后整体乘 physics.throw_power。
/// 返回 None = 温柔放下（轨迹为空 / 停留过久 / 窗口太短 / 峰值低于死区），调用方不抛。
pub fn estimate_release_velocity(
    trail: &[DragSample],
    now: Instant,
    physics: &PhysicsParams,
) -> Option<(f64, f64)> {
    if trail.is_empty() {
        return None;
    }
    let last = trail[trail.len() - 1];
    if now.saturating_duration_since(last.0).as_secs_f64() * 1000.0 > RELEASE_STALE_MS {
        return None;
    }
    let win: Vec<DragSample> = trail
        .iter()
        .copied()
        .filter(|(t, _, _)| now.saturating_duration_since(*t).as_secs_f64() * 1000.0 <= RELEASE_WINDOW_MS)
        .collect();
    if win.len() < 2 {
        return None;
    }
    let (t0, x0, y0) = win[0];
    let (t1, x1, y1) = win[win.len() - 1];
    let span_ms = t1.duration_since(t0).as_secs_f64() * 1000.0;
    if span_ms < MIN_SPAN_MS {
        return None;
    }
    let base_vx = (x1 - x0) as f64 / span_ms * 1000.0;
    let base_vy = (y1 - y0) as f64 / span_ms * 1000.0;
    let base_speed = (base_vx * base_vx + base_vy * base_vy).sqrt();
    if base_speed < 1e-6 {
        return None; // 窗口内几乎纯抖动：没有可靠方向，按原地放下处理
    }
    // 分段速度（过密采样向前合并，dt 下限 SEG_MIN_DT_MS）
    let mut segs: Vec<(f64, Instant)> = Vec::new(); // (speed, t_end)
    let (mut px, mut py, mut pt) = (x0 as f64, y0 as f64, t0);
    for &(t, x, y) in &win[1..] {
        let dt_ms = t.duration_since(pt).as_secs_f64() * 1000.0;
        if dt_ms >= SEG_MIN_DT_MS {
            let d = ((x as f64 - px).powi(2) + (y as f64 - py).powi(2)).sqrt();
            segs.push((d / dt_ms * 1000.0, t));
            px = x as f64;
            py = y as f64;
            pt = t;
        }
    }
    let peak_speed = if segs.is_empty() {
        base_speed
    } else {
        segs.iter().map(|(s, _)| *s).fold(0.0f64, f64::max)
    };
    // 末段加速度：末段峰值速度相对首段的斜率，仍在加速的甩动据此放大初速
    let mut accel = 0.0;
    if segs.len() >= 2 {
        let (last_speed, last_t) = segs[segs.len() - 1];
        let (first_speed, first_t) = segs[0];
        let denom = last_t.duration_since(first_t).as_secs_f64().max(MIN_SPAN_MS / 1000.0);
        accel = (last_speed - first_speed) / denom;
    }
    let speed_before_clamp = ((1.0 - PEAK_WEIGHT) * base_speed + PEAK_WEIGHT * peak_speed)
        * (1.0 + (accel.max(0.0) / ACCEL_REF).min(1.0) * ACCEL_GAIN_MAX);
    let speed = soft_clamp_speed(speed_before_clamp) * physics.throw_power;
    if speed < DEAD_ZONE_SPEED {
        return None;
    }
    Some((base_vx / base_speed * speed, base_vy / base_speed * speed))
}

/// 多屏抛掷步进：边界取自身体中心所在屏，越界后多问一句越界侧有没有屏。
/// 越界侧有别的屏就原样放行（自然跨屏飞行），什么都没有才按该边反弹。
/// 探测用完整面板而不是工作区：任务栏位于屏缝时按工作区探测会把接缝当墙。
/// lock_screen >= 0 表示锁定该屏（confine_to_screen）：边界恒取该屏、跳过邻屏探测；
/// 索引越界（飞行途中拔掉显示器）即退回不锁定。
pub fn throw_step_region(
    s: ThrowState,
    dt_raw: f64,
    space: &ThrowSpace,
    physics: &PhysicsParams,
    lock_screen: i32,
) -> StepOutcome {
    let dt = dt_raw.clamp(0.0, MAX_STEP_DT);
    let mut st = ThrowState { x: s.x, y: s.y, vx: s.vx, vy: s.vy };
    st.vy += physics.gravity * dt;
    st.x += st.vx * dt;
    st.y += st.vy * dt;
    if space.areas.is_empty() {
        return StepOutcome { state: st, screen: -1, bounced: false, at_rest: false };
    }

    let h = space.size * 9.0 / 16.0;
    let mut bounced = false;
    let locked = lock_screen >= 0 && (lock_screen as usize) < space.areas.len();
    // 该用哪块屏的边界/工作区/面板：锁定时恒为锁定屏，否则取身体中心所在屏
    let pick = |i: i32| -> usize { if locked { lock_screen as usize } else { i.max(0) as usize } };

    // 横向：边界来自身体中心所在屏，放行与否看缝外同高度处有没有像素
    let mut cur = screen_of_box(space, st.x, st.y);
    let mut b = space.bounds[pick(cur)];
    let mut pa = space.panels[pick(cur)];
    let cy = st.y + h / 2.0;
    if st.x < b.min_x {
        if locked || index_at_point(&space.panels, pa.x - 1.0, cy) < 0 {
            st.x = b.min_x;
            st.vx = st.vx.abs() * physics.restitution;
            bounced = true;
        }
    } else if st.x > b.max_x {
        if locked || index_at_point(&space.panels, rect_right(&pa), cy) < 0 {
            st.x = b.max_x;
            st.vx = -st.vx.abs() * physics.restitution;
            bounced = true;
        }
    }

    // 纵向：横向若被夹回，身体中心可能已换屏，重新定位
    cur = screen_of_box(space, st.x, st.y);
    b = space.bounds[pick(cur)];
    pa = space.panels[pick(cur)];
    let cx = st.x + space.size / 2.0;
    if st.y < b.min_y {
        // ceiling_bounce=false：顶部无边界，不夹不弹——宠物飞出屏幕顶部，靠重力落回
        if physics.ceiling_bounce && (locked || index_at_point(&space.panels, cx, pa.y - 1.0) < 0) {
            st.y = b.min_y;
            st.vy = st.vy.abs() * physics.restitution;
            bounced = true;
        }
    } else if st.y >= b.max_y {
        // 下方还有一块屏（上下排布的桌面）⇒ 不当地面，继续往下掉
        if locked || index_at_point(&space.panels, cx, rect_bottom(&pa)) < 0 {
            st.y = b.max_y;
            st.vx *= (1.0 - physics.ground_friction * dt).max(0.0);
            if st.vy.abs() < REST_VY {
                st.vy = 0.0;
            } else {
                st.vy = -st.vy.abs() * physics.restitution;
            }
            bounced = true;
        }
    }

    cur = pick(screen_of_box(space, st.x, st.y)) as i32;
    b = space.bounds[cur as usize];
    let speed = (st.vx * st.vx + st.vy * st.vy).sqrt();
    let at_rest = (st.y >= b.max_y - 1.0 && st.vy.abs() < 1.0 && st.vx.abs() < REST_VX)
        || (bounced && speed < REST_VY && st.vy.abs() < 1.0);
    StepOutcome { state: st, screen: cur, bounced, at_rest }
}

/// 一帧该积几步（整数 ≥ 1）。N = 1 是常态；弹弓拉得很远时一帧位移上百 px，
/// 不拆会跳过落地与反弹那几帧。dt 先按 MAX_STEP_DT 夹取，保证步数算的就是实际要积的时间。
pub fn throw_substeps(vx: f64, vy: f64, dt: f64) -> u32 {
    let travel = (vx * vx + vy * vy).sqrt() * dt.clamp(0.0, MAX_STEP_DT);
    if !(travel > SUBSTEP_PX) {
        return 1;
    }
    SUBSTEP_MAX.min((travel / SUBSTEP_PX).ceil() as u32)
}
