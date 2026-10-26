//! 抛掷物理：重力、地面反弹、摩擦、左右边界反弹（参照 dsh-pet 默认手感）。

pub const GRAVITY: f64 = 2600.0; // px/s²
pub const RESTITUTION: f64 = 0.55; // 反弹恢复系数
pub const GROUND_FRICTION: f64 = 3.5; // 地面摩擦 /s
pub const REST_THRESHOLD: f64 = 40.0; // 低于此速度视为停稳

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
    pub on_ground: bool,
}

impl Pet {
    pub fn step(&mut self, dt: f64, ground_y: f64, screen_w: f64) {
        if self.dragging {
            return;
        }
        self.vy += GRAVITY * dt;
        self.x += self.vx * dt;
        self.y += self.vy * dt;

        let bottom = self.y + self.h as f64;
        if bottom >= ground_y {
            self.y = ground_y - self.h as f64;
            if self.vy > 0.0 {
                if self.vy.abs() > REST_THRESHOLD {
                    self.vy = -self.vy * RESTITUTION;
                } else {
                    self.vy = 0.0;
                }
            }
            self.on_ground = true;
        } else {
            self.on_ground = false;
        }
        if self.on_ground {
            self.vx *= (1.0 - GROUND_FRICTION * dt).max(0.0);
            if self.vx.abs() < 12.0 {
                self.vx = 0.0;
            }
        }

        if self.x < 0.0 {
            self.x = 0.0;
            self.vx = self.vx.abs() * RESTITUTION;
        }
        let right = self.x + self.w as f64;
        if right > screen_w {
            self.x = screen_w - self.w as f64;
            self.vx = -self.vx.abs() * RESTITUTION;
        }
    }

    pub fn throw(&mut self, vx: f64, vy: f64) {
        self.vx = vx;
        self.vy = vy;
        self.on_ground = false;
        self.dragging = false;
    }
}
