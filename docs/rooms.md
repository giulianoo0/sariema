# Rooms: scenery and switching without a seam

A game pack holds one or more rooms. Each room is a room pack (what is
drawn) and, when the game collides, a collision pack (what blocks). This
page covers what a room can change while it plays (groups, pieces, nodes,
hinges, scene parameters) and how a game goes from one room to the next
with no black frame: neighbours loaded in the background, decoded and
uploaded a share at a time, collision worlds built off the step, and three
ways to show the switch.

## Scenery inside a room

`sariema build` turns the montage of a room into:

- **Groups.** Each sub-room in `show` is a group of the draws and lights it
  places. `groups_off` lists sub-rooms that start switched off: built,
  uploaded, and drawn only once the game switches them on. A group off draws
  nothing and lights nothing, as if the pack had been built without it, and
  costs no pipeline or buffer to switch back on.
- **Pieces and states.** A kit piece with states (`estados` in the kit,
  `estado` on the montage placement) keeps every state in the pack; state 0
  is the look the placement had before any state. Switching a state shows
  only that state's draws.
- **Nodes.** Every loose piece is a node pivoted at its placement; a node
  can be moved, hidden or bound to an entity. A kit node marked `giro`
  (`<piece>_<hinge>`) is split out of its piece as a hinged node, stored
  turned by its montage angle, with its axis and pivot in the pack.
- **Scene parameters.** Each light's colour, intensity and range, the sun,
  the ambient light, the background and the look are named parameters of
  the room, read and tweened by name.

What the game changed is kept for the whole run, per room: a room left and
entered again is as the game left it. The state hash covers only the
difference from the room as loaded.

Names are name ids (`id("hall")` in the SDK); `sariema api` lists every
command, enum and error.

| Command | Does |
|---|---|
| `room_group { group, on }` | Switches a group of the active room. |
| `piece_state { piece, state }` | Shows a state of a piece of the active room. |
| `tween { property: scene, var: <parameter>, to, duration, ease }` | Tweens a scene parameter. |
| `spawn { kind: room_node, asset: <node> }` | Binds a node to an entity: its transform and visibility follow the entity. |
| `spawn { kind: light, asset: <light> }` | Binds a light to an entity: it follows the entity's position. |

Unknown names are refused with `E_SCENE_UNKNOWN`; an unknown room with
`E_ROOM_UNKNOWN`.

## Rooms of a game

The project lists its rooms under `rooms` and the game's room graph under
`game.rooms` (`schemas/project.schema.json`); `game.room` is the first one.

```json
"game": {
  "room": "porch",
  "rooms": {
    "porch": { "beside": ["hall"], "ambience": "wind" },
    "hall": { "beside": ["porch"], "neighbours": ["attic"] },
    "attic": { "neighbours": ["hall"], "ambience": "wind" }
  }
}
```

- **`neighbours`**: rooms the game may switch to next. They are loaded in
  the background while this room is active.
- **`beside`**: rooms whose geometry continues this one (two rooms of one
  house). They are loaded and drawn with it all along, whole and lit by
  their own lights, so walking from one into the other shows no seam.
- **`ambience`**: a looped sound of the bank that plays while the room is
  active and crossfades into the next room's at a switch.

Either every room has a collision pack or none has.

| Command or event | Does |
|---|---|
| `room_preload { room }` | Loads a room in the background, beyond the neighbours. |
| `room_switch { room, transition, duration }` | Makes `room` active. |
| `room_switched { room }` | Event: the active room changed, by the game or a cutscene. |

A switch keeps everything that is not the room: actors keep their
positions, speed and scripted moves; the camera keeps its rig; sound plays
on. The transition says how it is shown:

- **`Cut`**: the next frame shows the new room. Inside a cutscene, under a
  fade the cutscene asks for, this is the switch.
- **`Continuous`**: both rooms are drawn whole for `duration` while their
  light, sky, sun, ambient light and look blend from one to the other. For
  rooms that occupy one space, usually `beside` each other.
- **`Dissolve`**: a dissolve in screen space. The last frame shown of the
  room left, as its own camera and light saw it, fades out over the next
  room, which is drawn alone by its own camera and light from the switch's
  frame on. It lasts `duration`, or `ViewSettings::dissolve_s` when the
  switch gives none, along `dissolve_ease`. The room left is never drawn
  through the next room's camera. For places that do not continue each
  other, and for a jump in the story's time (night to day): the light of
  one room never rises into the other's.

