// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { test } from "node:test";
import { localDirs, namedDirs, remapFlags, rustFlags } from "../scripts/remap.mjs";

const flags = remapFlags("/src/sariema", "/opt/cargo");
const remap = ["--remap-path-prefix=/src/sariema=/sariema", "--remap-path-prefix=/opt/cargo/registry/src=/cargo"];

test("the repository and a cargo home outside home are remapped", () => {
  assert.deepEqual(flags, remap);
});

test("without user flags the remap joins cargo's config", () => {
  assert.deepEqual(rustFlags(flags, {}), { args: ["--config", `build.rustflags=${JSON.stringify(remap)}`], env: {} });
});

test("RUSTFLAGS and the target's rustflags come first and stay", () => {
  const want = { args: [], env: { CARGO_ENCODED_RUSTFLAGS: ["-Cfoo", "-Cbar", ...remap].join("\x1f") } };
  assert.deepEqual(rustFlags(flags, { RUSTFLAGS: " -Cfoo  -Cbar " }), want);
  assert.deepEqual(rustFlags(flags, { CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS: "-Cfoo -Cbar" }), want);
});

test("CARGO_ENCODED_RUSTFLAGS keeps its spaces and wins over RUSTFLAGS", () => {
  assert.deepEqual(rustFlags(flags, { CARGO_ENCODED_RUSTFLAGS: "-L\x1f/a b", RUSTFLAGS: "-Cignored" }), {
    args: [],
    env: { CARGO_ENCODED_RUSTFLAGS: ["-L", "/a b", ...remap].join("\x1f") },
  });
  assert.deepEqual(rustFlags(flags, { CARGO_ENCODED_RUSTFLAGS: "" }).env.CARGO_ENCODED_RUSTFLAGS, remap.join("\x1f"));
});

test("a local directory in the bytes is found", () => {
  const dirs = localDirs("/src/sariema", { CARGO_HOME: "/opt/cargo" });
  assert.ok(dirs.includes("/opt/cargo"));
  assert.deepEqual(namedDirs(Buffer.from("/cargo/x.rs /sariema/y.rs"), dirs), []);
  assert.deepEqual(namedDirs(Buffer.from("at /opt/cargo/registry/src/a.rs"), ["/opt/cargo"]), ["/opt/cargo"]);
});
