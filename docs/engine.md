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
`engine.draw_hud(alpha)`; `crates/sariema-view` does, for the window
player, `run --headless --screenshot` and the web host alike.

`Engine::load` takes a game pack and the assets it carries (room, collision,
actors, sound bank, HUD; [game-api.md](game-api.md) has the pack). With a
collision pack the runtime's built-in mover is switched off
(`motion.builtin = false`) and the collision module moves every actor;
`Engine::new`, given a simulation, refuses collision with the mover still
on (`E_SETTINGS`), since every actor would move twice, and physics
settings out of range (`E_SETTINGS`, naming the key), however the host
built them.

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
| 8 | Sound | the step's sound and music requests, the camera | mixer commands for sounds already decoded; the rest left to the feed |
| 9 | HUD | the game's `ui_*` commands, the screen state, the dialogue, who holds control | the screen for the next frames |
| 10 | Host | the step's messages to the host | the host queue; a load no host will answer, answered as no save |
| 11 | Rejections | what the modules refused | `rejected` events for the game's next step |

1. **Input.** `Engine::step(actions)` takes them as they are (a replay);
   `Engine::frame(dt, raw)` maps the raw keys, pad and sticks through the
   game's input settings for every step the frame owes. Before the step the
   HUD's screen runs its own step: its joystick held (the first named one
   in the description's order, when it has several) gives the touch stick,
   mapped through the same settings, and replaces an axis it moves; a button
   node named after an input button holds that button down, ORed with the
   keys.
2. **Game logic** ticks with the events of the step before and its commands
   are applied before anything moves.
