# Sariema

An open-source native game engine written in Rust on `wgpu`, built to be
embedded in apps and to run games downloaded as data packages. The design is
in [docs/design.md](docs/design.md).

Status: early. The room pack format, the glTF kit importer and room assembly
exist; the renderer and the CLI are next.

- `crates/sariema-format` — the room pack (`SRMA`): types, encoding, validation.
- `crates/sariema-import` — packaging-time import: compressed glTF kits and room
  assembly from placement and lighting files (`schemas/room.schema.json`).

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Licensed under either of MIT (`LICENSE-MIT`) or Apache-2.0 (`LICENSE-APACHE`),
at your option.
