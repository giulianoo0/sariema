// SPDX-License-Identifier: Apache-2.0

// Builds dist/: the engine compiled to WebAssembly, its JavaScript bindings,
// the shrunk .wasm, the typed API, the audio worklet, the probe page and the
// demo game it plays (examples/climb). Prints the .wasm's sizes as JSON.
//
//   node scripts/build.mjs [--speed] [--debug] [--no-opt]
//   node scripts/build.mjs --alloc-count
//
// The default optimises for size; --speed for speed (opt-level 3, wasm-opt
// -O3), a larger download for less CPU a frame. --alloc-count adds to an
// existing dist/ the .wasm whose allocator counts, for the tests:
// dist/wasm/sariema_web_counting_bg.wasm, which takes the same bindings.
//
// CARGO_TARGET_DIR is honoured; SARIEMA_CARGO names a cargo to use instead
// of the repository's pinned toolchain.

import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";
import { cargoHome, remapFlags, rustFlags } from "./remap.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, "..");
const repo = resolve(pkg, "../..");
const dist = join(pkg, "dist");
const args = new Set(process.argv.slice(2));
const debug = args.has("--debug");
const optimise = !debug && !args.has("--no-opt");
const speed = args.has("--speed");
const counting = args.has("--alloc-count");

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
const remap = rustFlags(remapFlags(repo, cargoHome()));
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(repo, "target");
const profile = debug ? "dev" : "release";
// Set as environment overrides so the workspace's own release profile stays
// as it is.
const sizeProfile = debug
  ? {}
  : {
      CARGO_PROFILE_RELEASE_OPT_LEVEL: speed ? "3" : "s",
      CARGO_PROFILE_RELEASE_LTO: "true",
      CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "1",
      CARGO_PROFILE_RELEASE_PANIC: "abort",
      CARGO_PROFILE_RELEASE_STRIP: "debuginfo",
    };
const features = counting ? ["--features", "alloc-count"] : [];
run(cargo, ["build", "-p", "sariema-web", "--lib", "--target", "wasm32-unknown-unknown", "--profile", profile, ...features, ...remap.args], {
  ...env,
  ...remap.env,
  ...sizeProfile,
  CARGO_TARGET_DIR: target,
});

const built = join(target, "wasm32-unknown-unknown", debug ? "debug" : "release", "sariema_web.wasm");
const bindgen = process.env.WASM_BINDGEN ?? join(homedir(), ".cargo", "bin", "wasm-bindgen");
const bind = (outDir) =>
  run(existsSync(bindgen) ? bindgen : "wasm-bindgen", [
    "--target",
    "web",
    "--out-dir",
    outDir,
    "--out-name",
    "sariema_web",
    ...(debug ? ["--debug", "--keep-debug"] : []),
    built,
  ]);

if (counting) {
  const glue = join(dist, "wasm", "sariema_web.js");
  if (!existsSync(glue)) throw new Error("dist/ is missing: run the build without --alloc-count first");
  const tmp = join(target, "sariema-web-counting");
  bind(tmp);
  const same = readFileSync(join(tmp, "sariema_web.js"), "utf8") === readFileSync(glue, "utf8");
  copyFileSync(join(tmp, "sariema_web_bg.wasm"), join(dist, "wasm", "sariema_web_counting_bg.wasm"));
  rmSync(tmp, { recursive: true, force: true });
  if (!same) throw new Error("the counting build's bindings differ from dist/'s: rebuild dist/ with the same flags");
  console.log(JSON.stringify({ wasm: "dist/wasm/sariema_web_counting_bg.wasm" }));
  process.exit(0);
}

rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, "wasm"), { recursive: true });
bind(join(dist, "wasm"));

const wasm = join(dist, "wasm", "sariema_web_bg.wasm");
const bindgenBytes = statSync(wasm).size;
if (optimise) {
  run(join(pkg, "node_modules", ".bin", "wasm-opt"), [
    speed ? "-O3" : "-Oz",
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
cli("web", join(repo, "examples", "climb"), "-o", join(dist, "demo"));

const bytes = readFileSync(wasm);
const sizes = {
  wasm: wasm.slice(pkg.length + 1),
  profile,
  for: speed ? "speed" : "size",
  wasm_opt: optimise,
  bindgen_bytes: bindgenBytes,
  bytes: bytes.length,
  gzip_bytes: gzipSync(bytes, { level: 9 }).length,
  brotli_bytes: brotliCompressSync(bytes, {
    params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length },
  }).length,
};
console.log(JSON.stringify(sizes));
