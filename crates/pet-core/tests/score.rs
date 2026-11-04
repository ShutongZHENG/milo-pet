//! pet-core 积分模块单元测试。

use pet_core::score::click_score;

#[test]
fn click_score_scales_with_speed() {
    assert_eq!(click_score(100.0, 462.0), 1);
    assert_eq!(click_score(200.0, 462.0), 2);
    assert_eq!(click_score(300.0, 462.0), 3);
}

#[test]
fn click_score_smaller_pet_scores_more() {
    let big = click_score(200.0, 462.0);
    let small = click_score(200.0, 231.0);
    assert!(small > big);
}

#[test]
fn click_score_floor_of_one() {
    // 只要速度 > 0，至少 1 分
    assert_eq!(click_score(1.0, 462.0), 1);
}

#[test]
fn click_score_zero_for_invalid_input() {
    assert_eq!(click_score(0.0, 462.0), 0);
    assert_eq!(click_score(-10.0, 462.0), 0);
    assert_eq!(click_score(100.0, 0.0), 0);
}
