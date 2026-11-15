# Sariema — design

Sariema is an open-source native game engine that plugs into any React Native
app as a Fabric component and runs games downloaded at run time, with no
WebView. The host app ships the engine in its binary; each game arrives later
as a data package. That makes an in-app game launcher possible.

Games are written by AI coding agents, so the engine ships a CLI that
packages, validates, runs headless, profiles and takes screenshots, all with
structured output.

Priorities, in order: performance and visual quality first.

## Goals

- A Fabric component for React Native (new architecture) on iPhone and
  Android.
- 2D and 3D games.
- A downloaded game is data plus WebAssembly logic. No native code is ever
  downloaded.
- Engine written from scratch in Rust, rendering through `wgpu`.
- A packaging and tooling CLI designed for agents.
- Full isolation of every game: time, memory and access.
- A stable 60 frames per second on the floor devices.

## Non-goals

- A visual editor.
- Compatibility with another engine's projects, scenes or scripts.
- Metal 4. `wgpu` does not reach it today.
- Networked multiplayer.
- Apple TV and Android TV in the first release. Input is abstracted from the
  start and a fallback graphics path is planned, so they can follow without
  rework.
- Running on the web. The stack makes it possible; it is not a target.

## Anti-goals

These would be failures even if everything worked:

- Game logic crossing the WebAssembly boundary once per object per frame.
- Hitches caused by shader compilation during play.
- The same game looking different on iPhone and Android.
- An API so loose that an agent ships a slow game and nothing warns it.

## Constraints

- **iOS allows neither JIT nor downloaded native code.** WebAssembly is
  interpreted on iPhone. Published interpreter benchmarks put the fastest
  interpreters roughly 8 to 10 times slower than native code.
- **Store rules.** Platform rules allow downloading interpreted code within
  limits. The engine provides the isolation; what a given app publishes is the
  responsibility of whoever publishes that app.
- **`wgpu`.** There is no Metal pipeline cache, so every pipeline is created at
  load time. The underlying Metal objects are reachable, which allows
  Apple-only features such as MetalFX behind an option.
- **Floor devices (proposed):** iPhone XR (A12) and a 2021 mid-range Android
  phone with Vulkan 1.1.

## Decisions

| Question | Choice | Why |
|---|---|---|
| Engine base | Own core in Rust on `wgpu` | One renderer for Metal and Vulkan; engine and games in the same language; full control of the package, the bindings and the CLI |
| Game language | WebAssembly, written in Rust | Static types, no garbage collector, and the game's memory is one block the engine reads directly |
| Interpreter | `wasmi`, behind an internal interface | Pure Rust, strong published numbers on Apple Silicon, fuel metering. Confirmed only after measuring on a real iPhone |

## Architecture

```
 React Native app
 ┌──────────────────────────────────────────────────────────┐
 │  <GameView/> (Fabric)      control module (Turbo)        │
 │      │ native surface             │ messages             │
 └──────┼────────────────────────────┼──────────────────────┘
        ▼  C ABI                     ▼
 ┌──────────────────────────────────────────────────────────┐
 │  Rust core                                               │
 │   simulation thread ── fixed step ── calls the game      │
 │   render thread ────── owns wgpu, paced by the display   │
 │   audio thread         asset I/O pool                    │
 │                                                          │
 │   render │ assets │ animation │ collision │ audio │ input│
 │   2D UI and text │ WebAssembly machine (sandboxed)       │
 └───────────────────────────────┬──────────────────────────┘
                                 │ one command batch per step
                                 ▼
                    Game package: logic .wasm + data
```

### React Native layer

- One npm package with the Fabric component `GameView` and a control module.
- iOS: a `UIView` backed by `CAMetalLayer`. Android: a `SurfaceView` handing
  over its `ANativeWindow`. Both are thin and hand-written, and pass the
  surface pointer to the core through a small C ABI: create, resize, suspend,
  resume, destroy, load package, send and receive messages.
- The core is distributed as a prebuilt binary (xcframework and AAR),
  downloaded at install time with a pinned hash and wired by an Expo config
  plugin. Users of the engine do not need Rust installed.

### Threads and time

- Simulation runs at a fixed 60 Hz step on its own thread. Rendering runs at
  the display rate (60 or 120 Hz) and interpolates between the last two
  states.
- Resolution is dynamic, with hysteresis: it drops after several frames over
  budget and rises after several under. The engine never flips between 60 and
  30 frames per second from one frame to the next.
- Game logic is deterministic: fixed step, seeded random numbers, recordable
  input. The same package with the same input gives the same result, which is
  what makes headless tests and image comparison possible.

