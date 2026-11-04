//! 空闲入睡插件：空闲超阈值 → 排入对齐帧过渡（等待机播到对齐帧再切睡）；
//! 响应 `Wake` / `ForceSleep` 事件做唤醒与强制入睡。

use std::time::Instant;

use crate::animator::{Transition, CLIP_IDLE, CLIP_SLEEP};
use crate::event::PetEvent;
use crate::plugin::{PetWorld, Plugin};

pub struct IdleSleepPlugin {
    sleep_after: f64,
    sleep_entry_frame: i64,
    sleep_start_frame: u32,
    /// 是否注册了睡觉剪辑（sleep.bin 存在且尺寸匹配）
    has_sleep: bool,
    debug: bool,
}

impl IdleSleepPlugin {
    pub fn new(
        sleep_after: f64,
        sleep_entry_frame: i64,
        sleep_start_frame: u32,
        has_sleep: bool,
        debug: bool,
    ) -> Self {
        IdleSleepPlugin { sleep_after, sleep_entry_frame, sleep_start_frame, has_sleep, debug }
    }
}

impl Plugin for IdleSleepPlugin {
    fn name(&self) -> &'static str {
        "idle-sleep"
    }

    fn on_tick(
        &mut self,
        world: &mut PetWorld,
        now: Instant,
        _dt: f64,
        events: &mut Vec<PetEvent>,
    ) {
        // 空闲超阈值且已落稳（不拖拽、速度接近 0）才排入对齐帧过渡。
        // 真正切换要等到待机播到 sleep_entry_frame 那一帧，保证衔接处两帧尽量相似。
        if world.animator.current() == CLIP_IDLE
            && self.has_sleep
            && !world.pet.dragging
            && world.pet.vx.abs() < 1.0
            && world.pet.vy.abs() < 1.0
            && now.duration_since(world.last_active).as_secs_f64() >= self.sleep_after
            && !world.animator.is_pending()
        {
            world.animator.schedule(Transition {
                to: CLIP_SLEEP.to_string(),
                at_frame: self.sleep_entry_frame,
                start_frame: self.sleep_start_frame,
            });
            events.push(PetEvent::IdleTimeout);
            if self.debug && self.sleep_entry_frame >= 0 {
                eprintln!(
                    "DEBUG sleep pending (idle {:.1}s), waiting idle frame {}",
                    self.sleep_after, self.sleep_entry_frame
                );
            }
        }
    }

    fn on_event(
        &mut self,
        world: &mut PetWorld,
        ev: &PetEvent,
        events: &mut Vec<PetEvent>,
    ) {
        match ev {
            PetEvent::Wake => {
                world.last_active = Instant::now();
                world.animator.clear_pending();
                if world.animator.current() != CLIP_IDLE {
                    world.animator.play(CLIP_IDLE);
                    events.push(PetEvent::Woke);
                    if self.debug {
                        eprintln!("DEBUG anim -> Idle (wake)");
                    }
                }
            }
            PetEvent::ForceSleep => {
                if self.has_sleep && world.animator.current() != CLIP_SLEEP {
                    world.animator.play_from(CLIP_SLEEP, self.sleep_start_frame);
                    events.push(PetEvent::SleepEntered);
                    if self.debug {
                        eprintln!(
                            "DEBUG anim -> Sleep (menu, start frame {})",
                            self.sleep_start_frame
                        );
                    }
                }
            }
            _ => {}
        }
    }
}
