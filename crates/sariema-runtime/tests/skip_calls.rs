// SPDX-License-Identifier: Apache-2.0

//! The calls a skip drains reach the game flagged as skipped, in the same
//! step as `cutscene_ended { skipped }`, while the screen is still dark; the
//! fade back in starts once the game has had that step.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::script::{Command, Event, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{native, send};

const SCENE: &str = r#"{ "name": "s", "roles": {}, "steps": [
    { "wait": 1 }, { "call": "a" }, { "wait": 1 }, { "call": "b" } ] }"#;

const STEPS: u64 = 240;

struct Run {
    /// Every event the game sees, as `(step, debug)`.
    events: Vec<(u64, String)>,
    /// The fade after each step.
    fades: Vec<f32>,
}

/// Plays the scene at step 1 and, when `skip_at` is given, skips it then;
/// with `keep_dark`, the game answers `cutscene_ended` by fading to black
/// itself.
fn run(skip_at: Option<u64>, keep_dark: bool) -> Run {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let logic = native(move |step, events, out| {
        log.borrow_mut()
            .extend(events.iter().map(|e| (step, format!("{e:?}"))));
        if keep_dark
            && events
                .iter()
                .any(|e| matches!(e, Event::CutsceneEnded { .. }))
        {
            send(
                out,
                Command::Fade {
                    to: 1.0,
                    duration: 0.5,
                    color: [0.0; 3],
                },
            );
        }
        if step == 1 {
            send(
                out,
                Command::CutscenePlay {
                    cutscene: name_id("s"),
                },
            );
        }
        if Some(step) == skip_at {
            send(out, Command::CutsceneSkip {});
        }
    });
    let settings = Settings::default();
    let cutscenes = vec![compile_json(SCENE).unwrap().0];
    let mut sim = Simulation::new(settings.clone(), cutscenes, logic, 1);
    let actions = Actions::for_settings(&settings.input);
    let mut fades = Vec::with_capacity(STEPS as usize);
    for _ in 0..STEPS {
        sim.step(&actions);
        fades.push(sim.state().screen.fade.value());
    }
    Run {
        events: seen.take(),
        fades,
    }
}

fn call(name: &str) -> String {
    format!(
        "CutsceneCall {{ cutscene: {}, call: {}",
        name_id("s"),
        name_id(name)
    )
}

/// The step the game sees the first event starting with `head`, and the
/// event.
fn find<'a>(events: &'a [(u64, String)], head: &str) -> (u64, &'a str) {
    events
        .iter()
        .find(|(_, e)| e.starts_with(head))
        .map(|(s, e)| (*s, e.as_str()))
        .unwrap_or_else(|| panic!("no {head}: {events:?}"))
}

#[test]
fn the_calls_a_skip_drains_are_flagged_and_end_with_it_under_the_fade() {
    let r = run(Some(80), false);
    let (a, played) = find(&r.events, &call("a"));
    assert!(a < 80 && played.ends_with("skipped: false }"), "{played}");
    let (b, drained) = find(&r.events, &call("b"));
    assert!(drained.ends_with("skipped: true }"), "{drained}");
    let (ended, end) = find(&r.events, "CutsceneEnded");
    assert!(end.ends_with("skipped: true }"), "{end}");
    assert_eq!(b, ended, "the drained call and the end share a step");
    let order: Vec<_> = r.events.iter().filter(|(s, _)| *s == b).collect();
    let at = |head: &str| order.iter().position(|(_, e)| e.starts_with(head));
    assert!(at(&call("b")) < at("CutsceneEnded"), "{order:?}");
    let n = b as usize;
    assert_eq!(r.fades[n - 1], 1.0, "dark while the game reads them");
    assert!(r.fades[n] < 1.0, "the fade back in starts after that step");
    assert_eq!(r.fades[STEPS as usize - 1], 0.0);
}

#[test]
fn a_scene_played_through_is_unchanged() {
    let r = run(None, false);
    let (a, first) = find(&r.events, &call("a"));
    let (b, last) = find(&r.events, &call("b"));
    let (ended, end) = find(&r.events, "CutsceneEnded");
    assert!(first.ends_with("skipped: false }"), "{first}");
    assert!(last.ends_with("skipped: false }"), "{last}");
    assert!(end.ends_with("skipped: false }"), "{end}");
    assert_eq!((a, b, ended), (62, 122, 122));
    assert!(r.fades.iter().all(|f| *f == 0.0));
}

#[test]
fn a_fade_the_game_sends_with_the_end_of_a_skip_is_kept() {
    let r = run(Some(80), true);
    let (ended, _) = find(&r.events, "CutsceneEnded");
    assert!(r.fades[ended as usize..].iter().all(|f| *f == 1.0));
}
