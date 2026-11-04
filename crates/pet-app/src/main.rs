// Windows GUI 应用：不显示控制台窗口
#![windows_subsystem = "windows"]

use std::sync::mpsc;

use pet_core::event::{PetCommand, PetEvent};
use windows::core::w;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

/// 独立运行入口（不带 Tauri）：创建空通道后调用宠物运行时。
fn main() {
    let (events_tx, _events_rx) = mpsc::channel::<PetEvent>();
    let (_commands_tx, commands_rx) = mpsc::channel::<PetCommand>();
    if let Err(e) = pet_app::run_pet(events_tx, commands_rx) {
        let msg = format!("Milo 启动失败: {e}");
        unsafe {
            MessageBoxW(
                None,
                &windows::core::HSTRING::from(msg.as_str()),
                w!("Milo"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}
