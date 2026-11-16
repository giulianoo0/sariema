// SPDX-License-Identifier: Apache-2.0

// Builds the landing's Rust snippet as a game's logic: `sariema new`, the
// snippet as its lib.rs, `sariema build`. Needs SARIEMA_CLI to name a
// sariema binary (target/release/sariema) and the pinned toolchain's
// wasm32-unknown-unknown target.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { rust } from "../app/(home)/snippets.ts";

const cli = process.env.SARIEMA_CLI;
const toolchain = /channel\s*=\s*"([^"]+)"/.exec(readFileSync(new URL("../../rust-toolchain.toml", import.meta.url), "utf8"))[1];

test("the Rust snippet builds as a game", { skip: !cli && "set SARIEMA_CLI to a sariema binary" }, () => {
  const dir = mkdtempSync(join(tmpdir(), "sariema-snippet-"));
  try {
    const project = join(dir, "game");
    execFileSync(cli, ["new", project, "--toolchain", toolchain, "--json"]);
    writeFileSync(join(project, "game", "src", "lib.rs"), rust);
    let out;
    try {
      out = execFileSync(cli, ["build", project, "-o", join(dir, "game.srmg"), "--json"], { encoding: "utf8" });
    } catch (e) {
      out = e.stdout;
    }
    const report = JSON.parse(out);
    assert.equal(report.ok, true, report.error?.message);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