## Loading without a seam

Each frame the view keeps the rooms the engine wants
(`Engine::wanted_rooms`: the active room, the one a switch is leaving, the
ones beside it, the preloaded ones newest first, the neighbours, then the
rooms beside the neighbours), at most `ViewSettings::max_resident`, and
frees the others. The first room and the rooms beside it are loaded whole
before the first frame, so a room beside never appears after the game has
started.

A room goes through four stages, none of which takes a frame's whole
budget:

1. **Decode.** On a thread, or, where there are none (the web),
   `RoomDecoder` reads `decode_bytes_per_frame` bytes of whole parts a frame
   (a mesh, a texture, the tail). It gives the room `decode` would, and the
   same errors.
2. **Upload.** `Renderer::begin_room` creates the room's pipelines and
   buffers; `upload_room` writes `upload_bytes_per_frame` bytes of meshes
   and texture mips a frame into a slot not drawn yet.
3. **Collision world.** A `WorldJob` builds the room's world and the
   walkable grid of each body: whole on a thread, or, where there are none,
   a share at a time until `world_ms_per_frame` is spent. A share adds
   colliders (`world_units_per_share` units, a collider costing its
   triangles or hull points, a primitive one), or folds, sorts and writes
   that many items of the tree over their parts, or builds
   `grid_columns_per_share` columns of a grid, or joins as many columns'
   floors into islands. Each step gives what building whole gives: the
   same tree, the same grid, the same islands. The frame stops before a
   share that would end past the budget, expecting it to take as long as
   the longest share of the frame or of the last frame (counted at half
   the budget at most, so a share the system paused does not slow the
   next frame); a frame does at least one share.
4. **Ambience.** The next room's ambience stream is opened and its first
   second decoded in an audio feed with nothing else to open, never during
   a switch (`Audio::prepare`), so the switch only starts a voice.

A switch to a room not loaded yet loads it, and the rooms beside it, at
once in that frame (`RoomsFrame::loaded_at_once` reports it). A switch to a room whose world is
not built builds the world in the step, without its grids: they are built
after the step, a share a frame (`Engine::grow_grids`), unless an actor's
path search needs one first, which builds it whole. The world is the same
either way, so the run is too.

`ViewSettings` (defaults): `max_resident` 3, `decode_bytes_per_frame` 4 MiB,
`upload_bytes_per_frame` 4 MiB, `threads` true, `world_ms_per_frame` 2,
`grid_columns_per_share` 8, `world_units_per_share` 2048, `shadows` true,
`dissolve_s` 0.5,
`dissolve_ease` `in_out_sine`. `SoundSettings::ambience_fade_s` (0.5) is the
crossfade between ambiences when the switch has no duration.

## A dissolve in screen space

The renderer keeps the frame drawn last (`Renderer::hold_frame`): before
anything of the switch's frame is drawn, the last frame's scene, bloom and
ambient occlusion are still in the renderer's own targets, and they are
composited again, with the uniform of the composite that drew them, into a
texture the size of the target. The HUD is drawn after the composite, so
it is not in the frame held. Each frame of the dissolve then draws that
texture over its own composite in one blended full-screen pass
(`Renderer::set_held_frame`); at opacity 0 the pass is skipped. The blend
mixes the colour as the target stores it: linear on an sRGB target,
encoded on the others.

What it costs: one full-screen composite on the switch's frame, one
blended full-screen pass a frame while the dissolve lasts, and a texture
the size of the target, created on the first dissolve and kept until the
target is resized.

The view follows `Engine::switched`, the last switch kept after its
transition is over, so a frame that runs several steps (a frame late on
the web) still shows a dissolve from its start, even one that lasts a
single step because it gave no duration.

## Numbers

