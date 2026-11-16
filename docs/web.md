# The web and app WebViews

The engine runs in a browser tab and in a React Native WebView as one
WebAssembly module, whole games included: a game pack's logic, collision,
animation, sound and HUD step as they do natively, to the same
`engine_hash`. WebGPU draws where the browser has it; WebGL2, which every
target browser has, draws everywhere else. This page covers the model,
games, how their logic runs in the browser, the two render paths and what
each WebGL2 gap costs, the settings, textures, audio, the commands to
build, test, measure and serve, WebView notes, and what was taken from
Bevy, Filament, three.js and the gas meters of Substrate and NEAR.

## Model

```
page (JS)                                   engine (wasm, crates/sariema-web)
─────────────────────────────────────────   ─────────────────────────────────────
@sariema/web  mount(canvas, options)  ───►  Engine.create: WebGPU or WebGL2,
                                              the render path, the Stage
  loadGame(folder) / playGame(pack) ─────►  Engine.load_game: sariema-engine
                                              (logic under wasmi, collision,
                                              animation, sound, HUD) and its
                                              view (sariema-view)
  requestAnimationFrame(t) ──────────────►  Engine.frame(t): the steps due,
                                              the audio feed, a frame drawn
                                              at alpha
  pointer / keyboard events ─────────────►  a game's HUD and input settings,
                                              or the Stage's UI
  on("host"), answerSaves ◄──────────────►  Engine.take_host, answer_saves:
                                              the game's messages and loads
  ResizeObserver, devicePixelRatio ──────►  Engine.resize
  visibilitychange: stop, suspend audio
  webglcontextlost / device lost ─────────►  frame() returns −1: open_gpu, then
                                              a game's engine adopts the GPU
                                              (Engine.adopt); a Stage replays
                                              every load
  AudioWorklet reports frames played ────►  Engine.mix: blocks due, posted
                                              back to the worklet
  fetch packs (streamed into one buffer)  ►  load_game / load_room / load_actor
```

- **Fixed step.** Simulation runs at `step.step_s` (1/60 s) from the
  page's clock; a frame runs the steps due, at most
  `step.max_steps_per_frame` (4), and draws at the interpolation `alpha`
  between the last two. A long frame drops what it cannot catch up; a
  hidden page pauses and does not catch up the time it was away.
- **Input** reaches the engine as `InputEvent`s in physical pixels, queued
  and delivered on step boundaries: pointers to the UI, Tab and Shift+Tab
  to move focus, Enter and Space to activate. Keys are the engine's only
  while nothing else on the page has the focus (the event's target is the
  page or the canvas), so a page's own inputs, text areas and buttons keep
  their typing and Tab; a key released elsewhere still reaches the engine,
  so none stays held.
- **Two modes.** A game pack plays whole (next section). Without one, the
  Stage shows rooms and actors the page loads and places, with a UI and
  sounds of its own, as before.

## Games

A game pack (`.srmg`, which `sariema build` and `sariema web` write) plays
whole: `sariema_web::game::GameHost` holds `sariema-engine`'s `Engine`,
which steps the runtime, the game's logic, the cutscene sequencer,
animation, collision, sound and the HUD at the runtime's fixed 1/60 s, and
a `sariema_view::GameView`, which draws the room, the actors and the HUD at
the frame's `alpha` on the Stage's renderer. The window player and `sariema
run --headless --screenshot` draw through the same `GameView`, so a
browser frame after a replay is the native one (the browser tests compare
them). `docs/engine.md` has the order of a step.

- **Input.** Keys (the DOM `KeyboardEvent.code`) go through the game's
  input settings; pointers, in physical pixels, go to the HUD, so a touch
  held on the HUD's joystick moves the hero and a button node named after
  an input button holds it.
- **Sound.** The host takes the engine's mixer (`Feeding::Manual`): the
  step's sound commands play at once and the worklet pulls mixed blocks,
  as for a Stage. The engine's audio feed (on-demand sounds, music, stream
  decoding) runs after the frame's steps, on the main thread, outside the
  step.
- **Cutscenes, fades and bars** are the engine's, drawn at the step's
  value (the fade contract in `docs/engine.md`).
