# Sariema

An open-source native game engine written in Rust on `wgpu`, built to be
embedded in apps and to run games downloaded as data packages. The design is
in [docs/design.md](docs/design.md).

Status: early. A room can be imported and rendered headless on desktop with
clustered forward lighting, shadows and the post-processing chain, driven by
the `sariema` command-line tool.

- `crates/sariema-format` — the packs: room (`SRMA`), collision (`SRMC`),
  actor (`SRMK`) and sound bank (`SRMS`); types, encoding, validation.
- `crates/sariema-import` — packaging-time import: compressed glTF kits and room
  assembly from placement and lighting files (`schemas/room.schema.json`).
- `crates/sariema-core` — the room camera.
- `crates/sariema-physics` — deterministic character movement and collision, collision packs (`SRMC`) and `sariema check` ([docs/physics.md](docs/physics.md)).
- `crates/sariema-render` — the wgpu renderer: clustered forward scene pass,
  shadow atlas, post-processing chain, quality tiers, headless rendering and
  image comparison.
- `crates/sariema-anim` — skeletal animation: clip sampling, crossfades,
  events and skinning palettes for actor packs (`SRMK`, see
  [docs/actors.md](docs/actors.md)).
- `crates/sariema-audio` — the mixer: buses, streamed music, ducking, loudness
  checks and offline rendering of cue lists ([docs/audio.md](docs/audio.md)).
- `crates/sariema-ui` — in-engine UI: flexbox tree, distance-field text, animation, touch widgets and a batched draw list ([docs/ui.md](docs/ui.md)).
- `crates/sariema-web` and `packages/sariema-web` — the engine in a web page or app WebView: WebAssembly on WebGPU or WebGL2, the typed `@sariema/web` package, browser tests and a probe page ([docs/web.md](docs/web.md)).
- `crates/sariema-cli` — the `sariema` tool: `build`, `inspect`, `run`, `test`,
  `check`, `physics-map`, `bench`, `audio` and `ui`, each with `--json`.
  `examples/demo` is a project it can build.

Docs:

- [docs/design.md](docs/design.md) — the public design.
- [docs/physics.md](docs/physics.md) — character movement, collision packs and `sariema check`.
- [docs/actors.md](docs/actors.md) — actor packs, skeletal animation and skinned drawing.
- [docs/audio.md](docs/audio.md) — sound banks, the mixer and `sariema audio`.
- [docs/ui.md](docs/ui.md) — the in-engine UI and `sariema ui render`.
- [docs/performance.md](docs/performance.md) — `sariema bench` and what a frame costs.
- [docs/web.md](docs/web.md) — browsers and WebViews: render paths, textures, audio, building, testing and serving.

Agent skills, one per module, for an agent building a game with it:

- [skills/sariema-physics](skills/sariema-physics/SKILL.md)
- [skills/sariema-actors](skills/sariema-actors/SKILL.md)
- [skills/sariema-audio](skills/sariema-audio/SKILL.md)
- [skills/sariema-ui](skills/sariema-ui/SKILL.md)
- [skills/sariema-web](skills/sariema-web/SKILL.md)

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
