# The engine: one step, every module

`crates/sariema-engine` makes one engine of the lanes: the game runtime
(`sariema-runtime`) stepped at a fixed 1/60 s with the collision module
(`sariema-physics`), the animator (`sariema-anim`), the mixer
(`sariema-audio`) and the HUD (`sariema-ui`) wired in. Nothing on the step
path needs a thread, the wall clock or a GPU, so the same crate runs the
headless replay, the window player and the web build. A host feeds it input
and draws what it gives at the frame's `alpha`.

```rust
let mut engine = Engine::load(pack, seed, EngineSettings::default())?;
// headless: one step per recorded set of actions
engine.step(replay.actions(step));
// window: the steps a display frame owes, then a frame drawn at alpha
engine.frame(dt, &raw_input);
let alpha = engine.alpha();
```

The host then draws `engine.camera(alpha)`, `engine.actors(alpha)` and
`engine.draw_hud(alpha)`; `crates/sariema-cli/src/game_view.rs` is one.

`Engine::load` takes a game pack and the assets it carries (room, collision,
actors, sound bank, HUD; [game-api.md](game-api.md) has the pack). With a
collision pack the runtime's built-in mover is switched off
(`motion.builtin = false`) and the collision module moves every actor.

## The order of a step

| # | Stage | Reads | Gives |
|---|-------|-------|-------|
| 1 | Input | the host's raw input, or an input log's actions; the HUD's joystick and buttons | the step's `Actions` |
| 2 | Game logic | last step's events, `Actions` | commands, applied at once |
| 3 | Cutscene sequencer | the playing cutscene | walks, clips, shots, sounds, lines, fades |
| 4 | Animation | the runtime's clip requests and clip times | poses, `clip_marker`, `clip_finished` |
| 5 | Collision | the runtime's physics intents, the poses | positions written back, `arrived`, `blocked`, `hit_wall`, triggers |
| 6 | Runtime | followers, tweens, timers, screen, dialogue | resolved world, no-jump measurement, camera, zones |
| 7 | Collision, after | the measurement's put-backs | the actor moved back before the next step |
| 8 | Sound | the step's sound and music requests, the camera | mixer commands; offline, the step's samples |
| 9 | HUD | the game's `ui_*` commands, the screen state, the dialogue | the screen for the next frames |
| 10 | Rejections | what the modules refused | `rejected` events for the game's next step |

1. **Input.** `Engine::step(actions)` takes them as they are (a replay);
   `Engine::frame(dt, raw)` maps the raw keys, pad and sticks through the
   game's input settings for every step the frame owes. Before the step the
   HUD's screen runs its own step: a joystick held gives the touch stick,
   mapped through the same settings, and replaces an axis it moves; a button
   node named after an input button holds that button down, ORed with the
   keys.
2. **Game logic** ticks with the events of the step before and its commands
   are applied before anything moves.
3. **The sequencer** runs the cutscene's steps. A `walk` becomes a
   `move_to` intent; skipped, it becomes a placement plus a `SkippedWalk`
   intent, so the collision module can check the place is reachable.
4. **Animation** runs where the runtime's built-in mover would, before the
   collision module, because a traversal's contact needs this step's pose
   (Godot runs an `AnimationTree` on the physics callback for the same
   reason). Each animated entity's animator is set to the runtime's clip and
   time: the runtime owns the clock, the animator follows it, so a clip seeks
   instead of drifting. Markers become `clip_marker`, a clip that ends
   `clip_finished`. A rig anchor and named contacts are read from the pose.
5. **Collision** keeps one collision actor per live actor entity, pools the
   ones of despawned entities, and per step: notes teleports, poses actors
   the runtime moved itself (a tween, a follow) collider off, applies the
   intents in the order given, drives the hero from the player's velocity,
   advances traversals (by arc length, the actor's feet eased onto each
   contact the pose names; a contact its pose lacks falls back to the feet),
   steps the world and writes positions and yaws back. A teleport drops the
   moves given before it and keeps those given after it, in the same step
   too. A `SkippedWalk` whose place has no path from where the role stood
   puts it back there and ends the walk `blocked`.
6. **The runtime** finishes its step: followers, tweens, timers, screen,
   dialogue, `World::resolve`, then the no-jump measurement (anything that
   moved farther than the visible speed allows is put back unless a cover
   hid it), the camera and the zones.
7. **After the step** the collision module takes the measurement's
   teleports and stops, so its actors stand where the runtime put them back.
8. **Sound.** Each sound and music request becomes a mixer command. A
   request names its sound by id; the bus is the bank's. A sound marked
   `on_demand` is decoded on its first play. A sound with an emitter is
   heard from the camera: full gain within `reference_distance`, then the
   inverse-distance model of OpenAL and Web Audio, panned along the camera's
   right, and followed while it plays. Headless, commands are stamped with
   the step's mixer frame and the step's samples are mixed and hashed; with
   a device the host takes the mixer, commands play at once, and streamed
   sounds decode on a thread of their own (`audio.stream_thread`), since the
   decoder allocates as it reads.
