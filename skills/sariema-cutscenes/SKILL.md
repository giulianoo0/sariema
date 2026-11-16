---
name: sariema-cutscenes
description: Use when writing, directing or fixing a Sariema cutscene JSON (traversals, insert shots, slow motion, clip markers, camera shake), when sariema check or build fails with E_CUTSCENE_TELEPORT, E_CUTSCENE_COVER, E_CUTSCENE_START, E_CUTSCENE_ABSENT, E_CUTSCENE_SPAWN, E_CUTSCENE_RACE, E_CUTSCENE_SPEED, E_CUTSCENE_ROLE or E_SETTINGS, or when a cutscene_play is rejected with E_CUTSCENE_UNBOUND, E_CUTSCENE_UNKNOWN or E_CUTSCENE_BUSY.
---

# Sariema cutscenes

## Overview

A cutscene file (`cutscenes/<name>.json`, `schemas/cutscene.schema.json`)
casts roles, each with a `start` place, and lists steps. The engine never
lets an actor jump on screen: before the steps it walks or eases every role
to its start, and a `teleport` needs a declared cover the checker can prove.
Full reference: `docs/cutscenes.md`.

## Worked example

`examples/hello-game/cutscenes/meet.json`:

```json
{
  "name": "meet",
  "roles": {
    "hero": { "start": { "position": [0, 0, -5.5], "yaw": 0 } },
    "friend": { "start": { "position": [0, 0, -8], "yaw": 180 } }
  },
  "steps": [
    { "letterbox": { "to": 1, "duration": 0.4 } },
    { "shot": { "frame": ["hero", "friend"], "pitch": 25, "blend": 0.8 } },
    { "walk": { "role": "hero", "to": [0, 0, -6.8], "speed": 1.2 } },
    { "shot": { "follow": "friend", "distance": 3, "pitch": 12, "yaw": 180, "blend": 0.6 } },
    { "line": { "speaker": "friend", "text": "Hello! You found the meadow.", "advance": "auto", "duration": 2.5 } },
    { "set_flag": "met" },
    { "letterbox": { "to": 0, "duration": 0.4 } }
  ]
}
```

From the game: `ctx.bind(id("hero"), hero); ctx.bind(id("friend"), friend); ctx.play_cutscene(id("meet"));`

Check, build and watch it play (`sariema` is `cargo run -q -p sariema-cli --`):

```sh
sariema check examples/hello-game --json
sariema build examples/hello-game -o hello.srmg --json
sariema run --headless --game hello.srmg --input examples/hello-game/input/walk.json --steps 600 --json
```

In the run, look for `cutscene_started`, `arrived` (the walks), `line_done`,
`flag_changed`, `control` and `cutscene_ended`.

## Directing a physical action

The engine's directing rules (`docs/cutscenes.md`, "Directing"), as steps:

| Rule | Write |
|---|---|
| 1 Whole action, cause to consequence | one `clip` per phase (reach, open, climb, land, get up), chained by `wait_for: clip_finished`; never start mid-way, never cut away a phase |
| 2 Anticipation, impact, recovery | clips for the stumble, the fall and getting up; `camera_shake` on the impact marker |
| 3 Insert shot on the beat | `wait_for: { clip_marker: { role, clip, marker } }`, then a `shot` with `"blend": 0` and `"bone"`/`"socket"`; cut back to the wide shot the same way |
| 4 Set pieces keyed to clip time | the leaf (window, door) is a role with its own clip, started in the same `parallel` as the actor's clip; a real hinge binding is limit L4 |
| 5 Traversal in clip segments | not expressible yet (L8); slow the beat with `time_scale` |
| 6 Contact authored, not penetration | not expressible yet (L8): the pose is in the clip, the root placement and the contact check come with L8 |
| 7 Clip time follows the step clock | automatic: clips are sought to `clip_time` (and drawn at `clip_time_at`, interpolated), so they slow with `time_scale` without still frames and `clip_finished` still comes |
| 8 Cut on the action, position continuous | cuts change the camera only, never `teleport` to hide a move; `wait` counts world time (0.6 s on screen at 0.4× is `wait: 0.24`) |
| 9 No HUD while covered | the game hides its HUD on `cutscene_started` and while it fades |