### The boundary between game and engine

The rule: **the game describes, the engine executes.**

- The engine owns the world (nodes, meshes, lights, actors, sounds). The game
  refers to everything by integer handles.
- Each step, the engine calls a single exported function, `tick`. The game
  writes a binary command batch into its own memory; the engine reads the
  whole batch at once. Events (collision, animation end, touch) come back in a
  single batch.
- Bulk data, such as the positions of many objects, lives in arrays in the
  game's memory, registered once. The engine reads them in place.
- Everything that repeats every frame runs in the engine: animation,
  interpolation, followers, path movement, particles, camera springs,
  triggers. The game switches them on, sets parameters and receives the
  completion event.
- The WebAssembly import surface is tiny. Ergonomics live in a Rust SDK that
  the game uses and that generates the commands. The batch format is
  versioned; a package declares the version it targets.
- Each game has a memory cap and an execution budget per step. Exceeding the
  budget does not hang the app: the engine stops the game and reports an error
  event to the host.
- A game reaches only what the engine exposes: no files, no network, no system
  APIs, unless the host app grants something through messages.

### Renderer

One renderer for 2D and 3D, with shaders in WGSL.

- Forward lighting into an HDR target. Per room: baked spherical-harmonics
  ambient light, one sun, and a few point or spot lights.
- Default material: PBR with a satin finish, plus an optional per-object rim
  light. Also unlit, additive and transparent.
- Shadows: a shadow map of the static scenery, redrawn only when the room
  changes, plus projected blob shadows under actors.
- A post-processing chain with per-room parameters: bloom from a chain of
  reduced resolutions; a miniature (tilt-shift) blur that reuses the same
  chain and always changes smoothly; a filmic tone curve; colour grading; and
  a final dither to avoid banding in gradients.
- Anti-aliasing is always on.
- Scenery is merged per material at packaging time; repeated objects are
  instanced.
- GPU skinning.
- **Game shaders:** a package may bring its own WGSL shaders in defined slots
  (a material's surface function and a post-processing pass). The CLI
  validates and compiles them at packaging time; the engine creates every
  pipeline at load time. Nothing is compiled during play.
- 2D: a sprite batch with atlases and an orthographic camera, in the same
  renderer.

### Assets and packages

- The CLI converts at packaging time: glTF becomes the engine's binary mesh
  format; textures become ASTC, read directly by the GPU; scenes and rooms are
  written as schema-validated JSON and compiled to binary.
- A package is a signed manifest plus content-addressed blocks, so an update
  downloads only the blocks that changed.
- Loading is per room. Music streams; voice lines load close to their use.

### Other modules

- **Animation:** skeletal clips, clip transitions, pieces attached to bones,
  animated material parameters.
- **Collision and paths:** a kinematic character against a grid, plus grid
  path search. A full physics engine enters when a game needs one.
- **Audio:** a mixer on its own thread with buses (music, voice, effects,
  ambience), loop points, music transitions and automatic ducking under voice.
- **Input:** abstract actions (movement axis, action button) fed by touch,
  gamepad or remote. The virtual joystick belongs to the engine.
- **UI and text:** a retained 2D layer inside the engine, because a downloaded
  game has to bring its own interface.
- **Saves and host:** the game writes to key-value slots; the host app
  persists them. A message channel connects app and game.

### CLI

Every command has a JSON mode with stable error codes. An unknown option is an
error, never ignored.

| Command | What it does |
|---|---|
| `new` | creates a game skeleton |
| `check` | validates schemas, shaders and **budgets** (triangles, draw calls, texture memory per room) |
| `build` | compiles the logic to WebAssembly and converts assets into a package |
| `run` | opens the game in a desktop window |
| `run --headless` | runs N steps with no window, with recorded input, and saves screenshots |
| `test` | compares screenshots against reference images |
| `profile` | per-frame and per-stage timings as JSON |
| `inspect` | contents and sizes of a package |
| `api` | describes the engine API in machine-readable form |

The desktop player uses the same core, so an agent can build and check a whole
game without a phone.

## Roadmap

1. **Core and Fabric component.** A room rendered on desktop; the Fabric
   surface on iPhone and Android; the WebAssembly boundary, measured on a real
   iPhone; actors, animation, collision and camera; script, audio, UI and
   saves, ending with one playable room inside a host app.
2. **Full CLI and final package format**, with the guide for agents.
3. **Download and launcher** on the React Native side.
4. **A full game port**, then the 2D path.
