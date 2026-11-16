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
| a place on the floor to report who stands in it | `ZoneCylinder { zone, center, radius, height }` (centre halfway up, feet within the radius and the height; a size below 0 is `E_ZONE_SIZE`), then `Event::TriggerEnter`/`TriggerExit`; `Zone` is a sphere, which bulges at mid-height |
| walk somewhere | `MoveTo { entity, target, speed }`, then `Event::Arrived` |
| a hidden actor out of the way | `SetCollider { entity: actor, enabled: false }`: others walk through it and doors swing through it; it still walks with collision. Hiding (`SetVisible`) alone leaves its body in the way |
| one actor to turn slower than the rest | `SetTurnRate { entity, rate }` in degrees per second, at most `motion.turn_rate` (set that to the fastest actor's rate); `rate: 0.0` goes back to the setting |
| later | `Timer { timer, duration, repeat }`, then `Event::TimerFired` |
| a smooth value | `Tween { … ease }` or `Spring { … }`, then `Event::TweenDone` |
| a button's index | `ctx.button(id("action"))` in `Game::new`; never copy its place in `input.buttons` |
| a button press | `Event::Button { action, down }`, or `Event::Buffered { action, .. }` until you send `Consume { action }` |
| randomness | `ctx.rng()`, seeded by the engine; never another source |
| persistence | `Save { key, value }`; `SaveLoad {}` → `SaveLoaded`…, `SaveLoadDone` |
| where an actor stands, for a save | `Locate { entity }`, then `Event::Located { entity, position, yaw }` at the next tick (yaw in (-180, 180]; an entity despawned in the asking step gets `Rejected` `E_ENTITY_STALE`); save those, never a guess from zones or the last target |
| many objects | `Transforms` bulk region: register once, `push` each tick |
| a name as id | `id("meet")`, a `const fn` |
| another room | `RoomSwitch { room, transition, duration }`, then `Event::RoomSwitched`; list it under `game.rooms` with its `neighbours` so it is loaded before. `Continuous` for a room whose geometry goes on (walk in, the camera keeps following); `Dissolve` (the last frame fades out over the next room, 0.5 s when `duration` is 0) for another place or a jump in time; `Cut` under a cutscene's fade |
| a room loaded ahead | `RoomPreload { room }`, beyond the neighbours |
| part of a room on or off | `RoomGroup { group, on }` (a sub-room); `PieceState { piece, state }` (0: as placed) |
| a light or the look to change | `Tween { property: Property::Scene, var: id("<parameter>"), .. }` |
| a room node or light to move | `ctx.spawn(Kind::RoomNode, id("<node>"))` or `Kind::Light`, then move the entity |

Test the logic natively with `sariema_sdk::testing::TestHost` (queue
events, `Runner::run`, decode with `testing::commands`;
`TestHost::with_buttons(&["confirm", "action"])` for the input buttons),
and the whole game
with `sariema run --headless … --json`.

## Common mistakes

| Symptom | Cause | Fix |
|---|---|---|
| `rejected` with `E_TELEPORT_VISIBLE` | something would move a visible actor faster than `motion.max_visible_speed`: `set_transform`, `teleport`, a fast `tween` or walk (at its speed scale), a rigid `follow`, `set_parent`, or moving what carries it. Opcode 0: the step's measurement put the actor back and stopped what moved it; the first event names the actor, the next ones each carrier stopped (a fast move begun under a fade or cut that lifted, or while hidden; a spring; a carrier turning or growing, even one spawned or placed this step; moves that add up). A walk stopped this way also ends with `blocked` | walk or ease it slower; hide it a step before; or keep the fade or cut over the whole move |
| `rejected` with `E_TURN_RATE` | `SetTurnRate` faster than `motion.turn_rate` or below 0 | raise `motion.turn_rate` to the fastest actor's rate in `game.settings` |
| `rejected` with `E_ENTITY_STALE` | a handle kept after `despawn` or a `despawned` event | drop handles when they die |
| `E_GAME_TRAP` at step 0 with "time not implemented" | `std::time` | count steps (`ctx.step()`) |
| `E_GAME_BUDGET` | heavy work in one tick | spread it over steps; bulk regions for many placements |
| `state_hash` changed after a rebuild, same log | it covers the module's memory, which a rebuild moves | compare `world_hash` across builds, `state_hash` within one |
| a press acted on twice | reading `Buffered` without `Consume` | send `Consume` once handled |
| nothing in the module's log after a panic | no panic hook | `std::panic::set_hook(Box::new(\|i\| sariema_sdk::report_panic(i)))` under `#[cfg(target_arch = "wasm32")]` |
