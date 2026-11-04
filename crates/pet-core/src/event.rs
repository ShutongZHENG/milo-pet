//! 消息词汇表：内核/插件之间、以及（后续）到 Tauri 前端的事件与命令。

/// 宠物运行期事件。
#[derive(Clone, Debug, PartialEq)]
pub enum PetEvent {
    // ---- 命令（驱动插件）----
    /// 请求唤醒（点击/拖拽/甩抛/菜单叫醒）
    Wake,
    /// 请求强制入睡（菜单）
    ForceSleep,

    // ---- 通知（插件/内核发出，供日志与后续 Tauri 桥接）----
    /// 点击/拎起宠物（x,y = 按下点物理像素）
    Grabbed { x: i32, y: i32 },
    /// 松手（温柔放下或甩抛）
    Released,
    /// 抛掷指令（初速 px/s）
    Thrown { vx: f64, vy: f64 },
    /// 落地（冲击速度 px/s）
    Landed { impact: f64 },
    /// 空闲超阈值（即将入睡）
    IdleTimeout,
    /// 入睡（切到睡觉动画）
    SleepEntered,
    /// 唤醒
    Woke,
    /// 飞行中被点击命中（得分）
    Scored { score: i32, speed: f64 },
}

/// 控制宠物的命令（来自托盘/菜单/前端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PetCommand {
    /// 回到初始位置
    Reset,
    /// 强制睡觉
    Sleep,
    /// 叫醒
    Wake,
    /// 退出
    Quit,
}
