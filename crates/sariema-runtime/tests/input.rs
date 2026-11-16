// SPDX-License-Identifier: Apache-2.0

use glam::Vec2;
use sariema_format::script::Event;
use sariema_runtime::input::{
    Actions, Input, InputLog, InputSettings, Key, PadButton, RawInput, Recorder, Replay, Stick,
};
use sariema_runtime::rng::Pcg32;

fn mapped(input: &Input, raw: &RawInput) -> Actions {
    let mut a = Actions::for_settings(input.settings());
    input.map(raw, &mut a);
    a
}

#[test]
fn keys_sticks_and_touch_map_to_the_move_axis() {
    let input = Input::new(InputSettings::default());
    let mut raw = RawInput::default();
    raw.set_key(Key::W, true);
    assert_eq!(mapped(&input, &raw).axes[0], Vec2::new(0.0, 1.0));
    raw.set_key(Key::D, true);
    let diagonal = mapped(&input, &raw).axes[0];
    assert!((diagonal.length() - 1.0).abs() < 1e-6 && diagonal.x > 0.0);

    raw.sticks[Stick::Left as usize] = Vec2::new(0.1, 0.0);
    assert_eq!(
        mapped(&input, &raw).axes[0],
        diagonal,
        "a stick inside the dead zone leaves the keys"
    );
    raw.sticks[Stick::Left as usize] = Vec2::new(-0.6, 0.0);
    assert!(
        (mapped(&input, &raw).axes[0].x - -0.5).abs() < 1e-6,
        "rescaled past the dead zone"
    );
    raw.touch = Some(Vec2::new(0.0, -1.0));
    assert_eq!(
        mapped(&input, &raw).axes[0],
        Vec2::new(0.0, -1.0),
        "touch wins"
    );

    raw.set_pad(PadButton::South, true);
    let confirm = input.settings().button("confirm").unwrap() as usize;
    assert!(mapped(&input, &raw).buttons[confirm]);
}

fn run(input: &mut Input, actions: &Actions) -> Vec<String> {
    let mut out = Vec::new();
    input.step(actions, |e| {
        out.push(match e {
            Event::Axis { x, y, .. } => format!("axis {x} {y}"),
            Event::Button { action, down } => format!("button {action} {down}"),
            Event::Buffered { action, age } => format!("buffered {action} {age}"),
            other => format!("{other:?}"),
        })
    });
    out
}

#[test]
fn a_press_stays_buffered_for_the_window_unless_consumed() {
    let settings = InputSettings::default();
    let confirm = settings.button("confirm").unwrap();
    let mut input = Input::new(settings.clone());
    let idle = Actions::for_settings(&settings);
    let mut pressed = idle.clone();
    pressed.buttons[confirm as usize] = true;

    assert_eq!(run(&mut input, &pressed), ["button 0 true", "buffered 0 0"]);
    assert!(input.pressed(confirm));
    assert_eq!(run(&mut input, &idle), ["button 0 false", "buffered 0 1"]);
    for age in 2..6 {
        assert_eq!(run(&mut input, &idle), [format!("buffered 0 {age}")]);
    }
    assert!(run(&mut input, &idle).is_empty(), "the window is 6 steps");

    run(&mut input, &pressed);
    assert!(input.consume(confirm));
    assert!(!input.consume(confirm));
    assert_eq!(run(&mut input, &idle), ["button 0 false"]);

    run(&mut input, &idle);
    run(&mut input, &pressed);
    input.clear_buffer();
    assert!(!input.buffered(confirm));
}

#[test]
fn a_recorded_log_replays_the_same_actions_through_json() {
    let settings = InputSettings::default();
    let mut rng = Pcg32::new(3, 0);
    let mut recorder = Recorder::new(&settings);
    let mut steps = Vec::new();
    let mut a = Actions::for_settings(&settings);
    for step in 0..600u64 {
        if rng.next_f32() < 0.05 {
            a.axes[0] = Vec2::new(rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0);
        }
        if rng.next_f32() < 0.03 {
            let b = (rng.next_u32() % 3) as usize;
            a.buttons[b] = !a.buttons[b];
        }
        recorder.record(step, &a);
        steps.push(a.clone());
    }
    let json = serde_json::to_string(&recorder.finish()).unwrap();
    let log: InputLog = serde_json::from_str(&json).unwrap();
    let mut replay = Replay::new(&log, &settings).unwrap();
    for (step, expected) in steps.iter().enumerate() {
        assert_eq!(replay.actions(step as u64), expected, "step {step}");
    }
}

#[test]
fn a_log_naming_an_unknown_action_is_refused() {
    let log: InputLog =
        serde_json::from_str(r#"{"changes":[{"step":0,"down":["jump"]}]}"#).unwrap();
    let error = Replay::new(&log, &InputSettings::default()).err().unwrap();
    assert_eq!(error.code(), "E_INPUT_LOG");
    assert!(error.to_string().contains("jump"));
    let log: InputLog = serde_json::from_str(
        r#"{"changes":[{"step":5,"down":["confirm"]},{"step":5,"up":["confirm"]}]}"#,
    )
    .unwrap();
    assert!(Replay::new(&log, &InputSettings::default()).is_err());
}
