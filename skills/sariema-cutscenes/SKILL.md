---
name: sariema-cutscenes
description: Use when writing or fixing a Sariema cutscene JSON, when sariema check or build fails with E_CUTSCENE_TELEPORT, E_CUTSCENE_COVER, E_CUTSCENE_START, E_CUTSCENE_ABSENT, E_CUTSCENE_SPAWN, E_CUTSCENE_RACE or E_CUTSCENE_ROLE, or when a cutscene_play is rejected with E_CUTSCENE_UNBOUND, E_CUTSCENE_UNKNOWN or E_CUTSCENE_BUSY.
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
- Skipping: `cutscene_skip` (or the `skip` button) lands the same end state
  as playing through, under a fade; do not undo the cutscene's effects in
  the game after `cutscene_ended { skipped: true }`.