- **Host messages and saves** reach the page after the frame's steps; the
  page keeps saves and answers loads, or every load is answered as no
  save ([below](#host-messages-and-saves)).
- **A lost GPU** moves the game to a new device: `open_gpu` opens one on
  the canvas and the running engine adopts it (`Engine.adopt`), loading
  into it the room and actors its game host kept, so the game goes on at
  the step it had reached, hash unchanged. If the GPU cannot be opened
  the engine stays as it was and a later `rebuild()` tries again.
- **Replay.** `replayGame(log, to)` steps to `to` with an input log, each
  step followed by the audio feed, and returns the hashes `sariema run
  --headless --game --input` prints. It goes on with the log it started
  with; another log is `E_USAGE`. A game played with `audio: false`
  mixes its sound with the steps as that run does; with sound live,
  `engine_hash` still matches and `audio_hash` follows the device.
- **A game that never returns** is stopped by its budget (`E_GAME_BUDGET`),
  as natively: the frame throws, the package stops its loop and emits
  `error`, and the page goes on. A replay ends at the step that stopped
  the game and names it (`step 29: ...`), the step `sariema run
  --headless` stops at. The tick that ran out stalls the page for its
  budget's worth of fuel ([numbers](#numbers)).

## Game logic in the browser

A game's logic is WebAssembly run by `sariema-vm` under a fuel budget and
a memory ceiling. In the browser it runs the same way: wasmi is compiled
into the engine's own `.wasm` and interprets the game's module there.

Ruling: wasmi inside the engine's `.wasm`, not the browser's WebAssembly
with gas injected at packaging — wasmi gives the native trip point of the
budget, the same canonical NaNs (`deterministic`) and the same host calls
in Rust, so `engine_hash` matches native by construction (the browser
tests show it in four configurations), and the climb's logic costs about
1 µs a step of an 80 µs step — a game whose logic does heavy work pays
for it: a tick of 20,000 float iterations takes 0.60–0.75 ms here against
0.1 ms natively and 0.01 ms on the browser's JIT, so a logic that needs
2 ms natively would need 12–15 ms in a page; that game needs the
injected-gas path.

The study is `tools/vm-bench`, run from the repository with

```sh
node tools/vm-bench/run.mjs
```

It builds `examples/climb`'s pack, records its first 900 ticks over its
input log (each step's event batch) and times them under native wasmi,
then runs those ticks and a synthetic tick in Chromium and WebKit, seven
repetitions each, these ways:

- **A**: wasmi compiled into a page's `.wasm` (the engine's build: opt
  level `s`, LTO, `wasm-opt -Oz`), with `VmSettings::default()`.
- **B**: the game's module instrumented at packaging as wasm-instrument's
  mutable-global mode does (Substrate's `pallet-contracts`; NEAR's
  `finite-wasm` charges at the same block boundaries): an exported `i64`
  charged at the start of every instruction sequence and a trap below
  zero; run by the browser's own WebAssembly with the `sariema` imports
  written in JavaScript.
- **B0**: B without the gas, the browser's floor.

Microseconds per tick, medians and [min–max] of seven runs (each run 900
climb ticks on ten instances, or 60 synthetic ticks), load average
8.5–13.8 from other work on the machine:

| Workload | Browser | Native wasmi | A | B | B0 |
|---|---|---|---|---|---|
| climb | Chromium | 0.27 [0.26–0.33] | 1.40 [1.30–1.83] | 0.30 [0.23–0.66] | 0.22 [0.20–0.26] |
| climb | WebKit | ″ | 1.00 [0.89–1.89] | 0.33 [0.22–0.78] | 0.22 [0.22–0.33] |
| synthetic | Chromium | 99 [76–176] | 748 [737–788] | 10 [8–17] | 10 [8–15] |
| synthetic | WebKit | ″ | 600 [600–667] | ≤ 17 (timer) | ≤ 17 (timer) |

A tick that never returns, on the first tick's budget: B's trap came in
0.6 times A's stop in Chromium and 18 times it in WebKit.

What decided it:

- **Determinism.** A runs the native interpreter: same fuel, NaNs
  canonical, the same Rust host. B would need NaN canonicalization
  injected after every float operation (browsers do not canonicalize), the
  native host to run the same instrumented module so the budget trips at
  the same step, and the imports rewritten in JavaScript. Writing them, the
  prototype's `button` first got the name id as a signed `i32`, so every
  id above 2³¹ missed and the climb never saw its action button: that
  class of bug is B's standing cost.
- **Cost.** For the climb the logic is noise either way (A adds about
  1.1 µs to a step of 80). On heavy logic B is 75 times faster in
  Chromium and at least 35 in WebKit, whose timer reads 1 ms.
- **Budget.** A's runaway tick stalls the page for its fuel, as long as
  the browser tests measure ([numbers](#numbers)); B's trap took 18
  times longer in WebKit.
- **Size.** Injection grew the climb's module from 60.9 KB to 73.2 KB
  (+20%, 937 sequences charged); wasmi adds to the engine's `.wasm`
  whether B is used or not, since native hosts keep it.

A fourth way was tried and dropped: A built with the WebAssembly
tail-call proposal on (`-C target-feature=+tail-call`), so wasmi takes its
tail-call dispatch; the dispatch still recursed on the browser's stack and
a long tick overflowed it. That is also why the workspace turns on
wasmi's `auto-dispatch`: without it, wasm32 (no tail-call proposal) took
the tail-call dispatch, which recursed once per instruction run, and any
tick longer than a few thousand instructions threw `RangeError: Maximum
call stack size exceeded` and left the engine borrowed; the loop dispatch
it falls back to keeps the stack flat. It falls back the same way for a
native wasmi built at opt-level 0 or 1 (a project depending on
`sariema-vm` with its own unoptimised debug profile); this workspace builds
wasmi at opt-level 3 in debug too, so its native builds always take the
tail-call dispatch.

## Backends and render paths

`Engine.create` picks the backend once: WebGPU when `navigator.gpu` gives
an adapter (or `backend: "webgl2"` forces GL), WebGL2 otherwise. The
renderer then picks its path once from the device's limits
(`RenderPath::of`): **full** where a shader stage has 4 storage buffers and
the backend is not GL, **downlevel** elsewhere. `path` forces either; the
downlevel path also runs natively, which is how `cargo test` covers it.

| WebGL2 gap | What the downlevel path does | Cost |
|---|---|---|
| No storage buffers | Lights, clusters, light indices and shadow records are rows of `Rgba32Uint` / `R32Uint` data textures `data_texture_width` texels wide, read with `textureLoad`; floats travel as their bits | Tables are rewritten as texture rows; capacity is width × the device's 2D size limit |
| Actor records (skinning palettes, face cells) | One uniform buffer of `actor_uniform_rows` vec4 rows (1024 = 16 KiB, WebGL2's minimum) per actor; an actor whose joints and faces need more rows is refused when loaded | About 250 joints per actor |
| No base vertex in draws | Indices are baked with their mesh's base vertex on every path | None; native output is byte-identical |
| No multisampled texture sampling | With MSAA, the scene's depth is discarded and opaque geometry is drawn again into a single-sample depth texture for post-processing | One more pass (15 instead of 14 on the demo room) |
| No `textureLoad` from depth textures in GLSL ES 3.0 | Post-processing binds the scene's depth as a float texture and reads it through a nearest sampler (wgpu's GL backend leaves a depth texture incomplete for a load without one) | One sampler binding |
| No texture level query | The composite takes the bloom's level count as a uniform, on every path | None |
| Uniforms bound in 16-byte rows | Every uniform struct is a multiple of 16 bytes; `glsl_es.rs` checks every downlevel shader translates to GLSL ES 3.00 | None |
| Float render targets need `EXT_color_buffer_float` | Required: a device that cannot draw into `FLOAT_TARGETS` (`Rgba16Float`, `R32Float`, `Rg16Float`) is refused with `E_GPU` naming them. There is no LDR fallback | Browsers without the extension cannot run the engine; the probe page reports it |
| No timestamp queries | `Renderer::time_gpu_passes` is unavailable; stats count passes and draws | No GPU pass times on WebGL2 |
| No compute | The engine uses none | — |
| 16 vertex attributes, 4 bind groups | The pipelines use at most 11 attributes (the UI's quads) and 3 groups | — |

WebGPU itself guarantees only 1 and 4 samples per pixel, so on WebGPU the
quality's MSAA becomes 4 or 1. `capabilities()` reports the backend, path,
MSAA, surface format, adapter limits and the texture variants the device
takes.

## Settings

The page passes these to `mount(canvas, options)`; every one is optional.
The engine's names (snake case, `WebSettings` in `settings.rs`) are in
brackets.

| Option | Default | |
|---|---|---|
| `backend` | `"auto"` | `"webgpu"` or `"webgl2"` forces one |
| `quality` | `"medium"` | `"low"`, `"medium"`, `"high"`: the renderer's tier |
| `path` | `"auto"` | `"full"` or `"downlevel"`; `"full"` without storage buffers fails |
| `msaa` | the tier's | samples per pixel; the device may hold fewer |
| `maxPixelRatio` (`max_pixel_ratio`) | 1.5 | a phone's DPR of 3 draws at 1.5, as the three.js game does |
| `compressedTextures` (`compressed_textures`) | true | false loads the RGBA8 packs on every device |
| `shadows` | true | false draws no shadow maps |
| `ambientOcclusion` (`ambient_occlusion`) | true | false turns SSAO off |
| `step.step_s`, `step.max_steps_per_frame` | 1/60, 4 | the fixed step |
| `audio.lead_s`, `audio.block_frames` | 0.1, 1024 | how far the mixer runs ahead (at least two blocks), in blocks of this many frames (whole 128-frame quanta) |
| `dataTextureWidth` (`data_texture_width`) | the tier's, 1024 | texels per row of the downlevel path's light, cluster and shadow tables |
| `actorUniformRows` (`actor_uniform_rows`) | the tier's, 1024 | rows of one actor's uniform record on the downlevel path (1024 = 16 KiB) |

The renderer clamps the downlevel sizes to the device: `actor_uniform_rows`
to `max_uniform_buffer_binding_size` / 16, and the light and cluster
budgets, which are the tier's, to what the data textures fit.

## Textures

| Backend | Compressed formats the engine asks for |
|---|---|
| WebGPU | `texture-compression-astc` (Apple, most phones), `-bc` (desktops), `-etc2` |
| WebGL2 | `WEBGL_compressed_texture_astc`, `_s3tc`, `_etc`, through wgpu's features |

Packs store colour textures as ASTC 4×4 or as RGBA8. `sariema web <project>
-o <dir>` builds every room and actor in both, `<id>.srma` and
`<id>.rgba8.srma`, and writes `sariema-web.json` naming them. The page asks
`capabilities().texture_variants` (`["astc", "rgba8"]` where ASTC samples,
`["rgba8"]` elsewhere) and fetches the first the game has.

Sizes of the Origem rooms (MB):

| Pack | ASTC raw / gzip / brotli | RGBA8 raw / gzip / brotli |
|---|---|---|
| origem-s5 | 4.17 / 1.39 / 1.23 | 10.92 / 1.46 / 1.28 |
| origem-base-s11 | 17.36 / 6.14 / 4.96 | 61.40 / 11.62 / 9.61 |
| origem-base-s12 | 17.36 / 6.14 / 4.96 | 61.40 / 11.62 / 9.61 |

ASTC is a quarter of the GPU memory and about half the download of the
textured rooms; RGBA8 compresses well over the wire but not in memory.
Load times are in [Numbers](#numbers).

## Audio

The mixer (`sariema-audio`, `Feeding::Manual`) runs on the page's main
thread; an AudioWorklet (`audio-worklet.js`) plays the interleaved stereo
blocks posted to it and reports the frames it has played once per block
(`audio.block_frames`, passed to it when it is made). Each report asks the
engine for the blocks that keep `lead_s` mixed ahead; the lead is at least
two blocks, so the next block is already queued when one is reported. A
new sound bank flushes what the old one queued and counts frames afresh,
so the lead holds however many banks are loaded. There is no `SharedArrayBuffer`, so the page needs no
cross-origin isolation; streamed music is decoded on the main thread by the
stage's steps. A sound played now is heard about `lead_s` later.

A game's sound takes the same road: the engine's mixer, taken by the host,
mixes the blocks a report asks for, and the engine's audio feed decodes
its music after the frame's steps. The climb's music plays through it
without an underrun at the default lead in every browser test.

Browsers start an `AudioContext` suspended until a user gesture: the
package resumes it on the first `pointerdown`, or call `resumeAudio()` from
your own button. AudioWorklet, like WebGPU, exists only in a secure context
(HTTPS or `localhost`). A hidden page suspends the audio with the loop.

## The package

```ts
import { mount } from "@sariema/web";

const sariema = await mount(canvas, { quality: "medium" });
await sariema.loadGame("/games/climb/");       // a game project's folder: plays its pack
sariema.on("error", (e) => console.log(e.code)); // E_GAME_BUDGET, E_GAME_TRAP, ...
sariema.on("lost", () => console.log("moving to a new GPU"));
```

A folder of a project without a game is shown on the Stage:

```ts
await sariema.loadGame("/games/origem/");      // sariema-web.json, start room, sounds, UI
const tito = sariema.spawnActor(await sariema.loadGameActor("tito"));
sariema.setActor(tito, { transform, clip: "walk", speed: 1 });
sariema.on("ui", (e) => console.log(e.type, e.node));
```

`loadGame(folder, { seed, audio })` and `playGame(pack, { seed, audio })`
start a game from its first step (seed 0, as `sariema run --game`); a
seed is a whole number from 0 to 2⁶⁴ − 1, a `bigint` past 2⁵³ − 1, and
anything else is `E_USAGE`. `audio: false` mixes its sound with the steps
instead of playing it.
`replayGame(log, to)` steps it with an input log and returns its hashes;
`gameHashes()` gives them at any time; `stats().game` gives its steps, the
hero's position and why it stopped.

`start()` / `stop()` run and stop the frame loop, or `frame(now)` runs one
frame from a loop the page already has; `capture()` reads a frame back;
`bench(n)` times n draws; `stats()` gives the last frame's passes, draws,
uploads and steps; `destroy()` lets go of the GPU and audio. On a lost
WebGL context or WebGPU device the package moves a playing game to a new
device on the same canvas, or makes a new engine and replays every load
in order, emitting `lost` and `restored`, or `error` if that fails (for
instance a canvas that gives no context, or a pack that can no longer be
fetched); `rebuild()` tries again. Packs are fetched as they stream in; a Content-Length is only a
size hint, since a compressed response from another origin shows its
compressed length and hides its encoding.

## Host messages and saves

The game's messages to its host reach the page as `host` events after the
frame's steps, each with the `step` that sent it:

| `type` | Fields | For the page |
|---|---|---|
| `ready` | | the game is ready to be shown |
| `exit` | | the game asks to close; the page decides |
| `orientation` | `orientation` | the orientation the game wants |
| `save` | `key`, `value` (`Uint8Array`) | a value to keep under the key |
| `load_saves` | | the game asks for every saved key |
| `message` | `topic`, `payload` (`Uint8Array`) | anything the game and the app agree on |
| `stopped` | `code`, `message` | the game stopped for good |

The package keeps no saves: storage is the app's. A page that keeps them
listens for `saveLoad` and answers it with `answerSaves`, an object of keys
to bytes:

```ts
const saves = JSON.parse(localStorage.getItem("saves") ?? "{}");
sariema.on("host", (m) => {
  if (m.type !== "save") return;
  saves[m.key] = Array.from(m.value);
  localStorage.setItem("saves", JSON.stringify(saves));
});
sariema.on("saveLoad", () => sariema.answerSaves(saves));
await sariema.loadGame("/games/meadow/");
```

While anything listens for `saveLoad` the page answers the game's loads,
at once as above or later, from an asynchronous store (IndexedDB) or from
the app behind a WebView, which takes a round trip of a few frames. A load
still unanswered after `host.save_load_timeout_s` (5 s of game time) is
answered as no save, the engine logs a warning, and a later `answerSaves`
returns false. With no listener, every load is answered as no save at the
step it was asked. Either way a game that waits for `save_load_done`
always starts. `replayGame` answers loads as no save at once whatever
listens, as `sariema run --headless` without `--saves` does, so its hashes
match; its messages reach `host` when it returns. Between frames the
queue holds 256 messages and 4 MiB (`host.max_messages`, `host.max_bytes`);
a message past them is refused with `E_HOST_QUEUE_FULL`.

In a React Native WebView the app keeps the saves: the page forwards
`host` events, and the app answers through `injectJavaScript`:

```ts
const bytes = (b?: Uint8Array) => (b ? Array.from(b) : undefined);
sariema.on("host", (m) => window.ReactNativeWebView.postMessage(
  JSON.stringify({ ...m, value: bytes("value" in m ? m.value : undefined), payload: bytes("payload" in m ? m.payload : undefined) }),
));
sariema.on("saveLoad", () => {});
window.answerSaves = (saves) => sariema.answerSaves(saves); // webview.injectJavaScript(`answerSaves(${json})`)
```

## Build, test, measure, serve

Prerequisites: the toolchain of `rust-toolchain.toml` with the
`wasm32-unknown-unknown` target, `wasm-bindgen-cli` at the version of the
workspace's `wasm-bindgen` (`cargo install wasm-bindgen-cli --version
0.2.130 --locked`), Node.

```sh
cd packages/sariema-web
npm install
npx playwright install chromium webkit   # once; Playwright 1.64.1 uses chromium-1248

npm run build      # dist/: the .wasm (wasm-opt -Oz), bindings, typed API, worklet,
                   # probe page and the demo game (examples/climb); prints the
                   # .wasm's sizes (--speed: opt-level 3 and wasm-opt -O3 instead)
npm test           # test/run.mjs, about a minute
```

A game project becomes a folder the package plays with `sariema web`:

```sh
sariema web ../../examples/climb -o dist/demo --json   # game.srmg (ASTC), game.rgba8.srmg, sariema-web.json
```

`npm test` writes the fixtures and native references with `cargo run
--example fixtures`, builds `examples/demo` and `examples/climb` with
`sariema web`, renders the climb natively at steps 120, 300, 600 and 900
with `sariema run --headless --game --input --screenshot` and times its
native replay (`cargo run --example replay`), builds the counting `.wasm`
(`npm run build -- --alloc-count`, whose allocator counts), serves them,
and in Chromium with WebGPU, Chromium with WebGPU hidden (WebGL2), WebKit
with WebGPU and WebKit with WebGPU hidden: draws the demo room with and
without occlusion and the posed arm and compares each with the native
golden, loads the game folder, and loads it again gzipped from another
origin; plays a sound through the worklet and checks its samples arrive
without underruns, loads a second sound bank and checks the lead holds,
and plays with an 18 ms lead in 128-frame blocks; loses and restores the
context or device and compares again; taps a UI button; types into a page
input, which keeps its keys while the engine takes keys sent to the page;
checks a rebuild that cannot fetch its room emits `error`; runs the probe
page; and remounts without compressed textures to check the RGBA8 packs
load. Then the climb: replayed with its input log it reaches the native
`engine_hash`, `state_hash` and `physics_hash` at each of the four steps,
and its frame there matches the native one; asked to go on with another
log it is `E_USAGE` and stays; seeds out of range are `E_USAGE`, and one
past 2⁵³ reaches the native hashes; its step is timed (900 steps, seven
times); played live, its music reaches the worklet without an underrun at
the default lead; a synthetic touch drag on the HUD's joystick moves the
hero; the page keeps a game's save and answers its load frames later,
and with no listener for loads a load is answered at once; a game whose
logic never returns raises `E_GAME_BUDGET`, stops the
loop, and a timer fires on time after it, and one whose 30th tick never
returns stops at the step and with the hashes it does natively; a GPU
lost mid-game leaves the game at its step and hash, and it goes on
drawing, also when the canvas first gives no context and a second restore
succeeds; and, on the counting build, its replayed steps allocate at the
steps and as often as natively, nothing from 600 to 1200 nor while only
the HUD animates, and a second of live frames no more than the audio
feed's steady second. Results and the numbers land in `test/out/results.json`, the
browsers' climb frames in `test/out/renders/`. The tolerance is the
goldens' own: no channel more than 6 apart, or at most 0.5% of pixels.
`SARIEMA_BROWSERS=chromium-webgl2,webkit-webgpu` picks configurations;
`SARIEMA_GAME_PACKS=<dir>` adds every room pack in `<dir>`, compared with a
native frame the fixtures example writes beside it.

```sh
node test/bench.mjs --pack origem-s5.srma --backend webgl2 \
  --viewport 844,390,3 --msaa 1 --no-shadows --no-ao --reps 3
```

prints one JSON line a run: the steps to the first frame (mount, fetch,
load, first frame, from navigation), the median CPU of 120 frames at the
display's pace, and a burst of 300 draws up to one read back, per frame
CPU and wall time, with the load average. A game folder plays instead,
replayed first; its `replay` hashes are the ones `sariema run --headless`
prints for the same log and steps:

```sh
node test/bench.mjs --game test/out/fixtures/climb --log ../../examples/climb/input/play.json \
  --steps 120 --browser webkit --backend webgl2 --viewport 390,844,3 --reps 3
```

Natively, `cargo test -p sariema-render` covers the downlevel path: its
goldens within tolerance, limit detection, data-texture round trips,
skinning through the uniform fallback, and the GLSL ES 3.00 translation of
every shader. `cargo test -p sariema-web` runs the stage and the game host
on a native device; `--features alloc-count` adds the steady-step
allocation test.

### Serving to a phone on the same network

```sh
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=sariema-dev" \
  -addext "subjectAltName=DNS:localhost" -keyout key.pem -out cert.pem
npm run serve -- --host 0.0.0.0 --port 8787 --cert cert.pem --key key.pem
```

The server prints the addresses it listens on; open
`https://<this Mac's address>:8787/dist/probe/` on the phone and accept the
certificate warning. Without `--cert` it serves plain HTTP, where browsers
withhold WebGPU and AudioWorklet from any address but `localhost`. Any
static host serves `dist/` as it is; the `.wasm` must go out as
`application/wasm`, and gzip or brotli on the host cuts it to the sizes in
[Numbers](#numbers).

### The probe page

`dist/probe/` reports, as one JSON object to copy: WebGPU (adapter,
features, limits, canvas format), WebGL2 (renderer, limits, extensions,
whether an `RGBA16F` target is complete and its sample counts),
AudioWorklet, WASM SIMD and threads, secure context and pixel ratio. Its
smoke test mounts the engine, plays the demo game (the climb) for a
moment, times the first frame and checks the game's music reached the
worklet.

## WebView notes

- **iOS (WKWebView).** WebGL2 is on by default since iOS 15; WebGPU comes
  with the system's Safari. WebKit on macOS gives WebGL2 both
  `EXT_color_buffer_float` and ASTC (the browser tests run there); confirm
  a phone with the probe in the app's own WebView. Load the page over
  HTTPS or as a file URL so it is a secure context. Set
  `allowsInlineMediaPlayback` and leave
  `mediaTypesRequiringUserActionForPlayback` empty, or audio waits for a
  tap. An app sent to the background can lose its WebGL context; the
  package rebuilds on `webglcontextrestored`.
- **Android (System WebView).** Chromium's WebGL2 and AudioWorklet; ASTC
  on most phone GPUs, otherwise RGBA8. Whether WebGPU is exposed depends
  on the WebView version and GPU: run the probe in the app's own WebView,
  not in Chrome.
- **React Native.** With `react-native-webview`, pass
  `mediaPlaybackRequiresUserAction={false}` and
  `allowsInlineMediaPlayback`, serve the game folder from an HTTPS origin
  or bundled files, and forward events to the app with
  `sariema.on("ui", (e) => window.ReactNativeWebView.postMessage(JSON.stringify(e)))`,
  and the game's `host` messages and saves as
  [above](#host-messages-and-saves).
  A WebView in a tab that is not visible is hidden to the page, which
  pauses the loop and the audio.

## Numbers

Apple M5, Chromium 156 (Playwright's chromium-1248, ANGLE on Metal) and
Playwright's WebKit (webkit-2370), headless; the load average is given
with each set. While the climb's numbers were taken an emulator outside
this work held the machine's load at 6–10, so they are an upper bound.

**The `.wasm`** (`npm run build`, cargo `opt-level = "s"`, LTO, one
codegen unit, `panic = "abort"`, then `wasm-opt -Oz`), with the whole
engine; the build before games (the Stage alone) in the last column:

| Stage | Bytes | Before games |
|---|---|---|
| wasm-bindgen output | 9,786,713 | 6,699,337 |
| after wasm-opt | 5,499,724 | 4,040,137 |
| gzip -9 | 2,091,746 | 1,644,544 |
| brotli 11 | 1,557,398 | 1,243,013 |

The engine, wasmi, the runtime, collision, animation and the HUD add 1.46
MB raw and 314 KB brotli.

**The climb's step.** A replayed step and its audio feed (the sound mixed
with the steps), 900 steps, seven runs, milliseconds a step, median
[min–max]; native is the release build on the same machine, run by
`cargo run --example replay`; load average 6.3–7.1:

| Host | Step |
|---|---|
| native | 0.039 [0.039–0.049] |
| Chromium, WebGPU | 0.079 [0.078–0.083] |
| Chromium, WebGL2 | 0.082 [0.081–0.083] |
| WebKit, WebGPU | 0.079 [0.076–0.089] |
| WebKit, WebGL2 | 0.079 [0.077–0.083] |

A step costs about twice the native one in either browser; the backend
does not enter the step. Of the 0.08 ms the logic under wasmi is about
0.001 ms (see [Game logic in the browser](#game-logic-in-the-browser)).

**The climb's frame**, replayed to step 120 and then played on with no
input at the medium look; paced is the CPU of `frame(now)` (its steps and
its draw), median of 120 frames at the display's pace, for each of three
runs; burst is a burst of 300 draws up to one read back, wall and CPU a
frame, median [min–max] of the three; load average 7.5–9.3; WebKit's timer
reads whole milliseconds:

| Browser | Backend | Buffer | Paced CPU | Burst wall | Burst CPU |
|---|---|---|---|---|---|
| Chromium | WebGPU | 1920×1080 | 0.4, 0.4, 0.5 | 0.96 [0.94–1.34] | 0.07 |
| Chromium | WebGPU | 585×1266 | 0.4, 0.5, 0.5 | 0.69 [0.68–0.70] | 0.08 |
| Chromium | WebGL2 | 1920×1080 | 0.7, 0.8, 0.9 | 1.52 [1.48–1.52] | 1.06 |
| Chromium | WebGL2 | 585×1266 | 0.6, 0.6, 1.0 | 0.83 [0.83–0.83] | 0.58 |
| WebKit | WebGPU | 1920×1080 | 0, 1, 1 | 1.00 [0.98–1.26] | 0.09 |
| WebKit | WebGPU | 585×1266 | 0, 0, 0 | 0.69 [0.68–0.83] | 0.09 |
| WebKit | WebGL2 | 1920×1080 | 1, 1, 1 | 1.47 [1.47–1.48] | 1.36 |
| WebKit | WebGL2 | 585×1266 | 1, 1, 1 | 0.83 [0.83–0.84] | 0.78 |

585×1266 is a 390×844 page at pixel ratio 3, capped at 1.5.

**The climb's first frame** at 1920×1080, from navigation, five cold
browser contexts, served over loopback, milliseconds, medians; load is
`playGame` (decoding the pack, compiling the logic, loading the room,
actors, sounds and HUD); load average 8.0–9.9:

| Browser | Backend | Navigation → first frame [min–max] | Mount | Fetch | Load | First frame |
|---|---|---|---|---|---|---|
| Chromium | WebGPU | 133 [126–177] | 22 | 3 | 59 | 23 |
| Chromium | WebGL2 | 277 [229–283] | 127 | 4 | 73 | 29 |
| WebKit | WebGPU | 120 [107–172] | 40 | 10 | 41 | 5 |
| WebKit | WebGL2 | 260 [244–443] | 115 | 5 | 51 | 52 |

The game folder is 448 KB a variant (`game.srmg`, ASTC; the climb's one
texture makes the variants alike).

**Allocations**, on the counting build: the climb's replayed steps
allocate in every browser at the steps and as often as natively, 129
times in 18 of its 1,200 steps, none after step 600 and none while the
HUD only animates; `docs/engine.md` lists where. A second of live frames
(steps and audio feed, the music streaming) makes 44–48, the music
decoder's packets, as the feed makes natively (45 a second).

**A logic that never returns** raises `E_GAME_BUDGET` after its first
tick's 200 M fuel: 535–564 ms in Chromium, 320–361 ms in WebKit, the
longest gap a 10 ms timer saw 298–525 ms (load average 8–12). A later
tick's budget is a tenth of that, 20 M fuel, about 30–55 ms: two to three
frames at 60 Hz. `vm.fuel_first_step` and `vm.fuel_per_step` in the
project's settings set the two budgets, the same on every host.

**Time to first frame** of an Origem room at the medium look, measured
with the build before games, a phone-sized
page (844×390 at pixel ratio 3, drawn at 1266×585), served over loopback
(so the fetch has no network in it), a cold browser for each run, medians
of three, load average 6.8–7.8. Milliseconds:

| Room | Pack | Backend | MB | Navigation → first frame | Mount | Fetch | Load | First frame |
|---|---|---|---|---|---|---|---|---|
| s5 | ASTC | WebGPU | 4.2 | 91 | 29 | 6 | 9 | 19 |
| s5 | ASTC | WebGL2 | 4.2 | 186 | 114 | 12 | 8 | 10 |
| s5 | RGBA8 | WebGPU | 10.9 | 124 | 26 | 19 | 19 | 6 |
| s5 | RGBA8 | WebGL2 | 10.9 | 241 | 124 | 15 | 18 | 26 |
| base-s11 | ASTC | WebGPU | 17.4 | 123 | 23 | 24 | 22 | 21 |
| base-s11 | ASTC | WebGL2 | 17.4 | 220 | 118 | 23 | 21 | 11 |
| base-s11 | RGBA8 | WebGPU | 61.4 | 203 | 18 | 74 | 63 | 15 |
| base-s11 | RGBA8 | WebGL2 | 61.4 | 366 | 117 | 52 | 62 | 97 |
| base-s12 | ASTC | WebGPU | 17.4 | 137 | 19 | 26 | 22 | 23 |
| base-s12 | ASTC | WebGL2 | 17.4 | 223 | 119 | 31 | 22 | 14 |
| base-s12 | RGBA8 | WebGPU | 61.4 | 186 | 16 | 52 | 67 | 4 |
| base-s12 | RGBA8 | WebGL2 | 61.4 | 372 | 115 | 59 | 72 | 99 |

Mount is compiling the `.wasm`, making the device and the renderer; on
WebGL2 it is about 100 ms longer, ANGLE compiling the post chain's
shaders. The script itself starts about 35 ms after navigation in every
row. RGBA8 costs where the bytes are: 3.5 times the bytes, about three times
the fetch and load, and on WebGL2 about 85 ms more in the first frame.
Over a network the fetch dominates: a phone downloads the brotli sizes in
[Textures](#textures).

**Frame time against three.js.** The Origem rooms in one Chromium
session (three.js on WebGL2, as the game ships), interleaved, three
repetitions of 300 frames, medians; load average 6.3–14.7. three.js's side
is the perf lane's render-only burst of the game (the render call alone,
no tick, characters hidden), 6 passes. The engine's side is a burst of
draws read back once at the end. "lfl" is the perf lane's like for like
(no MSAA, shadows or occlusion, its `one-s11`/`one-s12` packs), the fair
comparison; "medium" is the engine's own look, with MSAA, shadows,
occlusion and bloom. Wall per frame including the GPU, in milliseconds,
± the spread (largest − smallest) of the three repetitions, and in
brackets the engine's time over three.js's; ≈ marks a difference no larger
than the two spreads added, which this run cannot tell from noise:

| Buffer | Room | Look | three.js | WebGPU | WebGL2 | Passes |
|---|---|---|---|---|---|---|
| 1920×1080 | s11 | lfl | 1.55 ±0.18 | 0.98 ±0.05 (0.63×) | 1.21 ±0.11 (0.78×) | 13 / 13 |
| 1920×1080 | s11 | medium | ″ | 1.41 ±0.51 (≈) | 2.10 ±0.09 (1.35×) | 16 / 17 |
| 1920×1080 | s12 | lfl | 1.22 ±0.01 | 0.86 ±0.53 (≈) | 1.04 ±0.09 (0.85×) | 13 / 13 |
| 1920×1080 | s12 | medium | ″ | 1.47 ±0.34 (≈) | 2.61 ±0.16 (2.14×) | 16 / 17 |
| 1920×1080 | s5 | lfl | 0.84 ±0.05 | 0.95 ±0.04 (1.14×) | 1.61 ±0.13 (1.92×) | 9 / 9 |
| 1920×1080 | s5 | medium | ″ | 1.85 ±0.48 (2.21×) | 3.05 ±0.46 (3.64×) | 13 / 14 |
| 1266×585 | s11 | lfl | 0.57 ±0.00 | 0.55 ±0.16 (≈) | 0.53 ±0.01 (0.93×) | 13 / 13 |
| 1266×585 | s11 | medium | ″ | 0.66 ±0.02 (1.17×) | 0.82 ±0.06 (1.44×) | 16 / 17 |
| 1266×585 | s12 | lfl | 0.45 ±0.00 | 0.68 ±0.16 (1.53×) | 0.53 ±0.00 (1.19×) | 13 / 13 |
| 1266×585 | s12 | medium | ″ | 0.66 ±0.02 (1.47×) | 0.96 ±0.07 (2.15×) | 16 / 17 |
| 1266×585 | s5 | lfl | 0.35 ±0.00 | 0.42 ±0.00 (1.19×) | 0.65 ±0.01 (1.85×) | 9 / 9 |
| 1266×585 | s5 | medium | ″ | 0.59 ±0.19 (1.69×) | 1.14 ±0.02 (3.26×) | 13 / 14 |

The earlier full run (load 7.2–10.5) gives the same verdict in every
cell but one, noted below; ranges cover both runs.

- **WebGPU, like for like,** is faster than three.js in s11 at 1080p (by 0.58 ms here,
  0.25 ms in the earlier run). It is within the noise in s12 at 1080p (here
  −0.36 ms on a 0.53 ms spread, earlier +0.07 ms) and in s11 at phone size.
  It is slower in s5 at both sizes (1.1–1.2×, 0.06–0.11 ms) and in s12 at
  phone size (1.2–1.5×, 0.10–0.23 ms).
- **WebGL2, like for like,** the backend three.js uses and the only one
  in WebViews without WebGPU (iOS before 26), is faster in s11 and s12 at
  1080p (0.74–0.86×) and in s11 at phone size (0.93×). It loses in s5,
  1.8–1.9× at both sizes (+0.73–0.77 ms at 1080p, +0.29–0.30 ms at phone
  size), and in s12 at phone size (1.2×, +0.07–0.09 ms).
- **The medium look** costs 1.2–2.3× three.js on WebGPU, except s11 and
  s12 at 1080p, which are within the noise (the earlier run put s12 at
  1.18×, the one verdict that differs), and 1.3–3.6× on WebGL2.
  WebGL2 takes 1.2–1.9× WebGPU's wall at that look. One known part is the
  extra depth pass; WebGL2 has no pass timing to split the rest.

WebGL2 is therefore the next performance round's target: the like-for-like
losses in s5 and in s12 at phone size, and the medium look's 1.3–3.6×.

CPU per frame, milliseconds (three.js's is the JavaScript of its render
call; the engine's is JavaScript and wasm):

| Buffer | Room | Look | three.js JS | WebGPU burst / paced | WebGL2 burst / paced |
|---|---|---|---|---|---|
| 1920×1080 | s11 | lfl | 0.06 | 0.06 / 0.2 | 0.56 / 0.3 |
| 1920×1080 | s11 | medium | 0.06 | 0.06 / 0.2 | 1.34 / 0.4 |
| 1920×1080 | s12 | lfl | 0.09 | 0.06 / 0.2 | 0.49 / 0.4 |
| 1920×1080 | s12 | medium | 0.09 | 0.08 / 0.2 | 1.77 / 0.4 |
| 1920×1080 | s5 | lfl | 0.12 | 0.06 / 0.2 | 0.47 / 0.3 |
| 1920×1080 | s5 | medium | 0.12 | 0.06 / 0.2 | 1.63 / 0.4 |
| 1266×585 | s11 | lfl | 0.07 | 0.07 / 0.2 | 0.27 / 0.4 |
| 1266×585 | s11 | medium | 0.07 | 0.07 / 0.2 | 0.55 / 0.4 |
| 1266×585 | s12 | lfl | 0.07 | 0.07 / 0.2 | 0.27 / 0.3 |
| 1266×585 | s12 | medium | 0.07 | 0.07 / 0.2 | 0.68 / 0.4 |
| 1266×585 | s5 | lfl | 0.12 | 0.07 / 0.2 | 0.20 / 0.4 |
| 1266×585 | s5 | medium | 0.12 | 0.06 / 0.2 | 0.63 / 0.4 |

Burst CPU on WebGL2 counts the waits for command buffers that a 300-draw
burst runs into, so the paced CPU, a median over 120 frames drawn from
`requestAnimationFrame` with `frame(now)`, is the main thread's real cost
per frame. `performance.now()` is coarsened to 0.1 ms on pages that are
not cross-origin isolated, so it reads 0.2 ms on WebGPU and 0.3–0.4 ms on
WebGL2. Reproduce one cell with `test/bench.mjs` (above); the comparison
itself needs the game's sources and runs locally only.

**Size or speed.** `npm run build -- --speed` (opt-level 3, `wasm-opt
-O3`) gave 4,831,327 bytes (1,374,445 brotli) against the size build's
4,038,837 (1,243,115), both at the same commit. Interleaved in one session, three repetitions, its wall time
per frame was within each pair's own spread of the size build's on every
room and backend tried (s5 lfl, s11 and s12 medium, both backends), so the
size build is the default.

## What was taken from other engines

- **Bevy** runs one renderer on WebGPU and WebGL2 and decides from the
  device's limits at startup whether clustered lights go in storage or
  uniform buffers; Sariema decides its whole path the same way, once.
  Bevy also turns on wgpu's `fragile-send-sync-non-atomic-wasm` for its
  single-threaded web builds, which lets the renderer's `Arc<Device>` stand
  on the web unchanged.
- **Filament**, written for OpenGL ES 3.0, keeps its per-froxel light
  lists in an integer texture and its light records in a uniform buffer
  held to the 16 KiB minimum. The downlevel path's integer data textures
  for clusters and indices, and its 16 KiB actor uniform, follow that.
- **three.js** reads skinning matrices and batched transforms from data
  textures by index (`boneTexture`, `BatchedMesh`), draws without a base
  vertex, and picks a compressed texture format per device from the
  extensions it finds (`KTX2Loader`). The lights and shadows tables as
  texture rows read by index, indices baked per mesh, and the per-device
  pack variant come from there. Its transcoding step does not: a room
  ships as two packs, ASTC and RGBA8, and the page fetches the one the
  device samples, so the loader stays a plain fetch and the `.wasm` holds
  no Basis transcoder. The cost is disk on the host (the RGBA8 base-s11 is
  61 MB raw, 9.6 MB brotli) and, on a GPU without ASTC, four times the
  texture memory; BC and ETC2 variants are not built.
