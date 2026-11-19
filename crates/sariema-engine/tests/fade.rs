// SPDX-License-Identifier: Apache-2.0

//! A teleport under a full fade, given in the same step as the fade back
//! out: the fade is drawn at a step's value at every point between steps,
//! never between two, and the frames that show the step it moved in are
//! fully covered.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_format::script::{Command, Event};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

const FROM: [f32; 3] = [1.5, 0.0, -0.6];
const TO: [f32; 3] = [2.5, 0.0, -1.25];

#[test]
fn a_fade_covered_teleport_is_never_drawn_at_an_intermediate_fade_value() {
    let refused = Rc::new(RefCell::new(Vec::new()));
    let seen = refused.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::Rejected { code, .. } = e {
                seen.borrow_mut().push((n, code.to_string()));
            }
        }
        let fade = |to: f32| Command::Fade {
            to,
            duration: 0.3,
            color: [0.0; 3],
        };
        match n {
            0 => spawn_hero(out, FROM, 0.0),
            5 => send(out, fade(1.0)),
            30 => {
                send(
                    out,
                    Command::Teleport {
                        entity: entity(0),
                        position: TO,
                        yaw: 90.0,
                        force: false,
                        hold: false,
                    },
                );
                send(out, fade(0.0));
            }
            _ => {}
        }
    });
    let mut engine = engine(logic, wired(), &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let (mut landed, mut before) = (None, 0.0_f32);
    for n in 0..70 {
        engine.step(&actions);
        let now = engine.state().screen.fade.value();
        let step_fade = now.max(before);
        before = now;
        for k in 0..=8 {
            let alpha = k as f32 / 8.0;
            let at = engine
                .actors(alpha)
                .find(|a| a.entity == entity(0))
                .map(|a| a.transform.w_axis.truncate())
                .unwrap();
            engine.draw_hud(alpha);
            let drawn = engine.hud().fade();
            assert_eq!(
                drawn, step_fade,
                "step {n}, alpha {alpha}: fade drawn between steps"
            );
            if landed.is_none() && at.distance(Vec3::from(FROM)) > 0.01 {
                landed = Some((n, alpha, drawn));
            }
        }
    }
    assert!(refused.borrow().is_empty(), "{:?}", refused.borrow());
    let (n, alpha, drawn) = landed.expect("the hero was teleported");
    assert_eq!(
        drawn, 1.0,
        "first drawn away from where it stood at step {n}, alpha {alpha}"
    );
}
