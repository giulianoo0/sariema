// SPDX-License-Identifier: Apache-2.0

// Builds dist/: the engine compiled to WebAssembly, its JavaScript bindings,
// the shrunk .wasm, the typed API, the audio worklet, the probe page and the
// demo game it draws. Prints the .wasm's sizes as JSON.
//
//   node scripts/build.mjs [--debug] [--no-opt]
//
// CARGO_TARGET_DIR is honoured; SARIEMA_CARGO names a cargo to use instead
// of the repository's pinned toolchain.

import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, "..");
const repo = resolve(pkg, "../..");
const dist = join(pkg, "dist");
const args = new Set(process.argv.slice(2));
const debug = args.has("--debug");
const optimise = !debug && !args.has("--no-opt");

function run(cmd, argv, env = {}) {
  execFileSync(cmd, argv, { stdio: ["ignore", "inherit", "inherit"], cwd: repo, env: { ...process.env, ...env } });
}

/** The pinned toolchain's own cargo and rustc, so a cargo earlier on PATH cannot pick a rustc without the wasm target. */
function toolchain() {
  if (process.env.SARIEMA_CARGO) return { cargo: process.env.SARIEMA_CARGO, env: {} };
  const channel = /channel\s*=\s*"([^"]+)"/.exec(readFileSync(join(repo, "rust-toolchain.toml"), "utf8"))?.[1];
  const root = join(process.env.RUSTUP_HOME ?? join(homedir(), ".rustup"), "toolchains");
  const name = channel && existsSync(root) && readdirSync(root).find((d) => d.startsWith(`${channel}-`));
  if (!name) return { cargo: "cargo", env: {} };
  const bin = join(root, name, "bin");
  return { cargo: join(bin, "cargo"), env: { RUSTC: join(bin, "rustc"), PATH: `${bin}:${process.env.PATH}` } };
}

const { cargo, env } = toolchain();
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(repo, "target");
const profile = debug ? "dev" : "release";
// Size first: the page downloads the binary before its first frame. Set as
// environment overrides so the workspace's own release profile stays as it is.
const sizeProfile = debug
  ? {}
  : {
      CARGO_PROFILE_RELEASE_OPT_LEVEL: process.env.CARGO_PROFILE_RELEASE_OPT_LEVEL ?? "s",
      CARGO_PROFILE_RELEASE_LTO: "true",
      CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "1",
      CARGO_PROFILE_RELEASE_PANIC: "abort",
      CARGO_PROFILE_RELEASE_STRIP: "debuginfo",
    };
run(cargo, ["build", "-p", "sariema-web", "--lib", "--target", "wasm32-unknown-unknown", "--profile", profile], {
  ...env,
  ...sizeProfile,
  CARGO_TARGET_DIR: target,
});

rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, "wasm"), { recursive: true });
const built = join(target, "wasm32-unknown-unknown", debug ? "debug" : "release", "sariema_web.wasm");
const bindgen = process.env.WASM_BINDGEN ?? join(homedir(), ".cargo", "bin", "wasm-bindgen");
run(existsSync(bindgen) ? bindgen : "wasm-bindgen", [
  "--target",
  "web",
  "--out-dir",
  join(dist, "wasm"),
  "--out-name",
  "sariema_web",
  ...(debug ? ["--debug", "--keep-debug"] : []),
  built,
]);

const wasm = join(dist, "wasm", "sariema_web_bg.wasm");
const bindgenBytes = statSync(wasm).size;
if (optimise) {
  run(join(pkg, "node_modules", ".bin", "wasm-opt"), [
    "-Oz",
    "--enable-bulk-memory",
    "--enable-nontrapping-float-to-int",
    "--enable-sign-ext",
    "--enable-mutable-globals",
    "--enable-reference-types",
    "--enable-multivalue",
    wasm,
    "-o",
    wasm,
  ]);
}

run(join(pkg, "node_modules", ".bin", "tsc"), ["-p", join(pkg, "tsconfig.json")]);
copyFileSync(join(pkg, "src", "audio-worklet.js"), join(dist, "audio-worklet.js"));
cpSync(join(pkg, "probe"), join(dist, "probe"), { recursive: true });
const cli = (...argv) =>
  run("cargo", ["run", "-q", "-p", "sariema-cli", "--release", "--", ...argv, "--json"], { CARGO_TARGET_DIR: target });
cli("web", join(repo, "examples", "demo"), "-o", join(dist, "demo"));
cli("audio", "build", join(repo, "examples", "audio"), "-o", join(dist, "demo", "sounds.srms"));

const bytes = readFileSync(wasm);
const sizes = {
  wasm: wasm.slice(pkg.length + 1),
  profile,
  wasm_opt: optimise,
  bindgen_bytes: bindgenBytes,
  bytes: bytes.length,
  gzip_bytes: gzipSync(bytes, { level: 9 }).length,
  brotli_bytes: brotliCompressSync(bytes, {
    params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length },
  }).length,
};
console.log(JSON.stringify(sizes));
