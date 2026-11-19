---
name: sariema-game-scripting
description: Use when writing or fixing the Rust logic of a Sariema game with sariema-sdk — spawning entities, reacting to events, moving the hero, telling why a walk was blocked, casting a ray against the room, timers, saves, input presses, opening doors, changing a room's light, sun, hemisphere, fog or look while playing — or when a run reports E_GAME_TRAP, E_GAME_BUDGET, E_GAME_MEMORY, E_BATCH_VERSION, a rejected event such as E_ENTITY_STALE, E_TELEPORT_VISIBLE, E_SCENE_UNKNOWN or E_SCENE_VALUES, or hashes that differ between runs.
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
| no allocation when spawning | `ctx.reserve_entities(count)` in `Game::new`, with the most entities alive at once |
| the player to walk it | `SetHero { entity, speed }`; the move axis drives it natively |
| a place on the floor to report who stands in it | `ZoneCylinder { zone, center, radius, height }` (centre halfway up, feet within the radius and the height; a size below 0 is `E_ZONE_SIZE`), then `Event::TriggerEnter`/`TriggerExit`; `Zone` is a sphere, which bulges at mid-height |
| walk somewhere | `MoveTo { entity, target, speed }`, then `Event::Arrived`, or `Event::Blocked { entity, reason }`: `NoPath` (no way there), `Stuck` (held back on the way), `Stopped` (something else ended it), `Unavailable` (a traversal the room lacks) |
| what lies along a line: which side of the hero is free, what a look hits | `ctx.ray(&Ray { origin, dir, max, colliders: true, actors: true, exclude: hero })` → `Ok(Some(RayHit { distance, point, normal, entity, collider }))`, `Ok(None)` for a miss; it sees the room as the step began, before this tick's commands; `vm.rays_per_step` (256) a tick, then `Err(RayRefused::OverBudget)`. `TestHost::ray_hit` answers it in tests |
| a hidden actor out of the way | `SetCollider { entity: actor, enabled: false }`: others walk through it and doors swing through it; it still walks with collision. Hiding (`SetVisible`) alone leaves its body in the way |
| one actor to turn slower than the rest | `SetTurnRate { entity, rate }` in degrees per second, at most `motion.turn_rate` (set that to the fastest actor's rate); `rate: 0.0` goes back to the setting |
| later | `Timer { timer, duration, repeat }`, then `Event::TimerFired` |
| a smooth value | `Tween { … ease }` or `Spring { … }`, then `Event::TweenDone` |
| the room's light or look to change | `ctx.scene_tween(tween, &[(id("look.exposure"), 0.6), (id("fog.density"), 0.05)], 2.0, Ease::InOutSine)`, one `TweenDone` when all arrive; names in `docs/lighting.md`. `ctx.reserve_scene_tween(pairs)` in `Game::new` keeps it from allocating; size your own `put_scene_value` buffers with `SCENE_VALUE_BYTES` |
| a button's index | `ctx.button(id("action"))` in `Game::new`; never copy its place in `input.buttons` |
| a button press | `Event::Button { action, down }`, or `Event::Buffered { action, .. }` until you send `Consume { action }` |
| randomness | `ctx.rng()`, seeded by the engine; never another source |
| persistence | `Save { key, value }`; `SaveLoad {}` → `SaveLoaded`…, `SaveLoadDone`, always, if only as no save (a host without saves, or 5 s without an answer) |
| where an actor stands, for a save | `Locate { entity }`, then `Event::Located { entity, position, yaw }` at the next tick (yaw in (-180, 180]; an entity despawned in the asking step gets `Rejected` `E_ENTITY_STALE`); save those, never a guess from zones or the last target |
| many objects | `Transforms` bulk region: register once, `push` each tick |
| a name as id | `id("meet")`, a `const fn` |
| another room | `RoomSwitch { room, transition, duration }`, then `Event::RoomSwitched`; list it under `game.rooms` with its `neighbours` so it is loaded before. `Continuous` for a room whose geometry goes on (walk in, the camera keeps following its rig, or with no rig slides to the next room's camera); `Dissolve` (the last frame fades out over the next room, 0.5 s when `duration` is 0) for another place or a jump in time; `Cut` under a cutscene's fade |
| a room loaded ahead | `RoomPreload { room }`, beyond the neighbours |
| part of a room on or off | `RoomGroup { group, on }` (a sub-room); `PieceState { piece, state }` (0: as placed) |
| a light or the look to change | `Tween { property: Property::Scene, var: id("<parameter>"), .. }` |
| a room node or light to move | `ctx.spawn(Kind::RoomNode, id("<node>"))` or `Kind::Light`, then move the entity |
| a door to open or shut | `Tween { entity: Entity::NONE, property: Property::Scene, var: id("hinge.<room>.<piece>_<hinge>"), to: <degrees from closed>, .. }`; its collider turns with it. If an actor is in the swing the leaf stops, the tween waits, and `Event::HingeBlocked { hinge, entity }` arrives once: open again or wait. `examples/door/game/src/lib.rs` |
| particles | define the effect in `game.settings.fx.effects`; `FxStart { emitter, effect, entity, position, points, seed }` (on a point, from an entity, or along `PointList::Slice(&[…])`), `FxStop { emitter, fade }`, `FxBurst { effect, entity, position, count, seed }`; seed from `ctx.rng()`. Fields: `docs/fx.md` |
| a way to a goal | `Trail { trail, from: hero, points: PointList::Slice(&[goal]), spacing: 1.0, size: 0.2, color: [1.0, 0.6, 0.2], intensity: 0.55, wave_speed, fade_from, fade_to, fade }` once when the walk starts (the dots stay put as the hero walks); `TrailOff { trail, fade }` on arrival |
| something to use, within reach | `Highlight { entity: Entity::NONE, node: id("<node>"), fill: 0.12, rim: 0.35, pulse: 2.0, fade: 0.6 }` on `TriggerEnter`, the same with `fill` and `rim` 0 on `TriggerExit`; an actor by its `entity` |
| something in a hand | `Hold { entity, actor, socket: id("<socket>"), offset, keep }`; `actor: Entity::NONE` lets go where it is |
| a lantern | a spot `Kind::Light` held at the hand, `LightAim { entity, at: <the light>, target: [0, 0, -4] }`; `SetVisible` off frees its light place |

Test the logic natively with `sariema_sdk::testing::TestHost` (queue
events, `Runner::run`, decode with `testing::commands`;
`TestHost::with_buttons(&["confirm", "action"])` for the input buttons),
and the whole game
with `sariema run --headless … --json`.

## Common mistakes

| Symptom | Cause | Fix |
|---|---|---|
| `rejected` with `E_TELEPORT_VISIBLE` | something would move a visible actor faster than `motion.max_visible_speed`: `set_transform`, `teleport`, a fast `tween`, walk or velocity (a velocity at its speed scale), a rigid `follow`, `set_parent`, or moving what carries it. Opcode 0: the step's measurement put the actor back and stopped what moved it; the first event names the actor, the next ones each carrier stopped (a fast move begun under a fade or cut that lifted, or while hidden; a spring; a carrier turning or growing, even one spawned or placed this step; moves that add up). A walk stopped this way also ends with `blocked` | walk or ease it slower; hide it a step before; or keep the fade or cut over the whole move |
| `rejected` with `E_TURN_RATE` | `SetTurnRate` faster than `motion.turn_rate` or below 0 | raise `motion.turn_rate` to the fastest actor's rate in `game.settings` |
| `rejected` with `E_ENTITY_STALE` | a handle kept after `despawn` or a `despawned` event | drop handles when they die |
| `rejected` with `E_HOST_QUEUE_FULL` | more `save`, `host_send` or other host messages than the host takes between its frames (256, or 4 MiB) | send fewer or smaller ones; save when something changes, not every tick |
| `rejected` with `E_SCENE_UNKNOWN` from `scene_tween` | a parameter the active room lacks (`sun.*` in a room without a sun, a light's name misspelt); nothing was tweened | list the room's parameters with `sariema inspect <room>.srma --json` (`params`) |
| `rejected` with `E_SCENE_VALUES` | `values` not a whole number of `SCENE_VALUE_BYTES` pairs, or a value not finite | build them with `ctx.scene_tween` (or `put_scene_value`) |
| `E_BATCH_VERSION` at load | the logic was built against an older SDK (version 2 added `reason` to `blocked`) | rebuild it against the engine's SDK |
| every `ctx.ray` misses | the pack carries no room collision, or the room is still loading | check `game.room` has collision; cast after `RoomSwitched` |
| `E_GAME_TRAP` at step 0 with "time not implemented" | `std::time` | count steps (`ctx.step()`) |
| `E_GAME_BUDGET` | heavy work in one tick | spread it over steps; bulk regions for many placements |
| `state_hash` changed after a rebuild, same log | it covers the module's memory, which a rebuild moves | compare `world_hash` across builds, `state_hash` within one |
| a press acted on twice | reading `Buffered` without `Consume` | send `Consume` once handled |
| a door that never finishes shutting | an actor stands in its swing: the tween waits for it | handle `HingeBlocked` (open again, retry on a timer) |
| an actor walks into a closed door and gets `blocked` | paths go through doors whatever their leaf; nothing opened it | open it first: tween it on a `trigger_enter` near the door |
| no particles, `rejected` with `E_FX_UNKNOWN` | the effect is not in `game.settings.fx.effects`, or the name differs | define it there; `id("<name>")` of the same name |
| fewer particles than the rate gives | the pool is full: an emitter keeps at most `pool` alive | raise the effect's `pool` (or `fx.pool`), or shorten `life` |
| sparkles that never stop after `FxStop` | a new `FxStart` of the same id each step | start once, stop once: a running emitter keeps going |
| a trail whose dots slide along the floor | a new `Trail` each step with the hero's place as the first point | send it once with `from: hero`; the engine follows the hero and keeps the dots still |
| `rejected` with `E_SCENE_UNKNOWN` after `Highlight` | no node of that name in the active room (a loose piece is `<group>.<piece>`, as `a.mesa`) | `sariema build <dir> --room <room> -o r.srma` then `sariema inspect r.srma` lists them under `loose draws`; highlight after the room switch |
| a dark piece glows pale, a light one turns white | `fill` too high: the tone curve washes bright colours | keep `fill` near 0.1 to 0.25, the `rim` two or three times it |
| nothing in the module's log after a panic | no panic hook | `std::panic::set_hook(Box::new(\|i\| sariema_sdk::report_panic(i)))` under `#[cfg(target_arch = "wasm32")]` |
