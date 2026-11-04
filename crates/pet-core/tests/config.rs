//! pet-core 配置读写往返测试。

use pet_core::config::{Config, Corner};
use pet_core::physics::PhysicsParams;

#[test]
fn save_load_roundtrip() {
    let cfg = Config {
        width: 400.0,
        corner: Corner::BottomLeft,
        margin_x: 10.0,
        margin_y: 20.0,
        physics: PhysicsParams::default(),
        confine_to_screen: true,
        sleep_after: 15.0,
        sleep_entry_frame: 100,
        sleep_start_frame: 5,
    };
    let path = std::env::temp_dir().join("milo-pet-test-config.toml");
    let path_str = path.to_str().unwrap();
    cfg.save(path_str).unwrap();

    let loaded = Config::load(path_str);
    assert_eq!(loaded.width, 400.0);
    assert_eq!(loaded.corner, Corner::BottomLeft);
    assert_eq!(loaded.margin_x, 10.0);
    assert_eq!(loaded.margin_y, 20.0);
    assert_eq!(loaded.confine_to_screen, true);
    assert_eq!(loaded.sleep_after, 15.0);
    assert_eq!(loaded.sleep_entry_frame, 100);
    assert_eq!(loaded.sleep_start_frame, 5);
    assert_eq!(loaded.physics.gravity, PhysicsParams::default().gravity);

    let _ = std::fs::remove_file(path);
}
