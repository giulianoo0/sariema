---
name: sariema-web
description: Use when shipping a Sariema game to a web page or a React Native or native app WebView, building the engine to WebAssembly, when a canvas stays black on WebGL2, the engine fails with E_GPU on a browser, a phone shows no WebGPU or plays no sound, a pack loads on desktop but not on a phone, or the browser tests or the web frame-time bench must be run.
---

# Sariema on the web

## Overview

The engine compiles to one `.wasm` that `packages/sariema-web` (`@sariema/web`)
mounts on a canvas: WebGPU where the browser has it, WebGL2 elsewhere, chosen
once. A game ships as a folder `sariema web` writes: every room and actor in
two texture variants (ASTC and RGBA8) and `sariema-web.json`; the page loads
the variant the device samples. Full reference: `docs/web.md`.

Core rule: **prove it in the browsers, not just natively.** WebGL2 runs
shaders as GLSL ES 3.00 and has GL-only failure modes; `npm test` draws the
reference scenes in Chromium and WebKit, on WebGPU and on WebGL2, against
the native frames.

## Quick reference

From the repository root; `sariema` is `cargo run -q -p sariema-cli --`.

```sh
# Once: the bindgen CLI at the workspace's wasm-bindgen version, and browsers
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cd packages/sariema-web && npm install && npx playwright install chromium webkit && cd ../..

# A game folder for the web (both texture variants, sound bank, manifest)
sariema web examples/demo -o packages/sariema-web/dist/demo --json

# The engine, package, probe page and demo game into packages/sariema-web/dist
(cd packages/sariema-web && npm run build)      # prints the .wasm's sizes

# Browser tests (about 20 s); add your rooms with SARIEMA_GAME_PACKS=<dir>
(cd packages/sariema-web && npm test)
(cd packages/sariema-web && SARIEMA_BROWSERS=chromium-webgl2 npm test)

# Native checks of the WebGL2 path, including the GLSL ES 3.00 translation
cargo test -p sariema-render
cargo test -p sariema-web

# Frame time of one room in Chromium, WebGL2, phone-sized
(cd packages/sariema-web && node test/bench.mjs --pack /abs/room.srma \
  --backend webgl2 --viewport 844,390,3 --reps 3)

# Serve dist/ (and the probe) locally; see "A phone" below for HTTPS
(cd packages/sariema-web && npm run serve)      # http://127.0.0.1:8787/dist/probe/
```

## Worked example: the demo game on a page

`npm run build` puts the demo game in `dist/demo/`. A page beside `dist/`:

```html
<canvas id="game" style="display:block;width:100vw;height:100vh"></canvas>
<script type="module">
  import { mount } from "./dist/index.js";
  const sariema = await mount(document.getElementById("game"));
  const game = await sariema.loadGame("./dist/demo/");   // picks demo.srma or demo.rgba8.srma
  const arm = sariema.spawnActor(await sariema.loadGameActor("arm"));
  sariema.setActor(arm, { clip: "bend", speed: 1 });
  sariema.on("ui", (e) => console.log(e.type, e.node));
  console.log(sariema.backend, sariema.capabilities().path, game.start);
</script>
```

Serve the folder holding the page and `dist/` (`npm run serve` serves the
package directory). The canvas is sized by CSS; the engine sets its pixels
from `devicePixelRatio`, capped at `maxPixelRatio` (1.5).

In a React Native WebView, forward events with
`sariema.on("ui", (e) => window.ReactNativeWebView.postMessage(JSON.stringify(e)))`
and set `mediaPlaybackRequiresUserAction={false}` and
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

## Before saying it works

- `npm test` passes with every browser, and with your rooms in
  `SARIEMA_GAME_PACKS`.
- The probe page's smoke test passes on the real phone, in the app's own
  WebView, over HTTPS.
- `sariema web` was rerun after the last change to a room or actor.