9. **HUD.** The game's `ui_visible`, `ui_text`, `ui_value` and `ui_play`
   reach the screen. Over the game's own nodes the engine draws its layer:
   the dialogue line (when the HUD has a font), the cinema bars and the fade.
10. **Rejections** from the collision module, the mixer and the screen are
    reported to the game as `rejected` events the next step; `REJECTED`
    lists their codes.

## A frame

`Engine::frame(dt, raw)` runs the steps a display frame owes (the runtime's
clock caps a long frame). The host then draws at `alpha = engine.alpha()`,
the fraction of the next step already elapsed:

- `camera(alpha)`: the runtime's camera pose, interpolated; `None` until the
  game chooses one, and the host shows the room's own.
- `actors(alpha)`: one `ActorView` per animated entity, its transform
  interpolated between the last two steps and its palette sampled at the
  runtime's `clip_time_at(alpha)`, so the pose and the position never come
  from different moments.
- `draw_hud(alpha)`: the HUD's draw list, animations sampled at `alpha`.

The renderer draws the room and the actors, then the HUD in its own pass
over the frame (`Renderer::render_ui`); `FrameStats` counts that pass's
draws and uploads with the frame's.

## The fade contract

The fade and the cinema bars are *set* on the HUD each step, never animated
between steps, and the fade is drawn at the larger of this step's value and
the step before's. The no-jump rule lets a step move anything at once when
the screen was fully faded at the step's start or at its end; the frames up
to the next step show that step's moves, so they draw the fade at its
fuller value. A teleport under a cover fade is therefore never seen.
`tests/fade.rs` pins it.

## Settings

Every number the glue uses is a field of `EngineSettings`, with a default;
each module's own settings stay in its crate.

| Field | Default | What |
|-------|---------|------|
| `physics` | `sariema_physics::Settings::default()` | the collision module's; its step is set to the runtime's, its turn speed capped at `motion.turn_rate` |
| `anim` | `AnimSettings::default()` | the animator's |
| `knockback_s` | 0.25 | seconds a `knockback` pushes for |
| `contact_blend` | 0.25 | fraction of a traversal over which the feet ease onto and off a contact |
| `audio.reference_distance` | 2.0 m | full gain within it |
| `audio.rolloff` | 1.0 | inverse-distance rolloff |
| `audio.emitters` | 32 | sounds followed at once |
| `audio.min_change` | 0.001 | smallest gain or pan change sent |
| `audio.stream_thread` | false | decode streams on a thread; for a host playing to a device |
| `mixer` | `MixerSettings::default()` | the mixer's; a host playing to a device sets `sample_rate` |
| `hud.bar_fraction` | 0.12 | cinema bar height at `letterbox` 1 |
| `hud.line_size`, `hud.line_margin` | 22, 48 | the dialogue line, in UI pixels |
| `hud.width`, `hud.height` | 1280, 720 | the screen until the host gives one |

## Determinism

The step is a pure function of the pack, the seed and the input: fixed
1/60 s, seeded randomness, no wall clock, no hash-map order, and every
transcendental through `libm` (glam with its `libm` feature, the runtime's
own trigonometry too), so `engine_hash` is the same in native debug, native
release and wasm32. `tools/wasm-replay` runs a pack and an input log in
Node's WebAssembly and prints the same hashes as
`sariema run --headless --game`. `audio_hash` covers the mixed samples and is
repeatable with one build, not across builds: the mixer's arithmetic is left
to the compiler.

## Hosts

- `sariema run --headless --game pack.srmg --input log.json --steps N`:
  the steps, the events and the hashes; with `--screenshot` (and
  `--filmstrip N`) the frames the window would show at the steps it keeps,
  drawn after those steps without changing any hash.
- `sariema run --game pack.srmg`: the window. Keys go through the game's
  input settings, the mouse is the HUD's pointer, and the sound plays on
  the default output device; with none (`E_AUDIO_DEVICE`) the game runs
  silent and the report says why.
- `sariema bench pack.srmg [--input log.json]`: each timed frame is one step
  and its frame; it reports the step, the HUD and the HUD pass's passes,
  draws and uploads apart.
- The web lane builds this crate for wasm32 and drives it the same way.

## What was taken from where

- The fixed step with the frame drawn at `alpha` between the last two steps:
  Glenn Fiedler's *Fix Your Timestep!*, as Bevy's `Time<Fixed>` overstep
  fraction and Godot 4's physics interpolation do.
- Animation before collision inside the fixed step: Godot's `AnimationTree`
  on the physics process callback, which root motion and contacts need.
- The HUD as a pass of its own after the frame: Godot's `CanvasLayer` over
  the 3D viewport, Bevy's UI pass after the main pass.
- Emitters heard from the camera with the inverse-distance model: OpenAL's
  `AL_INVERSE_DISTANCE_CLAMPED` and Web Audio's `PannerNode` `"inverse"`.
- Streams decoded off the audio and step threads: Godot's and FMOD's stream
  players, which read and decode ahead on a thread of their own.
