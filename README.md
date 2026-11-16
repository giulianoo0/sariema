# Sariema

An open-source native game engine written in Rust on `wgpu`, built to be
embedded in apps and to run games downloaded as data packages. The design is
in [docs/design.md](docs/design.md).

Status: early. A room can be imported and rendered headless on desktop with
clustered forward lighting, shadows and the post-processing chain, driven by
the `sariema` command-line tool.

- `crates/sariema-format` — the room pack (`SRMA`): types, encoding, validation.
- `crates/sariema-import` — packaging-time import: compressed glTF kits and room
  assembly from placement and lighting files (`schemas/room.schema.json`).
- `crates/sariema-core` — the room camera.
- `crates/sariema-render` — the wgpu renderer: clustered forward scene pass,
  shadow atlas, post-processing chain, quality tiers, headless rendering and
  image comparison.
- `crates/sariema-audio` — the mixer: buses, streamed music, ducking, loudness
  checks and offline rendering of cue lists ([docs/audio.md](docs/audio.md)).
- `crates/sariema-ui` — in-engine UI: flexbox tree, distance-field text, animation, touch widgets and a batched draw list ([docs/ui.md](docs/ui.md)).
- `crates/sariema-cli` — the `sariema` tool: `build`, `inspect`, `run --headless`
  and `test`, each with `--json`. `examples/demo` is a project it can build.

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
