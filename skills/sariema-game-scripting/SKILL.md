---
name: sariema-game-scripting
description: Use when writing or fixing the Rust logic of a Sariema game with sariema-sdk — spawning entities, reacting to events, moving the hero, timers, saves, input presses — or when a run reports E_GAME_TRAP, E_GAME_BUDGET, E_GAME_MEMORY, a rejected event such as E_ENTITY_STALE or E_TELEPORT_VISIBLE, or hashes that differ between runs.
---

# Sariema game scripting

## Overview

The logic is a `Game`: `new` runs on the first tick, `tick` on every tick
after the SDK read the step's events. It describes what should happen with
commands; the engine executes them. Never loop over time inside a tick:
start a `move_to`, a `tween` or a `timer` and wait for its event.

## Worked example

`examples/hello-game/game/src/lib.rs`, in short:

```rust
use sariema_sdk::{Command, Ctx, Entity, Event, Events, Game, Kind, game, id};

const NEAR_FRIEND: u32 = 1;

pub struct Hello { hero: Entity, met: bool }

impl Game for Hello {
    fn new(ctx: &mut Ctx<'_>) -> Hello {
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform { entity: hero, position: [0.0; 3], yaw: 0.0 });
        ctx.send(Command::SetHero { entity: hero, speed: 2.0 });
        ctx.send(Command::Zone { zone: NEAR_FRIEND, center: [0.0, 0.0, -8.0], radius: 3.0 });
        ctx.send(Command::SaveLoad {});
        Hello { hero, met: false }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::SaveLoaded { key: "met", value: [1] } => self.met = true,
                Event::TriggerEnter { trigger: NEAR_FRIEND, entity } if entity == self.hero && !self.met => {
                    ctx.bind(id("hero"), self.hero);
                    ctx.play_cutscene(id("meet"));
                }
                Event::CutsceneEnded { .. } => ctx.send(Command::Save { key: "met", value: &[1] }),
                _ => {}
            }
        }
    }
}

game!(Hello);
```

## Quick reference

| Want | Send / read |
|---|---|
| a new entity | `ctx.spawn(kind, asset)`; place it in the same tick with `set_transform` |
| the player to walk it | `SetHero { entity, speed }`; the move axis drives it natively |
| walk somewhere | `MoveTo { entity, target, speed }`, then `Event::Arrived` |
| later | `Timer { timer, duration, repeat }`, then `Event::TimerFired` |
| a smooth value | `Tween { … ease }` or `Spring { … }`, then `Event::TweenDone` |
| a button press | `Event::Buffered { action, .. }` until you send `Consume { action }` |
| randomness | `ctx.rng()`, seeded by the engine; never another source |
| persistence | `Save { key, value }`; `SaveLoad {}` → `SaveLoaded`…, `SaveLoadDone` |
| many objects | `Transforms` bulk region: register once, `push` each tick |
| a name as id | `id("meet")`, a `const fn` |

Test the logic natively with `sariema_sdk::testing::TestHost` (queue
events, `Runner::run`, decode with `testing::commands`), and the whole game
with `sariema run --headless … --json`.

## Common mistakes

| Symptom | Cause | Fix |
|---|---|---|
| `rejected` with `E_TELEPORT_VISIBLE` | `set_transform`/`teleport` of a visible actor | walk it, hide it first, or fade to 1 first |
| `rejected` with `E_ENTITY_STALE` | a handle kept after `despawn` or a `despawned` event | drop handles when they die |
| `E_GAME_TRAP` at step 0 with "time not implemented" | `std::time` | count steps (`ctx.step()`) |
| `E_GAME_BUDGET` | heavy work in one tick | spread it over steps; bulk regions for many placements |
| `state_hash` changed after a rebuild, same log | it covers the module's memory, which a rebuild moves | compare `world_hash` across builds, `state_hash` within one |
| a press acted on twice | reading `Buffered` without `Consume` | send `Consume` once handled |
| nothing in the module's log after a panic | no panic hook | `std::panic::set_hook(Box::new(\|i\| sariema_sdk::report_panic(i)))` under `#[cfg(target_arch = "wasm32")]` |
