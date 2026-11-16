# Sariema

An open-source native game engine written in Rust on `wgpu`, built to be
embedded in apps and to run games downloaded as data packages. The design is
in [docs/design.md](docs/design.md).

Status: early. A room can be imported and rendered headless on desktop with
clustered forward lighting; shadows, post-processing and the CLI are next.

- `crates/sariema-format` — the room pack (`SRMA`): types, encoding, validation.
- `crates/sariema-import` — packaging-time import: compressed glTF kits and room
  assembly from placement and lighting files (`schemas/room.schema.json`).
- `crates/sariema-core` — the room camera.
- `crates/sariema-render` — the wgpu renderer: clustered forward scene pass,
  quality tiers, headless rendering and image comparison.

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
