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
| `sariema_batch_version` | `() -> i32` | Optional. The batch version it was built against; any other is refused at load with `E_BATCH_VERSION`. |
| `memory` | memory | Its linear memory. |

and imports only these four functions from module `sariema`:

| Import | Signature | |
|---|---|---|
| `submit` | `(ptr, len)` | Appends `len` bytes of commands. |
| `events` | `(dst, cap) -> i32` | Copies the next whole events of this step into `dst`: the bytes copied, 0 once all were read, or minus the size of an event larger than `cap`. |
| `register` | `(kind, ptr, len) -> i32` | A bulk region the engine reads in place after every tick; its id, or −1. Kind 1 is transforms. |
| `log` | `(level, ptr, len)` | A line of UTF-8 text, level 0 (debug) to 3 (error). |

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
5. The hero is driven by the move axis (when the player has control),
   actors move and turn, followers, tweens, springs and timers advance, the
   fade, cinema bars and dialogue advance.
6. World placement is resolved, the camera rig steps, trigger zones report.

Events produced in step 5 and 6 (`arrived`, `tween_done`, `trigger_enter`…)
reach the game at its next tick.

## The batch

Commands and events travel as one binary batch each way. A batch is a run of
records: a little-endian `u16` opcode, then the fields its table entry lists,
in order, with no padding. `f32` fields must be finite; `bool` is one byte,
0 or 1; `Str` and `Bytes` are a `u32` length then the bytes, `Str` being
UTF-8; an `Entity` is a slot index and a generation, `u32` each; `Vec3` is
three `f32`; enums are one byte.

A command batch that does not decode stops the game for good with
`E_BATCH_TRUNCATED`, `E_BATCH_UNKNOWN` or `E_BATCH_INVALID`, naming the
record and byte offset. The version is checked once, at load, never per
step.

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
`ui_node`. Actors are what moves, walks, turns and plays clips. A visible
actor never jumps: from one step to the next it never goes faster than
`motion.max_visible_speed` (20 m/s) nor turns faster than
`motion.turn_rate`. The guarantee is a measurement taken after every system
has run in a step: each actor's world placement is compared with the
previous step's. An actor visible when the step began and still visible, not
placed this step (spawned, a forced `teleport`, or carried by something that
was), that went too far or turned too fast is put back where it was, and
what moved it stops: the tweens, springs, follow and walk of the actor and
of everything that carries it. Whatever moved it is caught: a fast tween or
walk begun under a fade or a cut that has lifted since, or while the actor
was hidden; a carrier that turns or grows and sweeps a child round; a
transform region; several moves in one step that each pass but add up. A
step is not measured when the screen is fully faded when it begins or when
it ends, or when the camera cuts in it.

