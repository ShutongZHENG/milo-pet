//! pet-core 空闲入睡插件单元测试。

use std::time::{Duration, Instant};

use pet_core::animator::{Animator, Clip, CLIP_IDLE, CLIP_SLEEP};
use pet_core::event::PetEvent;
use pet_core::physics::{Pet, PhysicsParams, ThrowSpace};
use pet_core::plugin::{PetWorld, Plugin};
use pet_core::plugins::idle_sleep::IdleSleepPlugin;

fn world() -> PetWorld {
    let mut animator = Animator::new();
    animator.register(CLIP_IDLE, Clip { count: 100, fps: 25 });
    animator.register(CLIP_SLEEP, Clip { count: 50, fps: 10 });
    animator.play(CLIP_IDLE);
    PetWorld {
        pet: Pet::default(),
        animator,
        last_active: Instant::now(),
        physics: PhysicsParams::default(),
        space: ThrowSpace {
            bounds: vec![],
            areas: vec![],
            panels: vec![],
            size: 0.0,
            side_allow: 0.0,
        },
        dpi_scale: 1.0,
        confine: false,
    }
}

#[test]
fn schedules_transition_after_idle() {
    let mut w = world();
    w.last_active = Instant::now() - Duration::from_secs(60);
    let mut p = IdleSleepPlugin::new(30.0, 261, 33, true, false);
    let mut events = Vec::new();
    p.on_tick(&mut w, Instant::now(), 0.0, &mut events);
    assert!(w.animator.is_pending());
    assert!(events.iter().any(|e| matches!(e, PetEvent::IdleTimeout)));
}

#[test]
fn no_transition_when_active() {
    let mut w = world();
    let mut p = IdleSleepPlugin::new(30.0, 261, 33, true, false);
    let mut events = Vec::new();
    p.on_tick(&mut w, Instant::now(), 0.0, &mut events);
    assert!(!w.animator.is_pending());
    assert!(events.is_empty());
}

#[test]
fn wake_event_resets_and_plays_idle() {
    let mut w = world();
    w.animator.play_from(CLIP_SLEEP, 5);
    let mut p = IdleSleepPlugin::new(30.0, 261, 33, true, false);
    let mut events = Vec::new();
    p.on_event(&mut w, &PetEvent::Wake, &mut events);
    assert_eq!(w.animator.current(), CLIP_IDLE);
    assert!(events.iter().any(|e| matches!(e, PetEvent::Woke)));
}

#[test]
fn force_sleep_event_plays_sleep() {
    let mut w = world();
    let mut p = IdleSleepPlugin::new(30.0, 261, 33, true, false);
    let mut events = Vec::new();
    p.on_event(&mut w, &PetEvent::ForceSleep, &mut events);
    assert_eq!(w.animator.current(), CLIP_SLEEP);
    assert!(events.iter().any(|e| matches!(e, PetEvent::SleepEntered)));
}

#[test]
fn force_sleep_noop_without_sleep_clip() {
    let mut w = world();
    let mut p = IdleSleepPlugin::new(30.0, 261, 33, false, false);
    let mut events = Vec::new();
    p.on_event(&mut w, &PetEvent::ForceSleep, &mut events);
    assert_eq!(w.animator.current(), CLIP_IDLE);
    assert!(events.is_empty());
}