Entering through a window, the foot catching the sill
(`crates/sariema-runtime/tests/fixtures/cutscenes/window_entry.json`, passes
`sariema check`):

```json
{
  "name": "window_entry",
  "roles": {
    "hero": { "start": { "position": [0, 0, 2], "yaw": 0 } },
    "window": { "start": { "position": [0, 1, 0], "yaw": 0 } }
  },
  "steps": [
    { "letterbox": { "to": 1, "duration": 0.4 } },
    { "shot": { "frame": ["hero", "window"], "pitch": 12, "yaw": 35, "blend": 0.6 } },
    { "walk": { "role": "hero", "to": [0, 0, 0.6] } },
    { "parallel": [
      [ { "clip": { "role": "hero", "clip": "reach_open" } } ],
      [ { "clip": { "role": "window", "clip": "swing_open", "mode": "hold" } },
        { "sound": { "sound": "window_creak", "at": "window" } } ]
    ] },
    { "wait_for": { "clip_finished": "hero" } },
    { "clip": { "role": "hero", "clip": "climb_trip", "blend": 0 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "foot_catches" } } },
    { "shot": { "follow": "hero", "bone": "foot_l", "distance": 0.9, "pitch": 5, "yaw": 90, "height": 0, "fov": 35, "blend": 0 } },
    { "time_scale": { "scale": 0.4 } },
    { "wait": 0.24 },
    { "shot": { "frame": ["hero", "window"], "pitch": 12, "yaw": 35, "blend": 0 } },
    { "time_scale": { "scale": 1, "blend": 0.15 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "lands" } } },
    { "camera_shake": { "amplitude": 0.03, "duration": 0.25 } },
    { "wait_for": { "clip_finished": "hero" } },
    { "clip": { "role": "hero", "clip": "get_up" } },
    { "wait_for": { "clip_finished": "hero" } },
    { "letterbox": { "to": 0, "duration": 0.4 } }
  ]
}
```

## Fixing a failed check

The message names the file, cutscene and step: `cutscene meet, steps[4].teleport: …`.

| Code | Fix |
|---|---|
| `E_CUTSCENE_TELEPORT` | `walk` the role; or `hide` it, `wait`, `teleport`, `show`; or add `"cover"` |
| `E_CUTSCENE_COVER` (fade) | put `{ "fade": { "to": 1, "duration": … } }` before the teleport on every path, and fade back after |
| `E_CUTSCENE_COVER` (cut) | put a `shot` with `"blend": 0` right before the teleport, with no step that takes time between them |
| `E_CUTSCENE_START` | give the role `"start": { "position": […], "yaw": … }`, or `spawn` it before its first use |
| `E_CUTSCENE_SPAWN` | spawn only roles without `start`, or after their `despawn` |
| `E_CUTSCENE_ABSENT` | do not use a role after `despawn` |
| `E_CUTSCENE_RACE` | move each role, and set each flag, in one `parallel` branch only |
| `E_CUTSCENE_SPEED` | slow the `walk` (or `cutscene.start_walk_speed`) to `motion.max_visible_speed` or less; to cross faster, hide it or cut |
| `E_SETTINGS` | in `game.settings`, keep `min_time_scale` and `max_time_scale` positive, the minimum no larger |
| `E_CUTSCENE_ROLE` | declare the role under `roles` |
| `E_SCHEMA` | the message names the field; `shot` needs `blend`, an `auto` line needs `duration` |

At run time a refused `cutscene_play` arrives as a `rejected` event:
`E_CUTSCENE_UNBOUND` (bind every role with a start to a live entity, and
every spawned role to `ctx.reserve()`), `E_CUTSCENE_UNKNOWN` (no file with
that `name` was built), `E_CUTSCENE_BUSY` (wait for `cutscene_ended`).

## Common mistakes

- A `start` far from where the game leaves the actor: legal, but the actor
  walks there in view first. Put the start where the trigger fires.
- Expecting `wait_for` to see a signal sent before the wait began: only
  later ones count.
- Hiding a move with a cut: a cut is not a cover for a walk you skipped;
  rule 1 wants the walk on screen.
- Waiting real seconds in slow motion: `wait` counts world time.
- Skipping: `cutscene_skip` (or the `skip` button) lands the same end state
  as playing through, under a fade; do not undo the cutscene's effects in
  the game after `cutscene_ended { skipped: true }`.
