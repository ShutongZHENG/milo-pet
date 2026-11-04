//! 动画控制器：命名剪辑 + 帧推进 + 对齐帧过渡。
//! 取代 main.rs 里写死的 `enum Anim { Idle, Sleep }` 与睡眠对齐 hack。
//! 加新动画 = 注册一个剪辑 + （可选）排入一条过渡。

use std::collections::HashMap;

/// 常用剪辑名。
pub const CLIP_IDLE: &str = "idle";
pub const CLIP_SLEEP: &str = "sleep";

/// 一个动画剪辑的元信息（不持有像素数据）。
#[derive(Clone, Copy, Debug)]
pub struct Clip {
    /// 帧数
    pub count: u32,
    /// 帧率
    pub fps: u32,
}

/// 对齐帧过渡：当前剪辑播到 `at_frame` 帧时切到 `to` 剪辑，从 `start_frame` 起播。
#[derive(Clone, Debug)]
pub struct Transition {
    pub to: String,
    /// 触发帧号；-1 = 下一帧立即切（不对齐）
    pub at_frame: i64,
    /// 目标剪辑起始帧
    pub start_frame: u32,
}

/// 每帧推进结果。
#[derive(Clone, Debug)]
pub struct FrameUpdate {
    /// 当前剪辑名
    pub clip: String,
    /// 当前帧号（用于解码）
    pub frame: usize,
    /// 是否换了帧（false = 与上一帧相同，可跳过解码）
    pub changed: bool,
    /// 本帧刚触发的过渡（若有），供上层日志/副作用
    pub fired: Option<Transition>,
}

/// 命名剪辑动画状态机。
pub struct Animator {
    clips: HashMap<String, Clip>,
    current: String,
    /// 当前剪辑内时间（秒），已对剪辑时长取模
    t: f64,
    /// 上次解码帧号；-1 = 需强制重解
    last_frame: i64,
    /// 待执行的对齐帧过渡（同一时刻最多一条）
    pending: Option<Transition>,
}

impl Animator {
    pub fn new() -> Self {
        Animator {
            clips: HashMap::new(),
            current: String::new(),
            t: 0.0,
            last_frame: -1,
            pending: None,
        }
    }

    /// 注册一个命名剪辑。
    pub fn register(&mut self, name: impl Into<String>, clip: Clip) {
        self.clips.insert(name.into(), clip);
    }

    /// 立即切换到剪辑，从第 0 帧起播。未知剪辑返回 false。
    pub fn play(&mut self, name: &str) -> bool {
        self.play_from(name, 0)
    }

    /// 立即切换到剪辑，从 `start_frame` 帧起播。未知剪辑返回 false。
    pub fn play_from(&mut self, name: &str, start_frame: u32) -> bool {
        match self.clips.get(name).copied() {
            Some(clip) => {
                self.current = name.to_string();
                self.t = (start_frame as f64 / clip.fps as f64)
                    % (clip.count as f64 / clip.fps as f64);
                self.last_frame = -1;
                self.pending = None;
                true
            }
            None => false,
        }
    }

    /// 排入一条对齐帧过渡（覆盖上一条）。
    pub fn schedule(&mut self, transition: Transition) {
        self.pending = Some(transition);
    }

    /// 是否有待执行的过渡。
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 取消待执行的过渡。
    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// 当前剪辑名。
    pub fn current(&self) -> &str {
        &self.current
    }

    /// 推进 dt 秒，返回当前帧信息；无当前剪辑返回 None。
    pub fn tick(&mut self, dt: f64) -> Option<FrameUpdate> {
        let clip = *self.clips.get(&self.current)?;
        let dur = clip.count as f64 / clip.fps as f64;
        self.t = (self.t + dt) % dur;
        let frame_secs = 1.0 / clip.fps as f64;
        let mut frame_idx = (self.t / frame_secs) as i64 % clip.count as i64;

        // 对齐帧过渡：当前帧到达触发帧（或 -1 立即）才切，未到则保留待下次
        let fire = matches!(&self.pending, Some(tr) if tr.at_frame < 0 || frame_idx == tr.at_frame);
        let mut fired = None;
        if fire {
            if let Some(tr) = self.pending.take() {
                if let Some(target) = self.clips.get(&tr.to).copied() {
                    self.current = tr.to.clone();
                    self.t = (tr.start_frame as f64 / target.fps as f64)
                        % (target.count as f64 / target.fps as f64);
                    self.last_frame = -1;
                    frame_idx = (self.t * target.fps as f64) as i64 % target.count as i64;
                    fired = Some(tr);
                }
            }
        }

        let changed = frame_idx != self.last_frame;
        if changed {
            self.last_frame = frame_idx;
        }
        Some(FrameUpdate {
            clip: self.current.clone(),
            frame: frame_idx as usize,
            changed,
            fired,
        })
    }
}

impl Default for Animator {
    fn default() -> Self {
        Self::new()
    }
}