`sariema bench <game.srmg> --warmup 0 --runs 1 --frames N --json` reports,
under `rooms`, the frame CPU time of steady frames, of frames that load
(decode, upload, build a world or a grid), of frames while a switch lasts,
and of each switch's own frame, with the bytes and worlds involved.
`--no-threads` loads as the web does. The bench runs frames as fast as it
can, not at 60 Hz, so a thread may not be done by a switch that would find
its work done in play; `--no-threads` does not depend on that.
`--timeline` adds every frame under `rooms.timeline`: the steps run, the
frame's CPU, the step, the audio feed, decoding, uploading and collision
work inside it, and the CPU time of the frame's thread (macOS and Linux),
in ms; `rooms.stage_max_ms` is the most a frame spent on each loading
stage. A frame whose CPU is well above its thread's time was paused by the
system: on a shared machine, read the thread's time.

Apple M5, release, 1920×1080, medium tier:

| Game | Loading | Frame CPU, ms: steady p50 · loading p99 · switch's own frame | Before this page's fixes, switch's frame |
|---|---|---|---|
| `examples/rooms` (3 rooms, 280 KB) | threads | 1.3 · 2.2–5.9 · 1.2–4.3 | 4.4–17 |
| `examples/rooms` | `--no-threads` | 1.2 · 3.2–3.5 · 0.6–3.2 | — |
| a house, 3 rooms of 13–20 MB, 124–236 colliders, a dissolve and a cut 16 s apart | `--no-threads` | 1.7–1.9 · 10–13 · 2.6–12.5 with the world in, 15–34 with only the world built in the step | 1,100–2,500 |

The frames before the fixes spent their time building the next room's
walkable grid in the switch's step (3–4 ms in the example, over a second
for a whole house) and opening the next ambience's stream in its feed
(3–4.7 ms). Steady frames here peak at 7–170 ms with nothing to do: the
GPU's `finish` on a shared machine, not loading. Reproduce with
`sariema build examples/rooms -o rooms.srmg` and
`sariema bench rooms.srmg --warmup 0 --runs 1 --frames 500 [--no-threads] --json`.

### Building a large room's world within the budget

A room of 20,316 collider parts whose two bodies' grids have 100,651
columns each, preloaded beside the active room while the game plays on,
`--no-threads` natively and in WebGL2. Its world and two grids take about
0.7 s of CPU natively. Before, the world was built in one share and each
grid's islands joined when it was finished.

| | Native, collision work a frame, ms: p99 · max | Native, frame thread CPU, max | WebGL2, frame CPU during the preload, mean · max | Frames with collision work |
|---|---|---|---|---|
| World in one share, islands at the end | — · 12–13 (world), 9–11 (islands) | — | — · 16–32 | — |
| Every stage in shares | 1.98–2.20 · 2.02–2.16 | 3.5–5.3 | 2.3 · 2.6 | 338–655 native, 472 WebGL2 |

Natively, at a load average of 4–5 on 10 cores, 3 runs of 1,200 frames: of
the 760–828 frames the system did not pause, one a run went over 2 ms, by
0.04 ms at most. The first frame of the browser takes 5.8–6.1 ms (the
first room's draw). The switch into the room, 1,094 steps in, takes
0.5–0.8 ms in WebGL2. The shares cost little: grids took 2 % more CPU
than built whole. Reproduce with
`sariema bench <game.srmg> --warmup 0 --runs 1 --frames 1200 --no-threads --timeline --json`
and, in a browser, `node test/bench.mjs --game <folder> --backend webgl2 --timeline 1260 --reps 2`.

In a browser, `node test/bench.mjs --game <folder> --timeline <frames>`
(in `packages/sariema-web`, after `npm run build` and `sariema web
examples/rooms -o <folder>`) plays the game from its first frame at the
display's pace and prints each frame's step, CPU time, time since the last
frame, passes and draws. Chromium 1248, ANGLE on Metal, 1920×1080, medium
tier, 3 runs of 800 frames each:

| Backend | Steady frame CPU, ms, p50 · max | Continuous switch (steps 170–229), max | Dissolve frames (steps 380–409), p50 · max |
|---|---|---|---|
| WebGL2 (downlevel path) | 0.5 · 2.0–5.3 | 4.0–6.9 | 0.5–0.6 · 0.8–0.9 |
| WebGPU | 0.3 · 0.5–1.9 | 3.4–10.7 | 0.3–0.4 · 0.5–1.3 |

The switch's own frame (step 380) takes 0.6–0.8 ms on WebGL2 and 0.4–1.2
ms on WebGPU, with one pass more than the frames of the dissolve, which
have one pass more than the frames after it. The peaks of the continuous
switch are its first frames and the attic, beside nothing, loading after
it. The time between frames does not show a frame missed in headless
Chromium: it stays at 16.6–16.8 ms even after a frame of 30 ms of CPU, so
these numbers are the frame's CPU, not its GPU.

## What was taken from other engines and games

- **Link's Awakening (screen transitions, from the technique study).**
  Inside a map, rooms stay loaded together and, at a doorway, the camera
  slides to the next room on a damped spring (ω 8.4 rad/s, ζ 0.8: 90 % in
  0.3 s, 99 % in 0.45 s) while the hero is nudged in; between maps, a
  fade. Here: `beside` rooms drawn together and `Continuous`; the fade
  stays the cutscene's, with `Cut` under it.
- **Godot (`ResourceLoader.load_threaded_request`).** Loading is requested
  ahead, polled, and fetched; fetching what is not done blocks until it is.
  Here: neighbours are requested by the room graph instead of by the game,
  and a switch to a room not in loads it at once, reported in the frame.
- **Bevy (asset server).** Assets load asynchronously behind handles, with
  a load state per handle. Here: the view's state per room (absent,
  decoding, uploading, resident) plays the handle's part.
