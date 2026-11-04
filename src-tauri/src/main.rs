//! Tauri 壳：托盘 + 宠物后台线程 + 事件/命令桥。
//! 宠物本体是原生分层窗口（跑在后台线程），Tauri 负责托盘/命令/事件桥/打包。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc;
use std::sync::Mutex;

use pet_core::config::Config;
use pet_core::event::{PetCommand, PetEvent};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};

/// 命令通道（托管状态，托盘菜单事件用它发命令给宠物线程）。
struct PetState {
    commands_tx: Mutex<mpsc::Sender<PetCommand>>,
}

/// 读取配置（设置页用）。
#[tauri::command]
fn get_config() -> Config {
    Config::load("assets/config.toml")
}

/// 写回配置（设置页用）。
#[tauri::command]
fn set_config(cfg: Config) -> Result<(), String> {
    cfg.save("assets/config.toml")
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_config, set_config])
        .setup(|app| {
            let (events_tx, events_rx) = mpsc::channel::<PetEvent>();
            let (commands_tx, commands_rx) = mpsc::channel::<PetCommand>();

            // 宠物后台线程：窗口 + 消息循环 + 渲染（独立于 Tauri 主循环）
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = pet_app::run_pet(events_tx, commands_rx) {
                    eprintln!("[milo-pet] 宠物运行时退出（错误）: {e}");
                }
                // 宠物退出 → 结束整个应用
                let _ = app_handle.exit(0);
            });

            // 事件转发线程：PetEvent → Tauri 前端（后续 M5 得分弹窗/设置页订阅）
            let app_handle2 = app.handle().clone();
            std::thread::spawn(move || {
                while let Ok(ev) = events_rx.recv() {
                    let _ = app_handle2.emit("pet-event", ev);
                }
            });

            // 命令发送端托管到状态
            app.manage(PetState { commands_tx: Mutex::new(commands_tx) });

            // 托盘菜单
            let reset_i = MenuItem::with_id(app, "reset", "回到初始位置", true, None::<&str>)?;
            let sleep_i = MenuItem::with_id(app, "sleep", "强制睡觉", true, None::<&str>)?;
            let wake_i = MenuItem::with_id(app, "wake", "叫醒", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu =
                Menu::with_items(app, &[&reset_i, &sleep_i, &wake_i, &settings_i, &sep, &quit_i])?;

            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))
                .expect("托盘图标加载失败");

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "settings" => {
                            if let Err(e) = WebviewWindowBuilder::new(
                                app,
                                "settings",
                                WebviewUrl::App("settings.html".into()),
                            )
                            .title("milo-pet 设置")
                            .inner_size(420.0, 560.0)
                            .resizable(false)
                            .build()
                            {
                                eprintln!("打开设置窗口失败: {e}");
                            }
                        }
                        id => {
                            let cmd = match id {
                                "reset" => Some(PetCommand::Reset),
                                "sleep" => Some(PetCommand::Sleep),
                                "wake" => Some(PetCommand::Wake),
                                "quit" => Some(PetCommand::Quit),
                                _ => None,
                            };
                            if let Some(cmd) = cmd {
                                let state = app.state::<PetState>();
                                let tx = state.commands_tx.lock().expect("命令通道锁中毒");
                                let _ = tx.send(cmd);
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
