//! pet-core 动画控制器单元测试。

use pet_core::animator::{Animator, Clip, Transition};

fn idle() -> Clip {
    Clip { count: 100, fps: 25 }
}
fn sleep() -> Clip {
    Clip { count: 50, fps: 10 }
}

#[test]
fn tick_advances_frames() {
    let mut a = Animator::new();
    a.register("idle", idle());
    assert!(a.play("idle"));
    let u = a.tick(0.04).unwrap();
    assert_eq!(u.clip, "idle");
    assert_eq!(u.frame, 1);
    assert!(u.changed);
}

#[test]
fn play_from_starts_at_frame() {
    let mut a = Animator::new();
    a.register("idle", idle());
    assert!(a.play_from("idle", 10));
    let u = a.tick(0.0).unwrap();
    assert_eq!(u.frame, 10);
    assert!(u.changed);
}

#[test]
fn play_unknown_clip_fails() {
    let mut a = Animator::new();
    assert!(!a.play("nope"));
}

#[test]
fn aligned_transition_fires_at_frame() {
    let mut a = Animator::new();
    a.register("idle", idle());
    a.register("sleep", sleep());
    a.play("idle");
    a.schedule(Transition { to: "sleep".into(), at_frame: 5, start_frame: 3 });

    let mut last = None;
    for _ in 0..5 {
        last = a.tick(0.04);
    }
    assert_eq!(a.current(), "sleep");
    let u = last.unwrap();
    assert_eq!(u.clip, "sleep");
    assert_eq!(u.frame, 3);
    assert!(u.changed);
    assert!(u.fired.is_some());
}

#[test]
fn immediate_transition_fires_next_tick() {
    let mut a = Animator::new();
    a.register("idle", idle());
    a.register("sleep", sleep());
    a.play("idle");
    a.schedule(Transition { to: "sleep".into(), at_frame: -1, start_frame: 0 });
    let u = a.tick(0.04).unwrap();
    assert_eq!(u.clip, "sleep");
    assert!(u.fired.is_some());
}

#[test]
fn pending_transition_wait_until_frame() {
    let mut a = Animator::new();
    a.register("idle", idle());
    a.register("sleep", sleep());
    a.play("idle");
    a.schedule(Transition { to: "sleep".into(), at_frame: 10, start_frame: 0 });
    for _ in 0..3 {
        a.tick(0.04);
    }
    assert_eq!(a.current(), "idle");
    assert!(a.is_pending());
}

#[test]
fn changed_flag_false_when_frame_unchanged() {
    let mut a = Animator::new();
    a.register("idle", idle());
    a.play("idle");
    assert!(a.tick(0.0).unwrap().changed);
    assert!(!a.tick(0.0).unwrap().changed);
}
