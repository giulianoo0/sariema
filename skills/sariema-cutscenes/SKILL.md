---
name: sariema-cutscenes
description: Use when writing, directing or fixing a Sariema cutscene JSON (traversals bound to a clip with path spans, clips chained in a row, insert shots, slow motion, clip markers, camera shake, a door or window leaf turned with a clip), when sariema check or build fails with E_CUTSCENE_TELEPORT, E_CUTSCENE_COVER, E_CUTSCENE_START, E_CUTSCENE_ABSENT, E_CUTSCENE_SPAWN, E_CUTSCENE_RACE, E_CUTSCENE_SPEED, E_CUTSCENE_PATH_CUT, E_CUTSCENE_ROLE or E_SETTINGS, or when a cutscene_play is rejected with E_CUTSCENE_UNBOUND, E_CUTSCENE_UNKNOWN or E_CUTSCENE_BUSY.
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
| 4 Set pieces keyed to clip time | a `hinge` step in a `parallel` with the actor's `clip`: `{ "hinge": { "node": "<room>.<piece>_<hinge>", "role", "clip", "span": [from_s, to_s], "to": degrees_from_closed, "ease" } }`; the creak on a `clip_marker` of the same clip |
| 5 Traversal in clip segments | a `path` step: `spans` of the clip (`at: [t0, t1]`) are the only times the root moves, to each span's `to`; a span's `rate` (0.4) slows the body alone on that span; `from` starts part way in, `hold` freezes the first frame under a cut or fade |
| 6 Contact authored, not penetration | the span's `to` is the point that puts the foot on the edge; the role is posed collider off over its spans and gets the collider back where it lands, so end the last span on free floor |
| 7 Clip time follows the step clock | automatic: clips are sought to `clip_time` (and drawn at `clip_time_at`, interpolated), so they slow with `time_scale` without still frames and `clip_finished` still comes; a clip shows frame 0 until the step after it begins, so start it one step before it must move; chain the clips after a traversal in the `path`'s `then` (`after` cuts the one before short), not with `wait_for: clip_finished` + `clip`, which shows a held last frame |
| 8 Cut on the action, position continuous | cuts change the camera only, never `teleport` to hide a move; `wait` counts world time (0.6 s on screen at 0.4× is `wait: 0.24`) |
| 9 No HUD while covered | the game hides its HUD on `cutscene_started` and while it fades |

Entering through a window, the foot catching the sill
(`crates/sariema-runtime/tests/fixtures/cutscenes/window_entry.json`, passes
`sariema check`):

```json
{
  "name": "window_entry",
  "roles": {
    "hero": { "start": { "position": [0, 0, 2], "yaw": 0 } }
  },
  "steps": [
    { "letterbox": { "to": 1, "duration": 0.4 } },
    { "shot": { "follow": "hero", "distance": 3.5, "pitch": 12, "yaw": 35, "height": 1.0, "fov": 40, "blend": 0.6 } },
    { "walk": { "role": "hero", "to": [0, 0, 0.6] } },
    { "parallel": [
      [ { "clip": { "role": "hero", "clip": "reach_open" } } ],
      [ { "hinge": { "node": "house.window_leaf", "role": "hero", "clip": "reach_open",
                     "span": [0.35, 0.9], "to": 80, "ease": "out_cubic" } } ],
      [ { "wait_for": { "clip_marker": { "role": "hero", "clip": "reach_open", "marker": "push" } } },
        { "sound": { "sound": "window_creak", "at": "hero" } } ]
    ] },
    { "wait_for": { "clip_finished": "hero" } },
    { "clip": { "role": "hero", "clip": "climb_trip", "blend": 0 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "foot_catches" } } },
    { "shot": { "follow": "hero", "bone": "foot_l", "distance": 0.9, "pitch": 5, "yaw": 90, "height": 0, "fov": 35, "blend": 0 } },
    { "time_scale": { "scale": 0.4 } },
    { "wait": 0.24 },
    { "shot": { "follow": "hero", "distance": 3.5, "pitch": 12, "yaw": 35, "height": 1.0, "fov": 40, "blend": 0 } },
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

A traversal in clip spans, the foot catching the sill in slow motion, the
landing and getting up chained without a still frame
(`crates/sariema-runtime/tests/fixtures/cutscenes/sill_trip.json`):

```json
{ "path": { "role": "hero", "clip": "climb_trip", "from": 0.1, "spans": [
    { "at": [0.3, 0.6], "to": [0, 0.9, 0.15], "ease": "out_quad" },
    { "at": [0.6, 0.75], "to": [0, 0.9, -0.1] },
    { "at": [0.75, 0.9], "rate": 0.4 },
    { "at": [0.9, 1.2], "to": [0, 0, -0.8], "ease": "in_quad", "arc": 0.1 } ],
  "then": [ { "clip": "land_belly" }, { "clip": "get_up", "after": 0.6 } ] } }
