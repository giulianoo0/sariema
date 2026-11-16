---
name: sariema
description: Use when building, changing or debugging a game for the Sariema engine — starting a project, writing its WebAssembly logic, adding cutscenes, rooms, actors, physics, audio or UI — or when a `sariema` command fails with an E_ code and the next step is unclear.
---

# Sariema

## Overview

A Sariema game is a project directory: rooms, a logic crate in Rust built to
WebAssembly with `sariema-sdk`, and cutscenes in JSON. The engine runs the
logic once per 1/60 s step; the logic sends commands and reads events, and
the engine does the moving, animating and drawing. Every command answers
`--json` with `{"ok":true,...}` or `{"ok":false,"error":{"code":"E_...","message":"..."}}`.

From the engine's repository root, `sariema` is `cargo run -q -p sariema-cli --`.

## The loop

```sh
sariema new games/meadow --toolchain 1.100.0 --json      # a project that builds as it is
sariema check games/meadow --json                        # cutscene rules
sariema build games/meadow -o meadow.srmg --json         # compile the logic, write the game pack
sariema run --headless --game meadow.srmg --input games/meadow/input/walk.json --steps 480 --json
(cd games/meadow/game && cargo test)                     # the logic's own tests, natively
```

Change, then run the loop again. `run --headless` prints every event the
game received (`step`, `event`, fields), the host messages (`save`, `ready`),
the game's log, the saves, `state_hash`, `world_hash` and timing. Same pack,
same input log, same seed: same hashes. Pin them in a test.

`sariema api --json` describes every command, event, enum, setting and error
code. Read it instead of guessing a name or a field.

## Project layout

```
sariema.json          name, assets_root, rooms, and game: { logic, cutscenes, settings, toolchain }
rooms/start.json      a room description (sariema build <dir> --room start -o start.srma)
game/                 the logic crate: cdylib, its own [workspace], depends on sariema-sdk
cutscenes/*.json      one cutscene per file
input/*.json          recorded input logs for run --headless
```

`examples/hello-game` is a complete game: a hero moved by the input axis, a
trigger that plays a two-shot cutscene, a save flag.

## Where to go next

- Writing the logic (commands, events, entities, input, saves, budgets):
  `sariema-game-scripting`, and `docs/game-api.md`.
- Writing a cutscene that passes the checks: `sariema-cutscenes`, and
  `docs/cutscenes.md`.
- Characters and their animation: `sariema-actors`.
- Collision, triggers and movement: `sariema-physics`.
- Sound and music: `sariema-audio`.
- Menus, HUD and the touch joystick: `sariema-ui`.

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
| `E_USAGE` | the command line is wrong | `sariema <command> --help` |
