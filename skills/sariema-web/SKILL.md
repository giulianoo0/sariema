---
name: sariema-web
description: Use when shipping a Sariema game to a web page or a React Native or native app WebView, building the engine to WebAssembly, when a game plays natively but not in a browser or its hashes differ there, a canvas stays black on WebGL2, the engine fails with E_GPU on a browser, a phone shows no WebGPU or plays no sound, a pack loads on desktop but not on a phone, or the browser tests or the web frame-time bench must be run.
---

# Sariema on the web

## Overview

The engine compiles to one `.wasm` that `packages/sariema-web` (`@sariema/web`)
mounts on a canvas: WebGPU where the browser has it, WebGL2 elsewhere, chosen
once. A game ships as a folder `sariema web` writes: the game pack in two
texture variants (`game.srmg` ASTC, `game.rgba8.srmg`) and
`sariema-web.json`; `loadGame` fetches the variant the device samples and
plays it whole (logic under wasmi, collision, animation, sound, HUD), to
the same `engine_hash` as `sariema run --headless`. A project without a
game section gets its rooms and actors instead, shown on the Stage. Full
reference: `docs/web.md`.

Core rule: **prove it in the browsers, not just natively.** WebGL2 runs
shaders as GLSL ES 3.00 and has GL-only failure modes, and a page has a
smaller stack and a coarser clock; `npm test` plays the climb and the
guide (particles, a trail, a highlight) in Chromium and WebKit, on WebGPU
and on WebGL2, against the native hashes (`fx_hash` included) and frames.

## Quick reference

From the repository root; `sariema` is `cargo run -q -p sariema-cli --`.

```sh
# Once: the bindgen CLI at the workspace's wasm-bindgen version, and browsers
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cd packages/sariema-web && npm install && npx playwright install chromium webkit && cd ../..

# The engine, package, probe page and demo game (the climb) into packages/sariema-web/dist
(cd packages/sariema-web && npm run build)      # prints the .wasm's sizes

# A game: new project, its web folder, then play it in a browser
sariema new games/meadow --toolchain 1.100.0 --json
sariema web games/meadow -o packages/sariema-web/dist/meadow --json   # game.srmg, game.rgba8.srmg
(cd packages/sariema-web && node test/bench.mjs --game dist/meadow \
  --log ../../games/meadow/input/walk.json --steps 240 --reps 1)       # prints "replay" hashes
sariema run --headless --game packages/sariema-web/dist/meadow/game.srmg \
  --input games/meadow/input/walk.json --steps 240 --json              # the same engine_hash

# Browser tests (about a minute); add your rooms with SARIEMA_GAME_PACKS=<dir>
(cd packages/sariema-web && npm test)
(cd packages/sariema-web && SARIEMA_BROWSERS=chromium-webgl2 npm test)

# Native checks of the WebGL2 path, including the GLSL ES 3.00 translation
cargo test -p sariema-render
cargo test -p sariema-web

# Frame time of a game or one room, WebGL2, phone-sized (--browser webkit for WebKit)
(cd packages/sariema-web && node test/bench.mjs --game dist/demo --backend webgl2 --viewport 390,844,3 --reps 3)
(cd packages/sariema-web && node test/bench.mjs --pack /abs/room.srma --backend webgl2 --viewport 844,390,3 --reps 3)
# Each frame's step, CPU, time since the last frame, passes and draws from
# the first frame on, to see what a moment (a room switch) costs
sariema web examples/rooms -o packages/sariema-web/dist/rooms
(cd packages/sariema-web && node test/bench.mjs --game dist/rooms --backend webgl2 --timeline 800 --reps 1)

# Serve dist/ (and the probe) locally; see "A phone" below for HTTPS
(cd packages/sariema-web && npm run serve)      # http://127.0.0.1:8787/dist/probe/
```

## Worked example: the climb on a page

`npm run build` puts the climb in `dist/demo/`. A page beside `dist/`:

```html
<canvas id="game" style="display:block;width:100vw;height:100vh"></canvas>
<script type="module">
  import { mount } from "./dist/index.js";
  const sariema = await mount(document.getElementById("game"));
  await sariema.loadGame("./dist/demo/");   // picks game.srmg or game.rgba8.srmg and plays it
  sariema.on("error", (e) => console.error(e.code, e.message));   // E_GAME_BUDGET, E_GAME_TRAP, ...
  console.log(sariema.backend, sariema.capabilities().path, sariema.stats().game);
</script>
```

Arrows or WASD walk the hero, the left half of the screen is the HUD's
joystick to a finger or the mouse, the action button (or E) waves; the music
starts on the first tap. To check a game against its native run in a
page, play it with `{ audio: false }` and call `replayGame(log, steps)`,
which returns the hashes `run --headless` prints.

Serve the folder holding the page and `dist/` (`npm run serve` serves the
package directory). The canvas is sized by CSS; the engine sets its pixels
from `devicePixelRatio`, capped at `maxPixelRatio` (1.5).

