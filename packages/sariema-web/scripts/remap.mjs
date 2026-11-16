// SPDX-License-Identifier: Apache-2.0

// The --remap-path-prefix flags of the engine's .wasm, the same as
// `sariema build` passes for a game's: the repository becomes /sariema and
// cargo's registry sources /cargo, so the .wasm names no local directory.

import { realpathSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/** Each directory canonical, then as given when that differs. */
function bothForms(dir) {
  let real = dir;
  try {
    real = realpathSync(dir);
  } catch {}
  return real === dir ? [dir] : [real, dir];
}

export function cargoHome(env = process.env) {
  return env.CARGO_HOME ?? join(homedir(), ".cargo");
}

export function remapFlags(repo, cargoHomeDir) {
  return [
    ...bothForms(repo).map((d) => `--remap-path-prefix=${d}=/sariema`),
    ...bothForms(join(cargoHomeDir, "registry", "src")).map((d) => `--remap-path-prefix=${d}=/cargo`),
  ];
}

const TARGET = "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS";

/**
 * How `flags` reach rustc without dropping one the user set: appended to
 * CARGO_ENCODED_RUSTFLAGS, RUSTFLAGS or the target's rustflags in the
 * environment, in the order cargo prefers them, or else a --config that
 * cargo joins to build.rustflags in its config files.
 */
export function rustFlags(flags, env = process.env) {
  const { CARGO_ENCODED_RUSTFLAGS: encoded, RUSTFLAGS: plain, [TARGET]: target } = env;
  let user;
  if (encoded !== undefined) user = encoded.split("\x1f");
  else if (plain !== undefined) user = plain.split(/\s+/);
  else if (target !== undefined) user = target.split(/\s+/);
  else return { args: ["--config", `build.rustflags=${JSON.stringify(flags)}`], env: {} };
  return { args: [], env: { CARGO_ENCODED_RUSTFLAGS: [...user.filter(Boolean), ...flags].join("\x1f") } };
}

/** The local directories a built file must not name. */
export function localDirs(repo, env = process.env) {
  return [...bothForms(repo), ...bothForms(cargoHome(env)), ...bothForms(homedir())].filter((d) => d.length > 1);
}

/** Which of `dirs` the bytes name. */
export function namedDirs(bytes, dirs) {
  const buf = Buffer.from(bytes);
  return dirs.filter((d) => buf.includes(d));
}
