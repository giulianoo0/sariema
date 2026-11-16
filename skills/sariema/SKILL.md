---
name: sariema
description: Use when building, changing or debugging a game for the Sariema engine — starting a project, writing its WebAssembly logic, adding cutscenes, rooms, actors, physics, audio or UI — or when a `sariema` command fails with an E_ code and the next step is unclear.
---

# Sariema

## Overview

A Sariema game is a project directory: a room with its collision, actors,
a sound bank, a HUD, a logic crate in Rust built to WebAssembly with
`sariema-sdk`, and cutscenes in JSON. The engine runs the logic once per
1/60 s step; the logic sends commands and reads events, and the engine does
the moving (with collision), animating, mixing and drawing. Every command answers
`--json` with `{"ok":true,...}` or `{"ok":false,"error":{"code":"E_...","message":"..."}}`.

From the engine's repository root, `sariema` is `cargo run -q -p sariema-cli --`.

## The loop

```sh
sariema new games/meadow --toolchain 1.100.0 --json      # room + hero + cutscene + sound + HUD
sariema check games/meadow --json                        # spots, routes, cutscene rules
sariema build games/meadow -o meadow.srmg --json         # compile the logic, pack it with its assets
sariema run --headless --game meadow.srmg --input games/meadow/input/walk.json --steps 480 --json
sariema run --headless --game meadow.srmg --input games/meadow/input/walk.json --steps 480 \
    --screenshot strip.png --filmstrip 8 --width 640 --height 360 --json   # look at it
sariema run --game meadow.srmg                           # play it in a window
sariema web games/meadow -o web/meadow --json            # a folder a web page plays (skill sariema-web)
(cd games/meadow/game && cargo test)                     # the logic's own tests, natively
```

With `--toolchain`, run the logic's tests with that toolchain's `cargo`,
`rustc` and `rustdoc` (`RUSTC=… RUSTDOC=… …/cargo test`); a `rustdoc` from
another toolchain fails the doc-test step.

Change, then run the loop again. `run --headless` runs the whole engine
(collision, animation, sound, HUD) and prints every event the game received
(`step`, `event`, fields), the host messages (`save`, `ready`), the game's
log, the saves, `engine_hash` (the same on every platform, browsers included),
`state_hash`, `world_hash`, `physics_hash`, `audio_hash` (repeatable within
one build) and timing. Same pack, same input log, same seed: same hashes.
Pin `engine_hash` in a test. `--screenshot` draws the frames a window would
show without changing any hash; look at every PNG.

The window takes the keys the input settings bind and the mouse as the
HUD's pointer (joystick, buttons), and plays the sound on the default
device. Its report gives `frames`, `steps`, `ms_per_frame` and `sound`;
`sound.device: false` with `E_AUDIO_DEVICE` means it ran silent.
`sariema bench meadow.srmg --input … --json` times a step and its frame,
the HUD pass apart.

`sariema api --json` describes every command, event, enum, setting and error
code. Read it instead of guessing a name or a field.

## Project layout

```
sariema.json          name, assets_root, rooms, actors, audio, and
                      game: { logic, cutscenes, room, hud, settings, toolchain }
rooms/start.json      a room and its collision (sariema build <dir> --room start -o start.srma)
actors/hero.json      an actor: its model, markers, sockets
audio.json, sounds/   the sound description and its files
hud.json, fonts/      the HUD: objective, joystick, action button
game/                 the logic crate: cdylib, its own [workspace], depends on sariema-sdk
cutscenes/*.json      one cutscene per file
input/*.json          recorded input logs for run --headless
```

The game pack carries the room named by `game.room`, its collision, every
actor, the sound bank and the HUD, so one `.srmg` runs everywhere.

`examples/climb` is the complete game: the hero walks with collision, a
wave whose marker rings a chime, a cutscene that walks to a mark, climbs
through a window with a contact, cuts close and back, says a line over
ducked music past the window and dips to black to a wide shot of the hero
where the climb left it; an objective animates in on the first move and
checks itself off. `examples/hello-game` is the smallest game, without
assets.

## Where to go next

- Writing the logic (commands, events, entities, input, saves, budgets):
  `sariema-game-scripting`, and `docs/game-api.md`.
- Writing a cutscene that passes the checks: `sariema-cutscenes`, and
  `docs/cutscenes.md`.
- Characters and their animation: `sariema-actors`.
- Collision, triggers and movement: `sariema-physics`.
- Sound and music: `sariema-audio`.
- Menus, HUD and the touch joystick: `sariema-ui`.
- The room's light, fog and look changed while playing (`scene_tween`, a
  cutscene's `scene` step): `docs/lighting.md`, and `examples/dusk`.
- Walls cut down between the camera and the hero (`cutaways` in a room,
  `cutaway.<name>.height`): `docs/lighting.md`, and `examples/cutaway`.

## Common failures

| Code | Means | Do |
|---|---|---|
| `E_GAME_BUILD` | cargo could not build `game/` for `wasm32-unknown-unknown` | read the message's compiler output; without `game.toolchain`, the `cargo` on PATH needs that target |
| `E_GAME_MODULE` | the module lacks `sariema_tick`, or imports something the engine does not give | end `lib.rs` with `game!(YourGame);`; do not use files, clocks or threads |
| `E_GAME_TRAP` | the logic panicked or trapped; the run stops at that step | the message ends with the panic text the game logged |
| `E_GAME_BUDGET` | one tick ran out of fuel (`vm.fuel_per_step`) | do less per step, or spread work over steps |
| `E_CUTSCENE_*` | a cutscene breaks a rule | see `sariema-cutscenes` |
| `E_SCHEMA` | a JSON file does not match its schema | the message names the file and the field |
| `E_PROJECT_EXISTS` | `new` was given a non-empty directory | pick a new directory |
| `E_USAGE` | the command line is wrong | `sariema <command> --help`; a headless game run needs `--steps` |
| `E_AUDIO_DEVICE` | the window found no sound device | it plays on silent; nothing to fix in the game |
| `rejected` event, `E_TELEPORT_BLOCKED` | a teleport target is inside something | pick free floor; `sariema physics-map` shows it |
| `blocked` after a cutscene `walk` | no path to the mark from where the role stood | move the mark; `sariema check` proves spots and routes |
