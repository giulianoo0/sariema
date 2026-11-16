# The web and app WebViews

The engine runs in a browser tab and in a React Native WebView as one
WebAssembly module. WebGPU draws where the browser has it; WebGL2, which
every target browser has, draws everywhere else. This page covers the
model, the two render paths and what each WebGL2 gap costs, the settings,
textures, audio, the commands to build, test, measure and serve, WebView
notes, and what was taken from Bevy, Filament and three.js.

## Model

```
page (JS)                                   engine (wasm, crates/sariema-web)
─────────────────────────────────────────   ─────────────────────────────────────
@sariema/web  mount(canvas, options)  ───►  Engine.create: WebGPU or WebGL2,
  requestAnimationFrame(t) ──────────────►    the render path, the Stage
                                              Engine.frame(t): fixed steps due,
  pointer / keyboard events ─────────────►      UI events, draw interpolated
  ResizeObserver, devicePixelRatio ──────►    Engine.resize
  visibilitychange: stop, suspend audio
  webglcontextlost / device lost ─────────►  frame() returns −1: make a new
                                              Engine, replay every load
  AudioWorklet reports frames played ────►  Engine.mix: blocks due, posted
                                              back to the worklet
  fetch packs (streamed into one buffer)  ►  load_room / load_actor / sounds
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
- **Game runtime seam.** `sariema_web::game::Game` is what a game's logic
  implements: `step(step_s, input)` once per fixed step, with that step's
  input. `Stage::set_game` installs it. The game runtime compiles to its
  own WebAssembly module that the browser's engine runs; it drives the
  stage through the same calls the page makes (rooms, actors, sounds, UI).
  This seam is documented, not implemented: no game runs on the web yet.

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

Browsers start an `AudioContext` suspended until a user gesture: the
package resumes it on the first `pointerdown`, or call `resumeAudio()` from
your own button. AudioWorklet, like WebGPU, exists only in a secure context
(HTTPS or `localhost`). A hidden page suspends the audio with the loop.

## The package

```ts
import { mount } from "@sariema/web";

const sariema = await mount(canvas, { quality: "medium" });
await sariema.loadGame("/games/origem/");      // sariema-web.json, start room, sounds, UI
const model = await sariema.loadGameActor("tito");
const tito = sariema.spawnActor(model);
sariema.setActor(tito, { transform, clip: "walk", speed: 1 });
sariema.on("ui", (e) => console.log(e.type, e.node));
sariema.on("lost", () => console.log("rebuilding"));
```

`start()` / `stop()` run and stop the frame loop, or `frame(now)` runs one
frame from a loop the page already has; `capture()` reads a frame back;
`bench(n)` times n draws; `stats()` gives the last frame's passes, draws,
uploads and steps; `destroy()` lets go of the GPU and audio. On a lost
WebGL context or WebGPU device the package makes a new engine on the same
canvas and replays every load in order, emitting `lost` and `restored`, or
`error` if the rebuild fails (for instance a pack that can no longer be
fetched). Packs are fetched as they stream in; a Content-Length is only a
size hint, since a compressed response from another origin shows its
compressed length and hides its encoding.

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
                   # probe page and the demo game; prints the .wasm's sizes
                   # (--speed: opt-level 3 and wasm-opt -O3 instead)
npm test           # test/run.mjs, about 20 s
```

`npm test` writes the fixtures and native references with `cargo run
--example fixtures`, builds `examples/demo` with `sariema web`, serves them,
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
load. The tolerance is the
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
CPU and wall time, with the load average.

Natively, `cargo test -p sariema-render` covers the downlevel path: its
goldens within tolerance, limit detection, data-texture round trips,
skinning through the uniform fallback, and the GLSL ES 3.00 translation of
every shader. `cargo test -p sariema-web` runs the stage on a native device.

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
smoke test mounts the engine, draws the demo game, times the first frame
and plays a chime through the worklet.

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
  `sariema.on("ui", (e) => window.ReactNativeWebView.postMessage(JSON.stringify(e)))`.
  A WebView in a tab that is not visible is hidden to the page, which
  pauses the loop and the audio.

## Numbers

Apple M5, Chromium 156 (Playwright's chromium-1248, ANGLE on Metal),
headless; the load average is given with each set.

**The `.wasm`** (`npm run build`, cargo `opt-level = "s"`, LTO, one
codegen unit, `panic = "abort"`, then `wasm-opt -Oz`):

| Stage | Bytes |
|---|---|
| wasm-bindgen output | 6,699,337 |
| after wasm-opt | 4,040,137 |
| gzip -9 | 1,644,544 |
| brotli 11 | 1,243,013 |

**Time to first frame** of an Origem room at the medium look, a phone-sized
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
