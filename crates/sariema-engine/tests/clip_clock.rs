// SPDX-License-Identifier: Apache-2.0

//! The runtime owns a clip's clock and the animator stands where it says:
//! a looping clip, slowed and sped back up with the world and restarted at
//! another speed, is at the runtime's clip time at every step.

mod common;

use std::time::Duration;

use sariema_anim::PlayMode;
use sariema_format::script::{ClipMode, Command, name_id};
use sariema_runtime::input::RawInput;

use common::{engine, entity, native, send, spawn_hero, wired};

fn wave(speed: f32) -> Command<'static> {
    Command::PlayClip {
        entity: entity(0),
        clip: name_id("wave"),
        mode: ClipMode::Loop,
        blend: 0.2,
        speed,
    }
}

fn time_scale(scale: f32) -> Command<'static> {
    Command::TimeScale { scale, blend: 0.3 }
}

#[test]
fn the_animator_stands_at_the_runtimes_clip_time_through_time_scale_changes() {
    let logic = native(|n, _, out| match n {
        0 => spawn_hero(out, [1.5, 0.0, -0.6], 0.0),
        2 => send(out, wave(1.3)),
        60 => send(out, time_scale(0.4)),
        200 => send(out, time_scale(1.0)),
        260 => send(out, wave(0.7)),
        _ => {}
    });
    let mut engine = engine(logic, wired(), &[]);
    let raw = RawInput::default();
    let model = &engine.animation().models[0];
    let clip = model.rig.clip("wave").expect("the hero has a wave");
    let duration = model.rig.clip_info(clip).duration;
    let mut compared = 0;
    while engine.steps() < 420 {
        engine.frame(Duration::from_secs_f64(1.0 / 60.0), &raw);
        let Some((_, t)) = engine.state().clip_time(entity(0)) else {
            continue;
        };
        let want = PlayMode::Loop.clip_time(f64::from(t), duration) as f32;
        let (_, _, at) = engine.animation().clip_times().next().unwrap();
        assert_eq!(
            at,
            want,
            "the animator's clip time at step {}",
            engine.steps()
        );
        compared += 1;
    }
    assert!(compared > 300, "{compared}");
}