```

Put it in a `parallel` with a branch of `wait_for: clip_marker` and
`shot`s to cut to the close on the catch. Copied into a project's
`cutscenes/`, `sariema check <project> --json` gives `"ok":true`; a span
too fast to be seen (say `"at": [0.6, 0.62]` to 2 m away) gives
`E_CUTSCENE_SPEED` naming `steps[2].parallel[0][0].path`.

From a game, the same is `clip_path` (spans written with
`ClipSpan::put`) and `clip_next`; refused with `E_CLIP_SPANS` (out of
order, past the clip's end) or `E_TELEPORT_VISIBLE` (too fast).

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
| `E_CUTSCENE_SPEED` | slow the `walk` (or `cutscene.start_walk_speed`), or widen the `path` span (or lower its `rate`), to `motion.max_visible_speed` or less; to cross faster, hide it or cut |
| `E_CUTSCENE_PATH_CUT` | give the path's first `then` clip an `after` at or past the last span's end, or 0 (the clip's end) |
| `E_SETTINGS` | in `game.settings`, keep `min_time_scale` and `max_time_scale` positive, the minimum no larger |
| `E_CUTSCENE_ROLE` | declare the role under `roles` |
| `E_CUTSCENE_HINGE` | the `hinge` step's `node` is `<room>.<piece>_<hinge>` of a loose piece of some room; fix the name or make the piece loose |
| `E_SCHEMA` | the message names the field; `shot` needs `blend`, an `auto` line needs `duration`, a `hinge` span starts at 0 s or later and ends after it starts |

At run time a refused `cutscene_play` arrives as a `rejected` event:
`E_CUTSCENE_UNBOUND` (bind every role with a start to a live entity, and
every spawned role to `ctx.reserve()`), `E_CUTSCENE_UNKNOWN` (no file with
that `name` was built), `E_CUTSCENE_BUSY` (wait for `cutscene_ended`).
A `walk` goes at its own speed in world units, whatever the role's speed
scale (`set_speed_scale` scales the player's axis and `move_intent` only),
so it takes its length over its speed. With a collision module a start a
few centimetres off in height only (the floor holds the actor) adds no
time, up to `start_height` (0.25 m); one off along the ground by up to
`start_blend_max` adds `start_blend` (0.25 s, 15 steps); one a storey up or
down is eased or walked to in height too. A `walk` the game takes away
(`stop`, a `teleport` that lands, `move_intent`) ends there and the
cutscene goes on.

## Common mistakes

- A `start` far from where the game leaves the actor: legal, but the actor
  walks there in view first. Put the start where the trigger fires.
- Expecting `wait_for` to see a signal sent before the wait began: only
  later ones count.
- Hiding a move with a cut: a cut is not a cover for a walk you skipped;
  rule 1 wants the walk on screen.
- Waiting real seconds in slow motion: `wait` counts world time.
- A `hinge` step whose role never plays its clip ends after
  `cutscene.hinge_clip_wait` (2 s) with `rejected` `E_CUTSCENE_HINGE_WAIT`,
  the leaf unturned: put the `clip` in the same `parallel`. One on a node
  the active room lacks ends at once with `E_SCENE_UNKNOWN`; `sariema
  check` already refuses a node no room has (`E_CUTSCENE_HINGE`). List the
  nodes with `sariema inspect <room>.srmc` (`hinges:`).
- Skipping: `cutscene_skip` (or the `skip` button) lands the same end state
  as playing through, under a fade; do not undo the cutscene's effects in
  the game after `cutscene_ended { skipped: true }`.