3. **The sequencer** runs the cutscene's steps. A `walk` becomes a
   `move_to` intent; skipped, it becomes a placement plus a `SkippedWalk`
   intent, so the collision module can check the place is reachable. Then the roots of clip paths (`clip_path`, a cutscene's `path`) are
   placed for their clips' times; the collision module poses them with the
   collider off and gets it back on where the last span lands.
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
   too; once it lands the runtime's walk is dropped with it, so a
   cutscene's `walk` ends, while a teleport the module refuses keeps it. A `SkippedWalk` whose place has no path from where the role stood
   puts it back there and ends the walk `blocked`.
6. **The runtime** finishes its step: followers, tweens, timers, screen,
   dialogue, `World::resolve`, then the no-jump measurement (anything that
   moved farther than the visible speed allows is put back unless a cover
   hid it), the camera and the zones.
7. **After the step** the collision module takes the measurement's
   teleports and stops, so its actors stand where the runtime put them back.
8. **Sound.** A sound request names its sound by id; the bus is the
   bank's. A sound with an emitter is heard from the camera: full gain
   within `reference_distance`, then the inverse-distance model of OpenAL
   and Web Audio, panned along the camera's right, and followed while it
   plays. A sound already decoded plays at once; a sound still to decode
   (marked `on_demand`, first play) or to open as a stream, and every music
   request, keeps its place in a queue of `audio.pending` for the audio
   feed. Headless, commands are stamped with the step's mixer frame; with a
   device the host takes the mixer and commands play at once.
9. **HUD.** The game's `ui_visible`, `ui_text`, `ui_value` and `ui_play`
   reach the screen. While a cutscene holds control the game's HUD steps
   aside: each node of the HUD's root is hidden and shown again, as the
   game last asked, once control has returned and the cinema bars the
   cutscene left are gone, so no bar covers it. A node in the group
   `cutscene` (`"groups": ["cutscene"]`) stays. Over the game's own nodes the
   engine draws its layer: the top cinema bar, the bottom one with the
   dialogue line (when the HUD has a font) riding `line_margin` above it,
   and the fade. A `say` line or `ui_text` longer than `hud.max_text_bytes`
   is refused with `E_UI_TEXT_TOO_LONG`, and one whose glyphs no longer fit
   the HUD's glyph atlas with `E_UI_ATLAS_FULL`: a refused line is hidden,
   though the runtime keeps it until it ends; a `ui_text` refused for its
   length leaves the node's previous text, and one refused for the atlas
   leaves the node empty. A cutscene's line that breaks either fails the load with
   that code.
10. **Host.** The step's messages to the host join the host queue
    ([below](#messages-to-the-host)); a `save_load` no host will answer is
    answered as no save.
11. **Rejections** from the collision module, the mixer, the screen and the
    host queue are reported to the game as `rejected` events the next step;
    `REJECTED` lists their codes.

## The audio feed

A decoder allocates as it reads, so the step never decodes.
`Engine::feed_audio()`, called by the host after its steps and before it
draws (`Engine::frame` calls it), plays what the steps queued (decoding an
on-demand sound, opening a stream), decodes the streams ahead and, when no
device took the mixer, mixes every step since the last feed and folds the
samples into `audio_hash`. Run after every step, as `run --headless` does,
the mixed sound is the same every run: the commands carry the step's mixer
frame, and the feed mixes that step after starting them. With
`audio.stream_thread` the streams decode on a thread of their own instead;
with `audio.predecode_streams` they are decoded whole when the bank loads
and neither the step nor the feed allocates. Refusals from the feed reach
the game at the next step, as the step's do.

## Messages to the host

The game's `host_ready`, `host_exit`, `host_orientation`, `host_send`,
`save` and `save_load`, and its stop, wait in a queue the host empties with
`Engine::take_host()` after its steps, every frame; each comes with the
step that sent it. The queue holds `host.max_messages` messages and
`host.max_bytes` bytes of keys, values, topics and payloads: a message past
either is refused with `rejected` `E_HOST_QUEUE_FULL`, so a host that never
empties it holds a bounded amount. The game's stop always gets in.

Every `save_load` is answered, so a game may wait for `save_load_done`:

- A host that keeps saves calls `Engine::answers_saves(true)` and answers
  each `load_saves` it takes with `Engine::answer_saves`, at once or frames
  later, as a WebView app's store does after a round trip. A load it leaves
  unanswered for `host.save_load_timeout_s` (5 s of game time) is answered
  as no save and the engine logs a warning (the `log` crate); an answer
  after that is dropped (`answer_saves` returns false).
- A host without a save handler, as the engine starts, has each load
  answered as no save at the step it was asked.

| Host | Saves | `host_exit` | The other messages |
|---|---|---|---|
| `run --headless --game` | the `--saves` store, answered after the step; the store is printed at the end | listed | listed under `host` |
| `run --game`, the window | `<pack name>.json` in `--save-dir` (default: `saves` beside the pack), the format of `--saves`, written when a frame's steps saved | closes the window | let go |
| `bench` | none: each load is answered as no save | let go | let go |
| `@sariema/web` | the page's: `on("host")` gives every message, `on("saveLoad")` with `answerSaves` answers loads ([web.md](web.md#host-messages-and-saves)) | the page's | the page's |

## Allocations

A step that only animates allocates nothing. The cinema bars are laid out
once at their full height and scaled from their outer edges to the
letterbox, the dialogue line moved with them by an offset; fades, tweens
and springs write transforms and colours; a named animation plays from
its tracks where they are kept. A step that changes content (a line
shown, a text set, a node resized) lays the HUD out again inside taffy,
which allocates as it lays out. So at load the engine shapes every line
the pack's cutscenes hold, rasterising their glyphs and making room for
the longest in the line's node, and lays the HUD out; a resize lays it out
at once rather than in the next step.

The climb, replayed over its input log and as long again with no input,
allocates in 18 of its 1,200 steps, 97 times, all before step 600; every
host makes the same counts at the same steps
(`crates/sariema-cli/tests/climb-allocations.json`, checked natively by
`cargo test -p sariema-cli --test allocations` and in every browser by
the web package's tests):

| Steps | Allocations | Where |
|---|---|---|
| 0 | 42 | the hero spawns: its body's walkable grid (the collision module builds it for an actor, so not before one exists), the world's and the logic's first buffers |
| 186 | 12 | the cutscene starts: its run state and the hero's planned walk |
| 341 | 20 | the cutscene's line shows: taffy lays the HUD out (15), the runtime keeps the line and the block's state (5) |
| 214, 244, 437, 461 | 1 each | a cutscene block starts: its boxed state |
| 20, 30, 31, 50, 65, 66, 98, 185, 213, 245, 267 | 19 | a buffer of the runtime, the collision world or the logic's host grows to a size it had not reached |

The cutscene's second walk arrives one step later than it used to (244, not
243): the hero no longer carries the stick the player held into the
cutscene's trigger, which took it a step further at each arrival, so every
step after it moved by one.

The audio feed runs between steps. The music's stream decoder (symphonia)
allocates for every packet it reads: a frame's bytes and its bit
reservoir, which its API does not let a caller lend, about 45 a second for
the climb's 44.1 kHz MP3. Opening the stream and decoding a sound loaded
on demand add a burst once (660 in the climb's first second, 151 in its
sixth); the resampler's kernel and output are sized before they are
filled. `audio.predecode_streams` decodes the streams at load instead,
after which neither the steps nor the feed allocate.

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
between steps: the bars, and the line riding on them, are drawn at the
step's height until the next step, and the fade at the larger of this
step's value and the step before's. The no-jump rule lets a step move anything at once when
the screen was fully faded at the step's start or at its end; the frames up
to the next step show that step's moves, so they draw the fade at its
fuller value. A teleport under a cover fade is therefore never seen.
`tests/fade.rs` pins it.

## Settings

Every number the glue uses is a field of `EngineSettings`, with a default;
each module's own settings stay in its crate. The collision module's are
the game's, `physics` in its runtime settings (`game.settings.physics` in
the project, [physics.md](physics.md)): the engine gives them to every
room's world with the runtime's step and its turns capped at
`motion.turn_rate`.

| Field | Default | What |
|-------|---------|------|
| `anim` | `AnimSettings::default()` | the animator's |
| `knockback_s` | 0.25 | seconds a `knockback` pushes for |
| `contact_blend` | 0.25 | fraction of a traversal over which the feet ease onto and off a contact |
| `audio.reference_distance` | 2.0 m | full gain within it |
| `audio.rolloff` | 1.0 | inverse-distance rolloff |
| `audio.emitters` | 32 | sounds followed at once |
| `audio.min_change` | 0.001 | smallest gain or pan change sent |
| `audio.stream_thread` | false | decode streams on a thread; for a host playing to a device |
| `audio.predecode_streams` | false | decode streams whole at load: no allocation after it, 23 MB a minute at 48 kHz |
| `audio.pending` | 16 | requests a step can leave to the feed; one more is `E_AUDIO_QUEUE_FULL` |
| `mixer` | `MixerSettings::default()` | the mixer's; a host playing to a device sets `sample_rate` |
| `hud.bar_fraction` | 0.12 | cinema bar height at `letterbox` 1 |
| `hud.line_size`, `hud.line_margin` | 22, 48 | the dialogue line's text, and its space above the bottom bar, in UI pixels |
| `hud.line_inset` | 0.12 | the line's space either side, a fraction of the screen's width |
| `hud.line_padding`, `hud.line_radius` | [10, 18], 12 | the line's box, in UI pixels |
| `hud.line_fill`, `hud.line_color` | black at 0.7, white | the line's box and text |
| `hud.hide_in_cutscenes` | true | the game's HUD steps aside while a cutscene holds control and its bars are up |
| `hud.max_text_bytes` | 4096 | bytes one `say` line or `ui_text` may hold, since the step shapes it; a longer one is `E_UI_TEXT_TOO_LONG` |
| `host.max_messages` | 256 | messages the host queue keeps until the host takes them; one more is `E_HOST_QUEUE_FULL` |
| `host.max_bytes` | 4 MiB | bytes of keys, values, topics and payloads it keeps, the most one tick can send; past it is `E_HOST_QUEUE_FULL` |
| `host.save_load_timeout_s` | 5 | seconds of game time a `save_load` waits for a host that answers saves before it is answered as no save |
| `hud.width`, `hud.height` | 1280, 720 | the screen until the host gives one |

## Determinism

The step is a pure function of the pack, the seed and the input: fixed
1/60 s, seeded randomness, no wall clock, no hash-map order, and every
transcendental through `libm` (glam with its `libm` feature, the runtime's
own trigonometry too), so `engine_hash` is the same in native debug, native
release and wasm32. The web package's browser tests (`npm test` in
`packages/sariema-web`) replay the climb in Chromium and WebKit and reach
the hashes `sariema run --headless --game` prints. `audio_hash` covers the mixed samples and is
repeatable with one build, not across builds: the mixer's arithmetic is left
to the compiler.

A change to the simulation changes the hashes, and replays recorded before
it need recording again. The latest: a walk's end and a hero taken from
the player now drop the stick, a walk is blocked by net progress over a
window, and a hidden actor is in no one's way (`docs/physics.md`, scripted
moves and `SetHidden`; `docs/game-api.md`, `control` and movement).

## Hosts

- `sariema run --headless --game pack.srmg --input log.json --steps N`:
  the steps, the events and the hashes; with `--screenshot` (and
  `--filmstrip N`) the frames the window would show at the steps it keeps,
  drawn after those steps without changing any hash.
- `sariema run --game pack.srmg [--save-dir DIR]`: the window. Keys go
  through the game's input settings, the mouse is the HUD's pointer, and
  the sound plays on the default output device; with none
  (`E_AUDIO_DEVICE`) the game runs silent and the report says why. Saves
  go to `DIR/<pack name>.json` (`saves` beside the pack by default), which
  the report names; `host_exit` closes the window.
- `sariema bench pack.srmg [--input log.json]`: each timed frame is one step
  and its frame; it reports the step, the HUD and the HUD pass's passes,
  draws and uploads apart. The game's messages to the host are let go.
- `@sariema/web`'s `loadGame` (`crates/sariema-web`): a page plays a game
  pack, keys through the input settings and pointers (touch included) to
  the HUD, the sound through an AudioWorklet with the mixer taken;
  `replayGame` steps with an input log as `run --headless` does.
  [web.md](web.md) has it.

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