Before that, each command that moves something is judged when it is given,
with how far its next step goes, so a move that surely jumps changes
nothing: `set_transform` and `teleport` (always too far for an actor),
`tween` (its fastest step), `follow` (its first step), `move_to`,
`move_intent`, `set_hero` and `set_speed_scale` (the speed), and
`set_parent` (the change of world placement). It is refused with
`E_TELEPORT_VISIBLE` when that would carry an actor too far in a step: the
entity itself, a child at any depth (going as far) or a follower (going its
lag's share), that was visible when the step began and is not placed yet
this step. It is allowed when the screen is fully faded, the camera cuts in
the same step, or `teleport` says `force`; once the cover lifts, the move is
measured like any other. A forced `teleport` carries the entity's children
with it, drawn where they land. Hiding and showing in the same step hides
nothing: an actor hidden for a whole step may then be placed anywhere.

A refused command produces a `rejected` event with the opcode, the code and
the entity; a move the step's measurement stopped reports opcode 0. The game
keeps running.

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
  `ctx.despawn`, `ctx.entities().is_alive(e)`.
- Cutscene control: `ctx.bind(role, entity)`, `ctx.play_cutscene`,
  `ctx.skip_cutscene`, `ctx.signal`.
- `ctx.rng()` is a PCG32 seeded from the engine's `started` event.
- `Machine<S>` holds a state, the step it was entered and an `entered()`
  flag, for logic written as states.
- `Transforms` is a bulk region of kind 1: entity placements applied after
  every tick without one command each. Its buffer is sized once; register it
  once with `ctx.register(Transforms::KIND, region.as_bytes())`. Actors are
  skipped; they move by commands.
- `testing::TestHost` runs a game natively in `cargo test`: queue events,
  `Runner::run` a tick, decode the commands with `testing::commands`.
- `report_panic` sends a panic's message to the engine's log; call it from a
  std game's panic hook. A `no_std` game enables the `panic-handler` feature.

The logic crate is a `cdylib` with its own `[workspace]`, built with
`cargo build --release --target wasm32-unknown-unknown`. `sariema build`
does that, with the project's `game.toolchain` when it names one.

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
0.026 ms; a steady step allocates nothing on the host. The target of at most
2 ms for game logic on the floor iPhone is not yet confirmed: no device run
has been made.

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
on. `control` hands the hero to the player, the script or a cutscene.

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
replays it and prints the events, host messages, log, saves, state and
world hashes and timing.

## The engine's systems

- **Movement**: `move_to` walks at a speed and ends with `arrived` (exactly
  on the target) or `blocked`; `move_intent` holds a velocity; `face` turns
  at `motion.turn_rate`. With `motion.builtin` the runtime moves actors
  itself; a collision module takes the same intents.
- **Tweens and springs** on position, yaw, pitch, roll, scale or a world
  variable, with 20 easings (Penner's, plus smoothstep). A new animation of a
  property starts from its current value, and a spring keeps its velocity, so
  interrupting never jumps. A spring settles exactly on its target.
- **Followers** keep an entity at another's position plus an offset,
  closing the gap with a half-life.
- **Timers** fire after a whole number of steps, once or repeating.
- **Zones**: spheres reporting actors entering and leaving.
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
- **Time scale**: `time_scale` plays the world slower or faster than real
  time, reached over `blend` seconds. The display frames' time is scaled
  before it becomes steps: a step stays 1/60 s of world time, so play stays
  deterministic, and everything that follows the steps (movement, tweens,
  clips, waits) slows together. Leftover frame time carries into the next
  step. The scale stays within `min_time_scale`, so the world never stops,
  and `max_time_scale`.
- **Screen**: `fade` to a colour, `letterbox`, and dialogue lines (`say`):
  an `auto` line ends after its duration, a `tap` line when the game sends
  `advance_line` (in a cutscene, on the `cutscene.advance_action` button);
  `line_done` reports the end.

## Saves and the host

`save` asks the host to persist a value under a key; `save_load` asks for
every saved key, answered by `save_loaded` events and one `save_load_done`.
`host_ready`, `host_exit`, `host_orientation` and `host_send` are messages to
the host; `host_message`, `orientation_changed` and `exit_requested` come
back as events. `sariema run --headless --saves saves.json` starts with a
save store, and prints the store at the end.

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
| `cutscene.skip_action` | `skip` | Button that skips a skippable cutscene. |
| `cutscene.skip_fade` | 0.3 | Seconds of the fade a skip runs under, out and back in. |
| `cutscene.start_blend` | 0.25 | Seconds of that easing. |
| `cutscene.start_blend_max` | 0.3 | Up to this distance an actor is eased into its start place; farther, it walks there. |
| `cutscene.start_tolerance` | 0.02 | Metres from its declared start an actor may be and still start in place. |
| `cutscene.start_walk_speed` | 1.5 | Metres per second of the walk to the start place. |
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
| `motion.turn_rate` | 540 | Degrees per second an actor turns; nothing turns faster, and a larger turn in one step is a jump. |
| `spring_rest` | 0.001 | A spring stops, exactly on its target, once its offset and velocity are both within this. |
| `vm.commands_bytes_per_step` | 4194304 | Bytes of commands one tick may submit; more stops the game with E_GAME_BUDGET. |
| `vm.fuel_first_step` | 200000000 | Budget of the first tick, which usually builds the world. |
| `vm.fuel_per_step` | 20000000 | Execution budget of one tick, about one unit per instruction; exceeding it stops the game with E_GAME_BUDGET. |
| `vm.log_bytes_per_step` | 4096 | Bytes of log text kept per tick. |
| `vm.log_lines_per_step` | 64 | Log lines kept per tick. |
| `vm.max_call_depth` | 1000 | Nested calls before the game traps. |
| `vm.max_regions` | 16 | Bulk data regions a game may register. |
| `vm.memory_bytes` | 67108864 | Ceiling of the game's memory; growing past it stops the game with E_GAME_MEMORY. |
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
