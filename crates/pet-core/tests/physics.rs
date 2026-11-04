//! pet-core 物理模块单元测试。

use pet_core::physics::{
    slingshot_power, slingshot_velocity, soft_clamp_speed, squash_scale, throw_substeps,
    MAX_THROW_SPEED, SQ_SQUASH,
};

#[test]
fn substeps_default_to_one_when_slow() {
    assert_eq!(throw_substeps(0.0, 0.0, 1.0 / 60.0), 1);
    assert_eq!(throw_substeps(10.0, 10.0, 1.0 / 60.0), 1);
}

#[test]
fn substeps_scale_with_travel() {
    // 2000 px/s × 16ms = 32px 位移 → 32 / 8 = 4 步
    assert_eq!(throw_substeps(2000.0, 0.0, 0.016), 4);
}

#[test]
fn soft_clamp_is_monotonic_and_bounded() {
    let mut prev = 0.0;
    for i in 1..=100 {
        let s = soft_clamp_speed(i as f64 * 50.0);
        assert!(s > prev, "soft_clamp_speed 应单调递增");
        assert!(s < MAX_THROW_SPEED, "软钳速应始终小于上限");
        prev = s;
    }
    assert_eq!(soft_clamp_speed(0.0), 0.0);
    assert_eq!(soft_clamp_speed(-100.0), 0.0);
}

#[test]
fn squash_scale_endpoints() {
    assert_eq!(squash_scale(0.0, SQ_SQUASH), 1.0);
    assert_eq!(squash_scale(1.0, SQ_SQUASH), 1.0);
    // 下压段中段应介于 1.0 与 squash 之间
    let mid = squash_scale(0.2, SQ_SQUASH);
    assert!(mid < 1.0);
    assert!(mid > SQ_SQUASH);
}

#[test]
fn slingshot_zero_for_no_pull() {
    assert_eq!(slingshot_power(0.0, 350.0), 0.0);
    assert_eq!(slingshot_velocity(45.0, 0.0, 1.0), (0.0, 0.0));
}