The game's messages to its host (`save`, `load_saves`, `ready`, `exit`,
`message`...) arrive as `sariema.on("host", m => ...)`. The package keeps
no saves: keep each `save` yourself, and to answer the game's loads listen
for `saveLoad` and call `sariema.answerSaves({ key: bytes, ... })`, then or
later. Unanswered after 5 s of game time, or with nothing listening for
`saveLoad`, a load is answered as no save, so the game never waits forever.
`docs/web.md` (Host messages and saves) has an example.

In a React Native WebView, forward events with
`sariema.on("ui", (e) => window.ReactNativeWebView.postMessage(JSON.stringify(e)))`
and the `host` ones likewise (their bytes as arrays), answer loads from the
app through `injectJavaScript`, and set `mediaPlaybackRequiresUserAction={false}` and
`allowsInlineMediaPlayback`.

## A phone on the same network

Browsers withhold WebGPU and AudioWorklet from plain HTTP except on
`localhost`. Serve HTTPS:

```sh
cd packages/sariema-web
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=sariema-dev" \
  -addext "subjectAltName=DNS:localhost" -keyout key.pem -out cert.pem
npm run serve -- --host 0.0.0.0 --cert cert.pem --key key.pem
```

Open the printed `https://<address>:8787/dist/probe/` on the phone, accept
the warning, press "Run the smoke test" and "Copy JSON". Keep `key.pem` out
of the repository.

## Common failures

| Symptom | Cause | Fix |
|---|---|---|
| `error[E0463]: can't find crate for core` building for wasm | A `cargo` earlier on `PATH` (Homebrew) whose `rustc` has no wasm target | Build with `npm run build`, which uses the pinned toolchain's own cargo and rustc; or set `SARIEMA_CARGO` |
| wasm-bindgen: "rust Wasm file schema version" mismatch | CLI version ≠ the crate's `=0.2.130` | Reinstall the CLI at that version |
| `E_GPU: this browser cannot draw into [Rgba16Float, …]` | WebGL2 without `EXT_color_buffer_float` | No fallback: the device cannot run the engine. The probe's "Float render targets" says so up front |
| Black canvas on WebGL2, console `Validation Error … ShaderStages` | A shader GLSL ES 3.00 cannot take (texture level queries, depth `textureLoad`, uniform not a multiple of 16 bytes) | `cargo test -p sariema-render glsl_es` names the shader and the reason |
| WebGL2 frame lacks occlusion or depth effects that WebGPU has | A depth texture read without a sampler on wgpu's GL backend reads 0 | Read depth through a sampler; the post chain already does |
| `E_SCHEMA … joints and faces take N rows a frame; the downlevel path binds Quality::actor_uniform_rows` | An actor too big for WebGL2's 16 KiB uniform | Fewer joints or faces; `actor_uniform_rows` cannot pass `max_uniform_buffer_binding_size / 16` |
| A pack loads on desktop and fails on a phone | A fixed `.srma` path: ASTC on a GPU without it, or a 60 MB RGBA8 room where ASTC was possible | Load through `loadGame` / `enterRoom` / `pick`, which choose by `capabilities().texture_variants` |
| No sound | `AudioContext` suspended before a gesture, or not a secure context | `resumeAudio()` from a tap (the package also resumes on the first `pointerdown`); serve HTTPS |
| `navigator.gpu` undefined on the phone | Plain HTTP over the network | `--cert` / `--key` above |
| Playwright cannot find `chromium-1248` | Another Playwright's `install` removed it from the shared cache | `cd packages/sariema-web && npx playwright install chromium` (Playwright 1.64.1 pins revision 1248) |
| A WebGL2 render differs from the native golden by more than 6 | Expected only at silhouettes (single-sample depth for post) and below 0.5% of pixels | Compare with `test/out/renders/<browser>/`; beyond 0.5% it is a bug |
| `RangeError: Maximum call stack size exceeded`, then "recursive use of an object" on every call | wasmi built without `auto-dispatch` (or a fork of its features) takes its tail-call dispatch on wasm32 and recurses once per instruction | Keep `auto-dispatch` in the workspace's `wasmi` features; `npm test`'s never-returning game catches it |
| The page freezes for half a second, then `error` with `E_GAME_BUDGET` | The logic ran out of fuel; the first tick has 200 M, later ones 20 M | Find the loop in the logic (`sariema run --headless` stops at the same step); the budget is `vm.fuel_first_step` and `vm.fuel_per_step` in the game's settings |
| A game's `engine_hash` in the page differs from `run --headless` | Not the same pack, log, steps or seed (default 0), or the sound played live | Replay with `{ audio: false }` from a fresh `playGame`; same files; `audio_hash` alone may differ live |

## Before saying it works

- `npm test` passes with every browser, and with your rooms in
  `SARIEMA_GAME_PACKS`.
- Your game's `replay` hashes from `test/bench.mjs --game` equal
  `sariema run --headless` for the same log and steps, in Chromium and in
  WebKit (`--browser webkit`).
- The probe page's smoke test passes on the real phone, in the app's own
  WebView, over HTTPS.
- `sariema web` was rerun after the last change to the logic, a room or an actor.
