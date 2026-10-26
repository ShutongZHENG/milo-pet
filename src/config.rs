//! 配置：assets/config.toml（缺省字段回落内置默认）。

use std::fs;

pub struct Config {
    /// 窗口宽度（逻辑像素，按 DPI 缩放）
    pub width: f64,
    pub corner: Corner,
    pub margin_x: f64,
    pub margin_y: f64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl Config {
    pub fn load(path: &str) -> Self {
        let mut cfg = Config {
            width: 350.0,
            corner: Corner::TopRight,
            margin_x: 24.0,
            margin_y: 100.0,
        };
        let Ok(text) = fs::read_to_string(path) else {
            return cfg;
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            return cfg;
        };
        if let Some(w) = value.get("width").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))) {
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
        cfg
    }
}
