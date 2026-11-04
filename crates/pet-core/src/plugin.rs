//! 插件系统：功能模块通过实现 `Plugin` 挂载到宠物运行时。
//! 每个插件可响应每帧推进（`on_tick`）与离散事件（`on_event`）。
//! 加新功能 = 实现一个插件 + 注册到 `PluginRegistry`，核心循环零改动。

use std::time::Instant;

use crate::animator::Animator;
use crate::event::PetEvent;
use crate::physics::{Pet, PhysicsParams, ThrowSpace};

/// 插件共享的运行时状态（受控访问；所有权归 App，插件按需借用）。
pub struct PetWorld {
    pub pet: Pet,
    pub animator: Animator,
    /// 上次互动时刻（空闲入睡判定用）
    pub last_active: Instant,
    pub physics: PhysicsParams,
    pub space: ThrowSpace,
    pub dpi_scale: f64,
    pub confine: bool,
}

/// 功能插件。`on_tick`/`on_event` 均有默认空实现，按需覆盖。
pub trait Plugin {
    fn name(&self) -> &'static str;

    /// 每帧推进。
    fn on_tick(
        &mut self,
        _world: &mut PetWorld,
        _now: Instant,
        _dt: f64,
        _events: &mut Vec<PetEvent>,
    ) {
    }

    /// 响应一个事件。
    fn on_event(
        &mut self,
        _world: &mut PetWorld,
        _ev: &PetEvent,
        _events: &mut Vec<PetEvent>,
    ) {
    }
}

/// 插件注册表：统一调度 tick / event，并累积一帧内产生的通知事件。
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    events: Vec<PetEvent>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        PluginRegistry { plugins: Vec::new(), events: Vec::new() }
    }

    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    /// 同步派发一个命令/事件给所有插件。
    pub fn emit(&mut self, world: &mut PetWorld, ev: PetEvent) {
        for p in &mut self.plugins {
            p.on_event(world, &ev, &mut self.events);
        }
    }

    /// 推进所有插件一帧。
    pub fn tick(&mut self, world: &mut PetWorld, now: Instant, dt: f64) {
        self.events.clear();
        for p in &mut self.plugins {
            p.on_tick(world, now, dt, &mut self.events);
        }
    }

    /// 取走本帧累积的通知事件（后续桥接到 Tauri 前端）。
    pub fn drain_events(&mut self) -> Vec<PetEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
