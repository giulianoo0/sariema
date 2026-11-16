// SPDX-License-Identifier: Apache-2.0

// The study behind docs/web.md's ruling on game logic in the browser, from
// the repository alone:
//
//   node tools/vm-bench/run.mjs
//
// Builds examples/climb's game pack, records its first 900 ticks over its
// input log and times them under native wasmi, injects gas into the logic
// modules, builds the bench's page wasm as the engine's web build is made
// (opt level s, LTO, wasm-opt -Oz), then runs the page in Chromium and
// WebKit. Prints the native timings and one JSON line per browser; the
// files go to tools/vm-bench/target/bench. Needs the web package's
// dependencies (npm install in packages/sariema-web) and wasm-bindgen.
//
//   SARIEMA_CHROMIUM   Chromium executable (default Playwright's chromium-1248)

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { homedir, loadavg } from "node:os";
import { dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");
const pkg = join(repo, "packages", "sariema-web");
const out = join(here, "target", "bench");
const data = join(out, "data");
mkdirSync(data, { recursive: true });

const run = (cmd, argv, options = {}) =>
  execFileSync(cmd, argv, { stdio: ["ignore", "pipe", "inherit"], ...options }).toString();

/** The pinned toolchain's own cargo and rustc, which have the wasm32 std. */
function toolchain() {
  const channel = /channel\s*=\s*"([^"]+)"/.exec(readFileSync(join(repo, "rust-toolchain.toml"), "utf8"))?.[1];
  const root = join(process.env.RUSTUP_HOME ?? join(homedir(), ".rustup"), "toolchains");
  const name = channel && existsSync(root) && readdirSync(root).find((d) => d.startsWith(`${channel}-`));
  if (!name) return { cargo: "cargo", env: process.env };
  const bin = join(root, name, "bin");
  return { cargo: join(bin, "cargo"), env: { ...process.env, RUSTC: join(bin, "rustc"), PATH: `${bin}:${process.env.PATH}` } };
}
const { cargo, env } = toolchain();

const pack = join(out, "climb.srmg");
run(cargo, ["run", "-q", "-p", "sariema-cli", "--release", "--", "build", join(repo, "examples", "climb"), "-o", pack, "--json"], { cwd: repo, env });
const bench = (bin, ...argv) => run(cargo, ["run", "-q", "--release", "--bin", bin, "--", ...argv], { cwd: here, env });
const native = JSON.parse(bench("record", pack, join(repo, "examples", "climb", "input", "play.json"), "900", data));
for (const name of ["climb", "synthetic", "forever"]) {
  native[`${name}_gas`] = JSON.parse(bench("inject", join(data, `${name}.wasm`), join(data, `${name}.gas.wasm`)));
}
console.log(JSON.stringify({ native }));

run(cargo, ["build", "-q", "--lib", "--release", "--target", "wasm32-unknown-unknown"], { cwd: here, env });
const bindgen = process.env.WASM_BINDGEN ?? join(homedir(), ".cargo", "bin", "wasm-bindgen");
const built = join(here, "target", "wasm32-unknown-unknown", "release", "vm_bench.wasm");
run(existsSync(bindgen) ? bindgen : "wasm-bindgen", ["--target", "web", "--out-dir", join(out, "a"), built]);
const a = join(out, "a", "vm_bench_bg.wasm");
run(join(pkg, "node_modules", ".bin", "wasm-opt"), ["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-mutable-globals", "--enable-reference-types", "--enable-multivalue", a, "-o", a]);

const { chromium, webkit } = createRequire(join(pkg, "package.json"))("playwright");
const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };
const server = createServer((req, res) => {
  const file = resolve(here, "." + decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (!file.startsWith(here + sep) || !existsSync(file)) return res.writeHead(404).end();
  res.writeHead(200, { "Content-Type": types[extname(file)] ?? "application/octet-stream" }).end(readFileSync(file));
});
const base = await new Promise((ok) => server.listen(0, "127.0.0.1", () => ok(`http://127.0.0.1:${server.address().port}/`)));
const chromiumPath =
  process.env.SARIEMA_CHROMIUM ??
  join(homedir(), "Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing");
try {
  for (const [name, engine, options] of [
    ["chromium", chromium, existsSync(chromiumPath) ? { executablePath: chromiumPath } : {}],
    ["webkit", webkit, {}],
  ]) {
    const browser = await engine.launch({ headless: true, ...options });
    try {
      const page = await browser.newPage();
      page.on("pageerror", (e) => console.error(name, e.message));
      await page.goto(new URL("page/index.html", base).href);
      await page.waitForFunction(() => window.run);
      const load_average = loadavg();
      const result = await page.evaluate(() => window.run(7));
      console.log(JSON.stringify({ browser: name, load_average, ...result }));
    } finally {
      await browser.close();
    }
  }
} finally {
  server.close();
}