- **Unity (additive scenes, `LoadSceneAsync`,
  `backgroundLoadingPriority`).** A scene loads additively next to the
  active one, and the time spent integrating it on the main thread is a
  budget per frame. Here: rooms are slots added to what is drawn, and
  decode, upload and world building each have a budget per frame.
- **Celeste, Hollow Knight.** Celeste keeps a level's rooms loaded and
  slides the camera between them; Hollow Knight loads the next scene behind
  a short fade at the doorway. Here: `Continuous` inside a house, `Dissolve`
  or a fade between places that do not continue each other.
- **Godot, Unity (screen-space transitions).** A common scene transition
  there captures the last frame (`get_viewport().get_texture()`, a
  render texture) faded over the next scene on a full-screen quad. Here:
  the same, with the capture taken from the renderer's own targets before
  the next frame draws, so it costs no read back and leaves out the HUD.
- **Unreal (`s.AsyncLoadingTimeLimit`), Unity
  (`Application.backgroundLoadingPriority`).** Integrating loaded objects
  on the main thread is limited to a time a frame, checked between
  objects, so a frame can go over by one object. Here: shares small
  enough to be checked often, and a share is left for the next frame
  when the longest one seen would not fit.
- **PixiJS v8 (`Assets`, `BackgroundLoader`, `PrepareBase`,
  `TextureGCSystem`).** Background loading runs one item at a time and
  pauses while a foreground load runs; a foreground load of an item being
  loaded in the background takes it over instead of starting again; the
  prepare system uploads a fixed number of resources a frame (4); textures
  idle for 3600 frames are released, checked every 600. Here: one room
  decoded or uploaded at a time in the order wanted, a switch takes over the
  room already on its way, the upload budget is bytes instead of a count
  (one texture of a room is megabytes), and rooms are freed by residency
  instead of idleness, since the room graph says which ones are coming.

## Known limits

- A collider is added in one share, whatever its triangles: 0.44 ms for
  the largest of a house with 124–236 colliders.
- `world_ms_per_frame` is wall time: a frame the system pauses mid-share
  goes over it, and on a busy machine the shares a frame shrink, so a
  room's world takes longer to be ready. At a load average of 30 on 10
  cores, the room above was not ready by its switch, 1,094 steps in, in 2
  of 6 runs; the switch then built the world in the step (5 ms) and the
  scripted move after it the actor's grid, whole (0.6 s).
- A walkable grid covers the room's static bounds at `nav.cell_size`
  unless the room declares `collision.nav_bounds` (`docs/physics.md`). A
  room whose geometry reaches far has a large grid: a tree-house room with
  ground 470 m across had 22.2 million columns, and the switch into it
  stopped 17–32 s while an actor walking across built its grid in the
  step; within its walls, 100,651 columns.
- An actor mid-move at a switch to a room whose grid is not built needs the
  grid at once, and builds it in the step: 0.37–0.49 s for that tree house
  when the switch comes 3.7 s after the game starts, before the shares of
  2 ms a frame reached it. Shares go to the active room's grids first,
  including those of bodies no actor uses yet.
- Grids are built for every body of the collision pack, whether an actor
  of that body is in the room or not.
