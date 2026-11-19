# The game API

A Sariema game is data plus one WebAssembly module of logic. The engine calls
the module once per fixed simulation step; the module reads that step's
events and writes commands. Everything that repeats every frame (movement,
tweens, springs, followers, timers, the camera, cutscene playback) runs
natively in the engine. The game describes; the engine executes.

`sariema api --json` prints every command, event, enum, setting and error
code described here, generated from the same table the decoder is generated
from (`crates/sariema-format/src/script/table.rs`), and matching
`schemas/api.schema.json`. Treat it as the reference; this page explains how
the pieces fit.

## The module

A game module exports:

| Export | Signature | |
|---|---|---|
| `sariema_tick` | `() -> ()` | One step: read events, submit commands. |
| `sariema_batch_version` | `() -> i32` | The batch version it was built against; any other, or none exported, is refused at load with `E_BATCH_VERSION`. The SDK's `game!` exports it. |
| `memory` | memory | Its linear memory. |

and imports only these functions from module `sariema`:

| Import | Signature | |
|---|---|---|
| `submit` | `(ptr, len)` | Appends `len` bytes of commands. |
| `events` | `(dst, cap) -> i32` | Copies the next whole events of this step into `dst`: the bytes copied, 0 once all were read, or minus the size of an event larger than `cap`. |
| `register` | `(kind, ptr, len) -> i32` | A bulk region the engine reads in place after every tick; its id, or −1. Kind 1 is transforms. |
| `log` | `(level, ptr, len)` | A line of UTF-8 text, level 0 (debug) to 3 (error). |
| `button` | `(name) -> i32` | The index `button` events carry for the input button whose name has the id `name`, or −1. |
| `ray` | `(query, query_len, hit, hit_len) -> i32` | Casts the ray record at `query` against the active room ([Rays](#rays)): 1 with the hit record written at `hit`, 0 for a miss, −1 for a record that is not a ray (a query or hit length other than the records' is refused before anything is read), −2 past `vm.rays_per_step`. |

Nothing else exists: no files, no network, no clock, no system calls. A
module that imports anything else does not load (`E_GAME_MODULE`).

## The step

The simulation advances in fixed steps of 1/60 s (`STEP_HZ` in
`sariema-core`). Each display frame adds its real duration to an integer
accumulator and runs the whole steps it owes, at most
`max_steps_per_frame` (default 5); time beyond that is dropped, so a stall
slows the game instead of freezing it. Rendering interpolates between the
last two steps with the leftover fraction. The same game pack and input log
give a bit-identical state at 30, 60, 120 or 144 Hz or with irregular frames.

One step, in order:

1. Input is sampled into actions.
2. The game's tick runs. Its events are the ones produced since its last
   tick, then this step's input changes.
3. Its commands are applied in the order sent, then its bulk regions.
4. The cutscene sequencer runs.
5. The hero is driven by the move axis (when the player has control), the
   room's hinged leaves turn to the angle their tween or the game gives
   them unless an actor is in the way, actors move and turn, what they
   hold is placed on its socket, followers, tweens, springs and timers
   advance, the fade, cinema bars and dialogue advance.
6. World placement is resolved and measured, putting back a visible actor
   that went too far ([Entities](#a-visible-actor-never-jumps)), the camera
   rig steps, trigger zones report.

Events produced in step 5 and 6 (`arrived`, `tween_done`, `trigger_enter`,
`hinge_blocked`…) reach the game at its next tick.

A skipped cutscene ends in this order. Once the skip's fade has covered the
screen, step 4 runs the rest of the cutscene at once; the next tick gets
the `cutscene_call`s it ran, each with `skipped: true`, then
`cutscene_ended { skipped: true }`, all in one batch while the screen is
still covered. The game sets its end state in that tick (teleports rather
than walks or slides), and the fade back in starts at step 4 of that same
step, unless the tick sent its own `fade`. A cutscene played through sends
`cutscene_call` with `skipped: false` as each call step runs.

## The batch

Commands and events travel as one binary batch each way. A batch is a run of
records: a little-endian `u16` opcode, then the fields its table entry lists,
in order, with no padding. `f32` fields must be finite; `bool` is one byte,
0 or 1; `Str` and `Bytes` are a `u32` length then the bytes, `Str` being
UTF-8; an `Entity` is a slot index and a generation, `u32` each; `Vec3` is
three `f32`; `Points` is a `u32` count then three `f32` per point (the SDK
takes a `PointList::Slice(&[[x, y, z], …])`); enums are one byte.

A command batch that does not decode stops the game for good with
`E_BATCH_TRUNCATED`, `E_BATCH_UNKNOWN` or `E_BATCH_INVALID`, naming the
record and byte offset. The version is checked once, at load, never per
step. Records carry no length, so a field added to a message changes the
layout and the version: version 2 added `reason` to `blocked`, version 3
`hold` to `teleport`, `hold_end` to `clip_path`, `held` to `located` and
`skipped` to `cutscene_call`, and a pack built against an older one is
refused with `E_BATCH_VERSION` until it is rebuilt against the current
SDK. Game packs older than the
current one (5) carry such logic, so they are refused the same way, from
the batch version their head names. A module that does not export
`sariema_batch_version` is refused the same way rather than read as some
version it may not speak.

Names (cutscenes, roles, flags, clips, sounds, signals) travel as ids: the
FNV-1a hash of the name's UTF-8 bytes (`name_id`, a `const fn` in the SDK).
`sariema run --headless` prints the names a pack declares beside their ids.

## Entities

The game chooses the slot of every entity it spawns, and the engine checks
every handle it is given. A slot is reused with a newer generation, so an
old handle is refused with `E_ENTITY_STALE` instead of reaching whatever
lives in the slot now. Slots are bounded by `max_entities`. The SDK keeps the
free list: `ctx.spawn` takes a slot, `ctx.despawn` and the engine's
`despawned` event free it.

Kinds: `empty`, `room_node`, `actor`, `light`, `camera`, `sound_emitter`,
`ui_node`. Actors are what moves, walks, turns and plays clips.

### A visible actor never jumps

The frames of a step draw each entity sliding from where the previous step
left it to where this step ends. A visible actor is never drawn going faster
than `motion.max_visible_speed` (20 m/s) nor turning faster than
`motion.turn_rate` (540°/s), except on a step the screen hides. The rule is
held in one place, on that slide: once every system has run in a step and
the world is resolved, each actor visible when the step began and still
visible is measured, carriers first, from where the previous step left it to
where it ends. One that went too far is put back where it was, whatever
carries it now. So whatever moved it is caught: a command, a walk, a tween
or spring, a follow, a carrier that moves, turns or grows and sweeps it
round, a transform region moving its carrier, or several moves that each
pass but add up.

Nothing else exempts an actor:

- a forced `teleport` of the actor, or of a carrier it has kept all step;
- a covered step, the screen fully faded when it begins or when it ends, or
  a cut, the camera cutting in it.

An exempt actor that went too far is drawn where it ends, never sliding
across the screen. What is done to a carrier passes nothing else on: a
carrier spawned, placed or given another parent this step exempts no actor
riding it, and a carrier an actor was given this step does not carry it
along its forced teleport.

An actor that appears, spawned or shown this step, is drawn from where it
ends, never sliding in from where it was hidden. Hiding and showing it in
the same step hides nothing: it was seen when the step began, so it is
measured. Anything that is not an actor is drawn from where it ends when it
was spawned, placed or given another parent this step, or when what carries
it is; otherwise it slides, unmeasured.

When the step's own motion carried the actor off, what moved it stops. Each
level, the actor and then each carrier up, is judged by how far its own
change this step carries the actor: the levels that alone carry it past the
limits stop; when none does alone but together they do, every level that
changed stops. A level stops its tweens and springs of placement, its follow
and its walk, which ends with `blocked` (a cutscene's `walk` ends there
too, whether the runtime or a collision module moves actors). An actor that went too far only because what carries it
was put back is put back too, and nothing of its own stops. A transform
region is not stopped: while it keeps moving a carrier too fast, the actor
riding it is put back every step. A carrier scaled below
`motion.min_carrier_scale` (10⁻⁶) cannot hold the actor anywhere, so the
actor leaves it. The step's `rejected` events,
with opcode 0 and `E_TELEPORT_VISIBLE`, name the actor put back, then each
carrier stopped.

Placements are `f32`, so the measurement is exact to their rounding: under a
millimetre within 10 km of the origin, more beyond (a third of a metre at
1,000 km).

A command that surely jumps is refused when given, with its own opcode, and
changes nothing: `set_transform` and `teleport` (always too far for an
actor), `tween` (its fastest step), `follow` (its first step), `move_to`,
`move_intent`, `set_hero` and `set_speed_scale` (the velocity times the
actor's speed scale; a `move_to` at its own speed), and `set_parent` (the change of world placement). It
is refused with `E_TELEPORT_VISIBLE` when it would carry too far an actor
visible when the step began that no forced teleport placed: the entity
itself, a child at any depth (going as far) or a follower (going its lag's
share). It is allowed when the screen is fully faded, the camera cuts in the
same step, or `teleport` says `force`; what is allowed is still measured at
the end of the step.

A refused command produces a `rejected` event with the opcode, the code and
the entity. The game keeps running.

## The Rust SDK

`crates/sariema-sdk` is `no_std` (with `alloc`) and generated from the same
table as the decoder, so it cannot disagree with the engine.

```rust
use sariema_sdk::{Command, Ctx, Event, Events, Game, Kind, game, id};

pub struct Hello { hero: sariema_sdk::Entity }

impl Game for Hello {
    fn new(ctx: &mut Ctx<'_>) -> Hello {
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetHero { entity: hero, speed: 2.0 });
        Hello { hero }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            if let Event::TriggerEnter { entity, .. } = event {
                if entity == self.hero { ctx.play_cutscene(id("meet")); }
            }
        }
    }
}

game!(Hello);
```

- `Game::new` runs on the first tick, after the SDK read that tick's events
  (so `ctx.rng()` is already seeded); `Game::tick` runs every tick.
- `ctx.send(Command::…)` is the typed builder for every command; commands
  apply after the tick, in order.
- `ctx.spawn`, `ctx.reserve` (a handle for an entity a cutscene spawns),
  `ctx.despawn`, `ctx.entities().is_alive(e)`. `ctx.reserve_entities(count)`,
  called while the game sets up with the most entities it keeps alive at
  once, keeps spawning and despawning them, and their commands, from
  allocating.
- Cutscene control: `ctx.bind(role, entity)`, `ctx.play_cutscene`,
  `ctx.skip_cutscene`, `ctx.signal`.
- `ctx.scene_tween(tween, &[(id("fog.density"), 0.05), …], duration, ease)`
  tweens scene parameters of the active room together.
  `ctx.reserve_scene_tween(pairs)`, called while the game sets up with its
  largest tween's count, keeps every later one from allocating. A game that
  builds `values` itself with `put_scene_value` sizes its buffer with
  `SCENE_VALUE_BYTES`, the bytes one parameter takes, the same constant the
  engine's decoder reads by.
- `ctx.rng()` is a PCG32 seeded from the engine's `started` event.
- `ctx.button(id("action"))` is the index `button` events carry for the
  input button named `action`, whatever its place in `input.buttons`;
  `None` when the project has none.
- `ctx.ray(&Ray { origin, dir, max, colliders, actors, exclude })` is the
  first thing the ray hits ([Rays](#rays)): `Ok(Some(RayHit))`, `Ok(None)`
  for a miss, `Err(RayRefused)` for a ray that is not one or past the
  step's budget. `TestHost::ray_hit` sets what it answers in tests, and
  `TestHost::rays` keeps what was cast.
- `Machine<S>` holds a state, the step it was entered and an `entered()`
  flag, for logic written as states.
- `Transforms` is a bulk region of kind 1: entity placements applied after
  every tick without one command each. Its buffer is sized once; register it
  once with `ctx.register(Transforms::KIND, region.as_bytes())`. Actors are
  skipped; they move by commands.
- `testing::TestHost` runs a game natively in `cargo test` (with
  `TestHost::with_buttons` for its input buttons): queue events,
  `Runner::run` a tick, decode the commands with `testing::commands`.
- `report_panic` sends a panic's message to the engine's log; call it from a
  std game's panic hook. A `no_std` game enables the `panic-handler` feature.

The logic crate is a `cdylib` with its own `[workspace]`, built with
`cargo build --release --target wasm32-unknown-unknown`. `sariema build`
does that, with the project's `game.toolchain` when it names one. The pack
also carries what the game plays with, so one file runs it on every host:
the room `game.room` names (the project's first without it) and its
collision, every actor in `actors`, the sound bank of `audio`, and the HUD
`game.hud` describes with its fonts and images. An entity spawned with an
actor's name id as its `asset` is drawn and animated with that actor.

Rustc embeds source paths in the logic, in its panic locations, and the
logic's memory is part of the state hash. So the pack and its hashes are the
same on every machine, `sariema build` passes `--remap-path-prefix`: the
checkout the CLI was built from becomes `/sariema` and cargo's registry
sources `/cargo`. The flags go after any in `CARGO_ENCODED_RUSTFLAGS`,
`RUSTFLAGS` or `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS`, and with none
of those set they join `build.rustflags` in cargo's config. Two cases are
not remapped, and the build reports each in `warnings` when the `.wasm`
still names the checkout, the home directory or cargo's:

- a `target.wasm32-unknown-unknown.rustflags` in cargo's config, which
  replaces `build.rustflags`; set those flags in `RUSTFLAGS` instead;
- an SDK from another checkout than the CLI's; build with that checkout's
  `sariema`.

## Rays

A game asks the active room what lies along a line with the `ray` import,
during its tick, and gets the answer at once. The engine answers from the
room as it stood when the step began: the game ticks before anything moves,
so the commands it sent earlier in the same tick have not moved anything
yet. A ray passes through every shape its origin lies inside, a collider
or an actor's body alike, whichever way it goes, as Godot's rays do unless
`hit_from_inside` is set: a ray cast outward from inside the hero sees what
is around the hero. The answer is the same on every host and every frame
rate, and a cast allocates nothing.

The query is a fixed little-endian record of 37 bytes
(`sariema_sdk::ray::Ray::BYTES`): origin and direction (three `f32` each),
the longest distance (`f32`), a flags byte (1: the room's colliders, 2:
actors' bodies) and an entity the ray passes through (index and generation,
`u32` each), such as the actor casting it. The direction may have any
length but zero; a negative or non-finite number makes the record not a
ray. Colliders are the enabled ones, static, switched or hinged, where they
stand; actors are the capsules of actors in other actors' way, so a hidden
actor is skipped unless `physics.hidden_actors_block` is on.

The hit is a record of 40 bytes (`RayHit::BYTES`): the distance from the
origin, the point, the surface's normal facing the origin (three `f32` each
for both), the entity (the actor hit, or the entity that last switched the
collider hit, or none) and the collider's name id (0 for an actor). The
nearest of the two wins; an actor exactly as far as a wall is behind it.

A game whose pack carries no room collision, or whose room is still being
built, gets a miss for every ray. Past `vm.rays_per_step` rays in one tick,
each further ray answers −2 without being cast.

## Budgets and the sandbox

Every tick runs under limits from the `vm.*` settings:

| Setting | Default | Exceeded |
|---|---|---|
| `vm.memory_bytes` | 64 MiB | `E_GAME_MEMORY` (growth refused, or the initial memory is larger) |
| `vm.fuel_per_step` | 20,000,000 | `E_GAME_BUDGET` |
| `vm.fuel_first_step` | 200,000,000 | `E_GAME_BUDGET` |
| `vm.max_call_depth` | 1000 | `E_GAME_TRAP` |
| `vm.max_regions` | 16 | `register` returns −1 |
| `vm.log_bytes_per_step` | 4096 | the rest of the step's log is dropped |
| `vm.log_lines_per_step` | 64 | the step's further lines are dropped |
| `vm.commands_bytes_per_step` | 4 MiB | `E_GAME_BUDGET` |
| `vm.rays_per_step` | 256 | `ray` answers −2 |

Fuel is about one unit per WebAssembly instruction. Exceeding any limit, or
any trap (`unreachable`, out-of-bounds access, a panic), stops the game for
good: the tick's commands are discarded, the host receives a `stopped`
message with the code, and the engine itself keeps running and keeps
stepping its native systems. A step is never resumed half-way. An infinite
loop is stopped within its step.

### The interpreter

The engine interprets WebAssembly on every platform. Two maintained
interpreters were measured behind the same `Interpreter` trait
(`crates/sariema-vm`), on the M1 load case (`crates/sariema-runtime/benches/swarm`:
2,000 objects, each deciding every step whether to rest, pick a goal or
steer toward it), on an Apple M-series Mac in release, median of the logic
tick over 600 steps:

| | wasmi 2.0 | tinywasm 0.12 beta |
|---|---|---|
| 2,000 `set_transform` commands per step | 0.51 ms | 2.01 ms |
| 2,000 placements through a bulk region | 0.28 ms | 1.28 ms |

The machine was shared with other builds; under heavier load the same runs
measured up to 1.4 ms and 0.7 ms for wasmi and 7.5 ms and 5.3 ms for
tinywasm, the ratio between them holding at four to eight.

wasmi is the engine's interpreter: four to eight times faster here, a stable
release, fuel metering and NaN canonicalisation built in, and it builds for
`aarch64-apple-ios` and `aarch64-linux-android`. tinywasm stays behind the
`tinywasm` feature of `sariema-vm` so the comparison can be re-run:

```sh
sariema build crates/sariema-runtime/benches/swarm -o swarm.srmg
cargo run --release -p sariema-runtime --features tinywasm --example m1 -- swarm.srmg
```

The swarm needs about 6.3 million units of wasmi fuel per step, so the
default budget of 20 million leaves about three times that. Its module
uses 1.5 MB of linear memory. Decoding its 52,000-byte batch of 2,000
commands takes 0.015 ms; the engine's native systems step 2,000 tweens in
0.033 ms, and 2,000 visible actors walking, each measured against the
no-jump rule, in 0.035 ms; a steady step allocates nothing on the host.
The target of at most 2 ms for game logic on the floor iPhone is not yet
confirmed: no device run has been made.

## Determinism

- One seed, given at start and delivered in the `started` event, drives the
  engine's PCG32 and the SDK's.
- WebAssembly NaNs are canonicalised; the interpreter has no clock or other
  source of variation.
- Time is counted in steps. Timers, tweens and waits of d seconds last
  round(d · 60) steps, exactly.
- The state hash covers the world (entities, transforms, flags, variables,
  fade, cinema bars, camera, control), the step, the random state, the
  cutscene playing and the module's memory. The world hash covers the world
  alone and does not change when the module is rebuilt.
- Float results of the engine's own math (trigonometry for the camera and
  turning) come from the platform's math library; hashes are compared on one
  platform.

## Input

Games see abstract actions, never keys. The default bindings
(`input.axes`, `input.buttons`):

| Action | Keyboard | Gamepad | Touch |
|---|---|---|---|
| `move` (axis) | WASD, arrows | left stick | the UI's joystick |
| `confirm` | Space, Enter, E | South | |
| `back` | Backspace | East | |
| `skip` | Escape | Start | |

An `axis` event reports a changed axis (x right, y up, each in [−1, 1]), a
`button` event each press and release. A press stays buffered for
`input.buffer_steps` (6) steps: a `buffered` event reports it every step
until the game sends `consume` or it expires, so a press made a few steps
early still counts. Handing control over (`control`, or a cutscene ending)
clears the buffer.

The hero (`set_hero`) is driven natively by the move axis while the player
has control, relative to the camera's view when `motion.camera_relative` is
on. `control` hands the hero to the player, the script or a cutscene; a
hero taken from the player lets go of the move axis, so it comes to rest
unless the script walks it. This and a walk's end dropping the stick
change `physics_hash` and the replays of games that walked the player
into a script walk, and those replays need recording again.

Input is recorded as the steps at which actions changed
(`schemas/input-log.schema.json`):

```json
{ "changes": [
  { "step": 30, "axes": { "move": [0, 1] } },
  { "step": 200, "axes": { "move": [0, 0] }, "down": ["confirm"] },
  { "step": 201, "up": ["confirm"] }
] }
```

`sariema run --headless --game pack.srmg --input log.json --steps N --json`
replays it in [the whole engine](engine.md), collision, animation, sound and
HUD included, and prints the events, host messages, log, saves and timing,
with `engine_hash` (the runtime, the collision world and every animator: the
same on every platform, wasm32 included), `state_hash` and `world_hash` (the
runtime's), `physics_hash`, and `audio_hash` over the samples it mixed
(repeatable with the same build, not across builds). `timing` gives the
mean, median and slowest step. With `--screenshot` it also draws the
frames a window would show at the end of the steps it keeps, the last
one or `--filmstrip N` spread over the run, without changing any hash;
without `--headless` the pack plays in a window.

## The engine's systems

- **Movement**: `move_to` walks at a speed, in metres per second of the
  world whatever the actor's speed scale, and ends with `arrived` (exactly
  on the target) or `blocked`, whose `reason` says why: `no_path` (no path
  leads to the target, or a skipped cutscene's walk could not have reached
  it), `stuck` (it stopped getting nearer: a closed door or an actor held
  it back), `stopped` (the engine stopped it: the jump rule, or the clip a
  clip path follows stopped) or `unavailable` (an unknown traversal, or no
  collision module to carry one); `set_speed_scale` scales the player's axis
  and `move_intent`; `move_intent` holds a velocity; `face` turns at
  the actor's turn rate, in place when nothing drives the actor (it drops
  the speed left from a walk); a walk under way keeps its speed. `motion.turn_rate` is every actor's rate and the
  fastest anything turns; `set_turn_rate` gives one actor a rate of its own,
  no faster (`E_TURN_RATE` otherwise), and 0 gives it the setting back, so
  a big hero and a small companion turn each at its own speed under the
  same jump limit. `set_collider` on an actor switches its body: off, a
  hidden or stowed actor is no invisible obstacle (others walk through it,
  doors swing through it), while it still walks with collision; the body
  stays off in every room until the game turns it on. A hidden actor
  (`set_visible` off) is in no other actor's way by default, as an unseen
  body would read as an invisible wall; it still stands on floors and stays
  out of walls. `physics.hidden_actors_block` keeps hidden actors in the
  way, as before. This, and a walk held back by an actor now ending
  `blocked` after `move_to.stuck_steps`, change `physics_hash`, and replays
  recorded before need recording again. With `motion.builtin` the runtime moves actors
  itself; a collision module takes the same intents
  ([the engine](engine.md) wires `sariema-physics` in when the room has a
  collision pack). `traverse` carries an actor along one of the room's
  declared traversals, collider off, over `duration` seconds timed by
  the traversal's `ease` ([physics](physics.md#traversals-and-intended-contact)),
  and ends with `arrived`; without a collision module it ends at once
  with `blocked`. A traversal that declares `hold_end` leaves the actor
  held at its last point (see "Held ends").
- **Held ends**: a kinematic move can end with the actor held where it
  stops instead of landing: a `traverse` whose traversal declares
  `hold_end`, a `clip_path` with `hold_end`, a cutscene `path` step with
  `"hold_end": true`. The actor stays exactly there, posed, collider off,
  with no gravity and no push-out (a hand on a window leaf stays on it, a
  climber stays at the top of a rope), until its next movement command:
  `move_to`, `move_intent`, `stop`, `knockback`, `teleport`,
  `set_transform` (on a hidden actor; on a visible one the no-jump rule
  refuses it and the hold stays), another `clip_path` or `traverse`, or
  the player's stick when it drives the hero. A path or traversal given
  to a held actor starts from the held pose and keeps it posed until it
  moves; a
  walk or a stop gets the collider back where it is, so a held actor high
  up falls. A hero held while the player has control falls at the first
  push of the stick: keep `control` with the game until its next move. A
  tween, spring or follow that moves a held actor keeps it held at its new
  place. A refused `traverse` leaves the hold as it was. A carried actor
  (one with a parent) cannot be held, and giving a held actor a parent
  releases it. `located` says `held: true` for a held actor, and `teleport`
  with `hold` puts an actor back held, so a save brings it back as it
  was.
- **Hinged leaves**: a door or window leaf of the room turns with its
  scene parameter `hinge.<node>` (degrees from closed), and its collider
  with it. A `tween` of it that would turn the leaf into an actor stops
  for that step instead, and the game gets `hinge_blocked { hinge, entity }`
  (the parameter's id and the actor) once, until the leaf moves again; the
  tween goes on once the way is clear, and the game may open the door
  again or cancel it. `examples/door` opens a door when the hero comes
  near and reopens it when it stops on the hero.
- **Tweens and springs** on position, yaw, pitch, roll, scale, a world
  variable or a scene parameter of the active room (its light, sun,
  hemisphere, fog, look and hinges; `property: scene`, `var` its name id),
  with 20 easings (Penner's, plus smoothstep). A new animation of a
  property starts from its current value, and a spring keeps its velocity, so
  interrupting never jumps. A spring settles exactly on its target.
  `scene_tween` tweens several scene parameters together with one
  `tween_done`; see [lighting](lighting.md#changing-the-light-and-the-look-while-playing).
- **Followers** keep an entity at another's position plus an offset,
  closing the gap with a half-life.
- **Timers** fire after a whole number of steps, once or repeating.
- **Zones**: spheres (`zone`) or upright cylinders (`zone_cylinder`)
  reporting actors entering and leaving by where their feet are. A
  cylinder takes the feet within its radius of its axis and between its
  bottom and top, so a place drawn as a circle on the floor counts the
  same at the floor and on a ramp or step within its height, where a
  sphere would bulge out in the middle and miss the rim. Both kinds share
  one set of ids: setting one replaces the other, and `remove_zone`
  removes either. A cylinder with a radius or height below 0 is
  `rejected` with `E_ZONE_SIZE`; a NaN or infinite one, like any
  non-finite number in a command, stops the game at decoding.
- **Camera rigs**: follow a subject (dead zone, spring, look-ahead, bounds),
  frame two subjects, or a fixed pose; `camera_use` blends from the pose on
  screen to the new rig over `blend` seconds with an easing. Reframing the
  rig in use (`rig_follow`, `rig_frame`, `rig_fixed`) is shown as is when it
  is gradual: the same subjects, nothing moving faster than
  `motion.max_visible_speed` or turning faster than `motion.turn_rate`, like
  a camera a stick turns every step. A sudden reframing blends from the pose
  on screen over `camera.reframe_blend`. A blend of 0 is a cut,
  the only way the camera moves discontinuously. `camera_shake` is seeded
  and deterministic. `rig_anchor` aims a rig at a bone or socket of its
  subject; the camera lists those anchors for the animation module to
  resolve and aims at the subject's origin until it does.
- **Clips**: `play_clip` asks the animation module for a clip at a `speed`.
  The runtime keeps each actor's clip time in steps, and the animation
  module seeks the clip to it instead of running it on its own clock:
  `clip_time` is the time at the last step, which `clip_marker` and
  `clip_finished` are judged against; `clip_time_at(alpha)` is the time to
  draw, between the last two steps at the frame's `alpha`, like every pose.
  In slow motion it still grows every frame. A clip begun in a step shows
  its first frame until the next step: the frames drawn then lie between
  the step before, which had no clip, and the one that started it, so there
  is no earlier time to show. Start a clip one step before it must move.
  The engine tells the runtime how long each actor's clips last.
- **Clips bound to the root**: `clip_path` plays a clip from `from`
  seconds into it, still for `hold` seconds first (under a fade or a cut:
  the pose does not move), and moves the actor's root only within the
  clip's spans (`ClipSpan`, written with `ClipSpan::put` into `spans`, in
  order of the clip's time). Within a span the clip plays at the span's
  `rate` (0.4 is slow motion on that span alone) and the root goes from
  where it stood when the span began to the span's point, eased, rising
  `arc` metres halfway, and turning to its `yaw`; outside the spans the
  root stays while the clip anticipates or settles, at its own rate. The
  root is placed each step from the clip's time, so a slowed span, slow
  motion or the hold keep body and root together, and a rate due inside a
  step takes over within it. With a collision module the actor is posed
  with its collider off for the spans (over a sill, onto a ledge) and gets
  it back where the root stands when the last span ends, pushed out of any
  overlap, then hears `arrived`, or with `hold_end` stays held where the
  root stands (see "Held ends"); a `move_to`, `move_intent`, `stop` or
  `teleport` ends the path where the root stands, and another clip ends it
  with `blocked`. Refused with `E_CLIP_SPANS` (out of order, overlapping,
  past the clip's end, a rate of 0, not finite) or `E_TELEPORT_VISIBLE`
  (a span faster than `motion.max_visible_speed`, or turning faster than
  `motion.turn_rate`, for a visible actor).
- **Clips in a row**: `clip_next` queues a clip after the one an actor
  plays and those queued after it. It begins once the one before has
  played `after` seconds, or at its end when `after` is 0, inside the step
  that happens, with the time left over: the seam shows no still frame and
  loses no time, where waiting for `clip_finished` and then playing loses a
  step or two. The animation module crossfades over `blend`. A `play_clip`
  or `clip_path` drops the queue; with no clip playing it plays at once.
- **Held things**: `hold` puts an entity (a room node, a light, an empty)
  in an actor's hand: at its socket or joint `socket` (a socket looked up
  first), staying where it stands on it with `keep`, else sitting at
  `offset` from it in the socket's frame. Each step, once the actors have
  moved and the animation module has posed them, it is placed on the
  socket rigidly, in metres of the world: the actor's scale never grows it
  or its offset. Holding takes it out of its parent, follow and tweens.
  `hold` with `actor` NONE lets go and it stays where it was, so a tween
  can carry it on from there (a bag into a bin); despawning the actor lets
  go too. Refused with `E_HOLD_ACTOR` for an actor to hold and, the step
  after, `E_SOCKET_UNKNOWN` for a name the actor's model lacks; an actor
  drawn without a model keeps it where it stands. While held, a room node
  is out of every cutaway ([lighting](lighting.md)), and goes back when let
  go.
- **A lantern in the hand**: a room's spot light spawned as a `light`
  entity, held at the hand, and aimed with `light_aim` at a point ahead in
  its own frame (or the carrier's) lights where the hand points as the
  actor walks and turns; the target is in metres of the world whatever the
  scale of the frame. `at` NONE aims at a point of the world; a point light
  has no aim. Hidden with `set_visible`, the light is off and takes no
  place among the frame's lights, so a lantern switched off frees its place
  for another light. `E_NOT_LIGHT` refuses an entity that is not a light.
  "Ahead" is +Z of the model in a socket's frame, and −Z in an actor
  entity's frame.
- **A clip's rate while it plays**: `clip_rate` plays the clip an actor
  plays at a new rate from the step it arrives, going on from the time it
  is at: no restart and no crossfade, where a second `play_clip` starts it
  over and blends. A game that eases its hero from a stroll to a run sets
  the walk cycle's rate to the pace over the clip's ground speed each step
  the pace changes, and the feet keep to the ground. A rate of 0 holds the
  clip where it is. A rate sent during a `clip_path` lasts until the next
  span begins with its own; the clips queued with `clip_next` keep theirs.
  Refused with `E_CLIP_NOT_PLAYING` when the actor plays another clip or
  none, and `E_CLIP_RATE` below 0. A `play_clip` or `clip_next` at a
  speed of 0 or below is refused with `E_CLIP_RATE` too: it would never
  reach its end.
- **Time scale**: `time_scale` plays the world slower or faster than real
  time, reached over `blend` seconds. The display frames' time is scaled
  before it becomes steps: a step stays 1/60 s of world time, so play stays
  deterministic, and everything that follows the steps (movement, tweens,
  clips, waits) slows together. Leftover frame time carries into the next
  step. The scale stays within `min_time_scale`, so the world never stops,
  and `max_time_scale`.
- **HUD**: `ui_visible`, `ui_text` and `ui_value` change nodes of the HUD by
  name, and `ui_play` plays one of its animations, so a game binds its
  events to the HUD (an objective that slides in on `trigger_enter`).
  While a cutscene holds control the engine hides the HUD and brings it
  back, as the game last set it, once the cinema bars are gone; a root
  node in the group `cutscene` stays. A text longer than the engine's
  `hud.max_text_bytes` (4096) is refused with `E_UI_TEXT_TOO_LONG`, and
  one whose glyphs overflow the HUD's glyph atlas with `E_UI_ATLAS_FULL`;
  a text refused for its length leaves the node as it was, and one refused
  for the atlas leaves it empty.
- **Particles**: `fx_start`, `fx_stop` and `fx_burst` run emitters of the
  effects the game defines in `fx.effects`, seeded by the command, simulated
  on the step and drawn in one instanced draw; see [effects](fx.md).
- **Guides**: `trail` lays soft dots along a route to a goal, with a wave of
  light running to it, and `trail_off` fades it; `highlight` makes an actor
  or a room node glow in its own colour, over all of it and at its rim,
  pulsing slowly, and eases it in and out; see [effects](fx.md).
- **Screen**: `fade` to a colour, `letterbox`, and dialogue lines (`say`):
  an `auto` line ends after its duration, a `tap` line when the game sends
  `advance_line` (in a cutscene, on the `cutscene.advance_action` button);
  `line_done` reports the end. A line the HUD cannot show, longer than
  `hud.max_text_bytes` or overflowing the glyph atlas, is reported with
  `rejected` (`E_UI_TEXT_TOO_LONG` or `E_UI_ATLAS_FULL`) and hidden; it
  still ends as its `advance` says.

## Saves and the host

`save` asks the host to persist a value under a key; `save_load` asks for
every saved key, answered by `save_loaded` events and one `save_load_done`.
`host_ready`, `host_exit`, `host_orientation` and `host_send` are messages to
the host; `host_message`, `orientation_changed` and `exit_requested` come
back as events.

Every `save_load` gets its `save_load_done`, so a game may wait for it
before it starts:

- a host without saves (the bench, a page that does not listen for loads)
  answers it as no save at once: `save_load_done` the next step;
- a host with saves answers when its store does, which in a WebView app
  takes a round trip of a few frames; one that has not answered after
  `host.save_load_timeout_s` (5 s of game time) is answered as no save, and
  the engine logs it.

Messages wait for the host in a queue of at most `host.max_messages` (256)
messages and `host.max_bytes` (4 MiB); one past them is refused with
`rejected` `E_HOST_QUEUE_FULL`. These are the engine's settings, which the
host sets, not the game's ([the engine](engine.md#messages-to-the-host)).

`sariema run --headless --saves saves.json` starts with a save store and
prints the store at the end. `sariema run --game` keeps saves in
`--save-dir` (`saves` beside the pack by default) as `<pack name>.json`, in
the same format, so a window's saves replay headless; `host_exit` closes
the window. A page gets every message from `on("host")` and answers loads
from `on("saveLoad")` ([web.md](web.md#host-messages-and-saves)).

A save that brings an actor back where it stood asks the engine, which
alone knows where collision left it: `locate` names the entity, and at the
next step `located` gives its feet in the world and its yaw as the step it
was asked in left them, after collision and the no-jump rule. The yaw is
in degrees, in (-180, 180], whether something carries the actor or not.
An entity despawned before that step ends gets `rejected` with
`E_ENTITY_STALE` and the opcode of `locate` instead. The answer comes from the step's state, so a replay, the native build and the web
build give the same numbers. Saving it and teleporting there on load puts
the actor back where the player last saw it; `located` also says whether
the actor is `held` at a held end, and a `teleport` with `hold` puts it
back held, with no push-out and no fall.

## Settings

A project sets them in `sariema.json` under `game.settings`, as nested
objects (`{"vm": {"fuel_per_step": 30000000}}`); an unknown key fails the
build with `E_SCHEMA`. `sariema api --json` lists them with the same docs.

| Key | Default | |
|---|---|---|
| `camera.damping` | 1 | Damping of that spring; 1 is critical, below overshoots. |
| `camera.dead_zone` | 0.25 | Metres a followed subject moves before the camera follows. |
| `camera.frequency` | 1.5 | Hz of the spring that carries the camera's focus. |
| `camera.look_ahead` | 0 | Seconds of the subject's velocity the focus leads by. |
| `camera.reframe_blend` | 0.5 | Seconds the camera blends when the rig in use is reframed suddenly; 0 cuts. |
| `camera.reframe_ease` | `in_out_cubic` | Easing of that blend. |
| `cutscene.advance_action` | `confirm` | Button that ends a tap dialogue line. |
| `cutscene.end_blend` | 0.6 | Seconds of the camera blend back to the game's rig when a cutscene ends. |
| `cutscene.end_ease` | `in_out_cubic` | Easing of that blend. |
| `cutscene.hinge_clip_wait` | 2 | Seconds a hinge step waits for its role to play its clip; then it ends, the leaf where it was, with a rejected E_CUTSCENE_HINGE_WAIT. |
| `cutscene.skip_action` | `skip` | Button that skips a skippable cutscene. |
| `cutscene.skip_fade` | 0.3 | Seconds of the fade a skip runs under, out and back in. |
| `cutscene.start_blend` | 0.25 | Seconds of that easing. |
| `cutscene.start_blend_max` | 0.3 | Up to this distance an actor is eased into its start place; farther, it walks there. |
| `cutscene.start_height` | 0.25 | With a collision module, metres a start may sit above or below the actor and still be measured along the ground, the floor holding the actor at its height; a start farther in height is eased or walked to in full. |
| `cutscene.start_tolerance` | 0.02 | Metres from its declared start an actor may be and still start in place. |
| `cutscene.start_walk_speed` | 1.5 | Metres per second of the walk to the start place. |
| `fx.effects` | none | The particle effects fx_start and fx_burst name: sprite, rate and puffs, life, size, speed, gravity, drag, drift, spin, colour keys, intensity and how additive; see docs/fx.md. |
| `fx.emitters` | 32 | Particle emitters alive at once, bursts and fading ones included; one more is refused with E_FX_FULL. |
| `fx.path_points` | 32 | Points an emitter's path may have; more are refused with E_FX_POINTS. |
| `fx.highlights` | 16 | Highlighted actors and nodes at once, fading ones included; one more is refused with E_FX_FULL. |
| `fx.pool` | 64 | Particles one emitter keeps alive, unless its effect gives its own pool; the ones past it are not born. |
| `fx.pulse_depth` | 0.4 | How far a pulsing highlight dims at the bottom of a pulse: 0.4 dims it to 60%. |
| `fx.trail.additive` | 1 | 1 adds the dots' light to the floor, 0 covers it, between both. |
| `fx.trail.lift` | 0.02 | Metres the dots lie above the route, so they never sink into the floor. |
| `fx.trail.sprite` | `disc` | The sprite of every trail's dots: dot, star, disc or spark. |
| `fx.trail.wave_every` | 8 | Metres between one wave's crest and the next. |
| `fx.trail.wave_gain` | 1 | How much brighter a dot is at the crest: 1 is twice as bright. |
| `fx.trail.wave_grow` | 0.25 | How much larger a dot is at the crest: 0.25 is a quarter larger. |
| `fx.trail.wave_alpha` | 0.4 | How much of its opacity a dot far from every crest loses: 0.4 leaves it at 60%. |
| `fx.trail.wave_length` | 1 | Metres from a wave's crest to where it has fallen to e^-2 of it. |
| `fx.trail_dots` | 256 | Dots one trail draws at most, counted from its goal. |
| `fx.trail_points` | 64 | Points a trail's route may have; more are refused with E_FX_POINTS. |
| `fx.trails` | 4 | Trails shown at once, fading ones included; one more is refused with E_FX_FULL. |
| `input.axes` | the input table above | Move axes: name, keys for up, down, left and right, an optional stick, whether the touch joystick drives it, and a dead zone. |
| `input.buffer_steps` | 6 | Steps a button press stays buffered for the game to consume. |
| `input.buttons` | the input table above | Buttons: name, keys and gamepad buttons. |
| `max_entities` | 65536 | Entity slots a game may use; a spawn at a higher index is rejected with E_ENTITY_LIMIT. |
| `max_steps_per_frame` | 5 | Fixed steps one display frame may run; the time beyond is dropped, so a stall slows the game instead of freezing it. |
| `max_time_scale` | 4 | The fastest a time_scale may play the world; a larger scale is lowered to it. Positive and at least min_time_scale, or loading fails with E_SETTINGS. |
| `min_time_scale` | 0.05 | The slowest a time_scale may play the world; a smaller scale is raised to it, so the world never stops. Positive, or loading fails with E_SETTINGS. |
| `motion.arrive_distance` | 0.01 | Metres from a walk target that count as arrived. |
| `motion.builtin` | true | The runtime moves actors itself; off when a collision module consumes the movement intents. |
| `motion.camera_relative` | true | The hero's move axis turns with the camera's view direction. |
| `motion.max_visible_speed` | 20 | Metres per second a visible actor may move, carried or driven by anything; a faster move is a jump. |
| `motion.min_carrier_scale` | 0.000001 | A carrier scaled below this cannot hold a visible actor the no-jump rule puts back: the actor leaves it. |
| `motion.turn_rate` | 540 | Degrees per second an actor turns unless set_turn_rate gives it a slower rate of its own; nothing turns faster, and a larger turn in one step is a jump. |
| `physics.*` | [physics.md](physics.md#settings) | The collision module's settings (`physics.controller.step_height`, `physics.controller.accel_steps`, …), with the defaults and meanings listed there. |
| `spring_rest` | 0.001 | A spring stops, exactly on its target, once its offset and velocity are both within this. |
| `vm.commands_bytes_per_step` | 4194304 | Bytes of commands one tick may submit; more stops the game with E_GAME_BUDGET. |
| `vm.fuel_first_step` | 200000000 | Budget of the first tick, which usually builds the world. |
| `vm.fuel_per_step` | 20000000 | Execution budget of one tick, about one unit per instruction; exceeding it stops the game with E_GAME_BUDGET. |
| `vm.log_bytes_per_step` | 4096 | Bytes of log text kept per tick. |
| `vm.log_lines_per_step` | 64 | Log lines kept per tick. |
| `vm.max_call_depth` | 1000 | Nested calls before the game traps. |
| `vm.max_regions` | 16 | Bulk data regions a game may register. |
| `vm.memory_bytes` | 67108864 | Ceiling of the game's memory; growing past it stops the game with E_GAME_MEMORY. |
| `vm.rays_per_step` | 256 | Rays one tick may cast; each past it is answered as over the step's budget. |
| `vm.table_elements` | 10000 | Elements a table may grow to. |

## What we took from other engines

- The fixed step with a cap on steps per frame: Godot's
  `max_physics_steps_per_frame` and Bevy's `Time<Fixed>` with `max_delta`.
- PCG32, the generator behind Godot's `RandomNumberGenerator`.
- Tweens that replace one another on the same property, from Godot's
  `Tween`; Robert Penner's easing equations.
- Springs integrated in closed form (Ryan Juckett's damped springs), so they
  are stable at any step and land exactly.
- Camera rigs with dead zones, look-ahead and eased blends from the pose on
  screen, as in Unity's Cinemachine.
- Fuel metering for a hard execution budget, from wasmi.
- A time scale on the clock that feeds the fixed step, from Bevy's
  `Time<Virtual>` relative speed.
- Drawing an entity from where it lands, instead of sliding from where it
  was, when it is placed, spawned or given another parent: Godot's
  `reset_physics_interpolation`, which Godot also applies when a node enters
  the tree, and bevy_transform_interpolation, which resets the easing of a
  transform changed outside the fixed step. Godot does not reset a node
  shown again, so it slides in; Sariema draws an actor that appears from
  where it ends.
- The ray query: Godot's `intersect_ray`, answered at once inside the
  physics step and passing through the bodies it excludes; Rapier's
  `cast_ray_and_get_normal`, a maximum distance, a filter that excludes one
  body and the nearest hit with its normal; Jolt's `CastRay`, which fills a
  result the caller owns. Sariema exchanges fixed records instead of
  Godot's dictionary, so a cast allocates nothing, and answers from the
  world as the step began instead of the world mid-step.
