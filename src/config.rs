//! 配置：assets/config.toml（缺省字段回落内置默认）。

use std::fs;

use crate::physics::PhysicsParams;

pub struct Config {
    /// 窗口宽度（逻辑像素，按 DPI 缩放）
    pub width: f64,
    pub corner: Corner,
    pub margin_x: f64,
    pub margin_y: f64,
    /// 抛掷物理参数（[physics] 段，缺省回落 dsh 对标默认值）
    pub physics: PhysicsParams,
    /// 锁定当前屏幕：抛掷只在所在屏内活动，不跨屏（根字段，不属 [physics]）
    pub confine_to_screen: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

fn num(v: Option<&toml::Value>) -> Option<f64> {
    v.and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
}

impl Config {
    pub fn load(path: &str) -> Self {
        let mut cfg = Config {
            width: 350.0,
            corner: Corner::TopRight,
            margin_x: 24.0,
            margin_y: 100.0,
            physics: PhysicsParams::default(),
            confine_to_screen: false,
        };
        let Ok(text) = fs::read_to_string(path) else {
            return cfg;
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            return cfg;
        };
        if let Some(w) = num(value.get("width")) {
            if w > 0.0 && w < 2000.0 {
                cfg.width = w;
            }
        }
        if let Some(c) = value.get("corner").and_then(|v| v.as_str()) {
            cfg.corner = match c {
                "top-left" => Corner::TopLeft,
                "bottom-right" => Corner::BottomRight,
                "bottom-left" => Corner::BottomLeft,
                _ => Corner::TopRight,
            };
        }
        if let Some(m) = value.get("margin_x").and_then(|v| v.as_float()) {
            cfg.margin_x = m.max(0.0);
        }
        if let Some(m) = value.get("margin_y").and_then(|v| v.as_float()) {
            cfg.margin_y = m.max(0.0);
        }
        if let Some(c) = value.get("confine_to_screen").and_then(|v| v.as_bool()) {
            cfg.confine_to_screen = c;
        }
        if let Some(p) = value.get("physics") {
            if let Some(g) = num(p.get("gravity")) {
                if g >= 0.0 {
                    cfg.physics.gravity = g;
                }
            }
            if let Some(r) = num(p.get("restitution")) {
                if (0.0..=1.0).contains(&r) {
                    cfg.physics.restitution = r;
                }
            }
            if let Some(f) = num(p.get("ground_friction")) {
                if f >= 0.0 {
                    cfg.physics.ground_friction = f;
                }
            }
            if let Some(b) = p.get("ceiling_bounce").and_then(|v| v.as_bool()) {
                cfg.physics.ceiling_bounce = b;
            }
            if let Some(t) = num(p.get("throw_power")) {
                if t >= 0.0 {
                    cfg.physics.throw_power = t;
                }
            }
        }
        cfg
    }
}
