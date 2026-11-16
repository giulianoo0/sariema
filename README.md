<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/lockup-dark.svg">
    <img src="assets/brand/lockup.svg" alt="Sariema" height="64">
  </picture>
</p>

<p align="center">The game engine that fits inside your app: Rust on wgpu, games shipped as data packs with sandboxed WebAssembly logic.</p>

<p align="center">
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-B4532A"></a>
  <img alt="Desktop" src="https://img.shields.io/badge/desktop-wgpu-4F6B3A">
  <img alt="Web: WebGPU and WebGL2" src="https://img.shields.io/badge/web-WebGPU%20%7C%20WebGL2-4F6B3A">
  <img alt="React Native: WebView" src="https://img.shields.io/badge/React%20Native-WebView-9AA284">
</p>

Status: early. A game steps at a fixed 1/60 s to the same `engine_hash` on
desktop and in the browser: rendering, collision, animation, sound, HUD and
cutscenes, with the logic as WebAssembly under a fuel budget. The design is
in [docs/design.md](docs/design.md).

## Quick start

### The CLI

The `sariema` tool makes, builds, runs and checks games, each command with
`--json`. Game logic compiles to WebAssembly with the toolchain pinned in
`rust-toolchain.toml`.

```sh
git clone https://github.com/giulianoo0/sariema && cd sariema
rustup target add wasm32-unknown-unknown
cargo build --release -p sariema-cli
export PATH="$PWD/target/release:$PATH"

sariema new my-game --toolchain 1.100.0
sariema build my-game -o my-game.srmg
sariema run --game my-game.srmg                  # in a window
sariema run --game my-game.srmg --headless --steps 120 --screenshot shot.png
```

### The web

`packages/sariema-web` is the engine compiled to WebAssembly with a typed
API, drawing on WebGPU or WebGL2. It needs `wasm-bindgen-cli` at the
workspace's version and Node.

```sh
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cd packages/sariema-web
npm install
npm run build      # dist/: the engine, its API and examples/climb
npm run serve      # http://127.0.0.1:8787/dist/probe/ plays the climb
```

```ts
import { mount } from "@sariema/web";

const sariema = await mount(canvas, { quality: "medium" });
await sariema.loadGame("/games/climb/"); // a folder written by `sariema web`
```

### React Native

Today an app runs the web package inside `react-native-webview`, pointed
at a page that mounts it; [docs/web.md](docs/web.md#webview-notes) has the
WebView settings and how events and saves cross to the app. A native
Fabric component, with no WebView, is the design's goal.

## Docs

The site in `site/` (`npm install && npm run dev` there) serves
[docs/](docs/) as they are. Start with [the engine](docs/engine.md) and
[the game API](docs/game-api.md); the rest covers
[physics](docs/physics.md), [actors](docs/actors.md),
[cutscenes](docs/cutscenes.md), [audio](docs/audio.md), [UI](docs/ui.md),
[the web](docs/web.md) and [performance](docs/performance.md). The logo,
colours and their rules are in [docs/brand.md](docs/brand.md).

For an agent building a game, the skills are in [skills/](skills/), starting
with [skills/sariema](skills/sariema/SKILL.md).

## Developing

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
