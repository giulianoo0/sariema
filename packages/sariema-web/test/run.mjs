// SPDX-License-Identifier: Apache-2.0

// The browser tests, one command:
//
//   npm test
//
// Writes the fixtures and native references (cargo), serves them with
// dist/, and in each browser draws the reference scenes and compares them
// with the native frames, plays a sound through the audio worklet, taps a
// UI button and loses and restores the GPU. Results land in
// test/out/results.json; the exit code is the verdict.
//
//   SARIEMA_BROWSERS   comma list of chromium-webgpu, chromium-webgl2, webkit-webgpu,
//                      webkit-webgl2 (default all)
//   SARIEMA_GAME_PACKS folder of room packs to compare too; their native
//                      frames and the browser's are written beside them
//   SARIEMA_CHROMIUM   Chromium executable (default Playwright's chromium-1248)

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright";

const MAX_DELTA = 6;
const MAX_RATIO = 0.005;

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(pkg, "../..");
const out = join(pkg, "test", "out");
const fixtures = join(out, "fixtures");
const packs = process.env.SARIEMA_GAME_PACKS ? resolve(process.env.SARIEMA_GAME_PACKS) : null;
const chromiumPath =
  process.env.SARIEMA_CHROMIUM ??
  join(
    homedir(),
    "Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
  );
const CHROMIUM_ARGS = [
  "--use-angle=metal",
  "--enable-gpu",
  "--ignore-gpu-blocklist",
  "--enable-unsafe-webgpu",
  "--autoplay-policy=no-user-gesture-required",
  "--mute-audio",
];
const CONFIGS = {
  "chromium-webgpu": { engine: chromium, hideWebGpu: false },
  "chromium-webgl2": { engine: chromium, hideWebGpu: true },
  "webkit-webgpu": { engine: webkit, hideWebGpu: false },
  "webkit-webgl2": { engine: webkit, hideWebGpu: true },
};

if (!existsSync(join(pkg, "dist", "index.js"))) {
  console.error("dist/ is missing: run `npm run build` first");
  process.exit(2);
}

mkdirSync(out, { recursive: true });
const cargoTarget = process.env.CARGO_TARGET_DIR ?? join(repo, "target");
execFileSync(
  "cargo",
  ["run", "-q", "-p", "sariema-web", "--example", "fixtures", "--release", "--", fixtures, ...(packs ? ["--packs", packs] : [])],
  { cwd: repo, stdio: ["ignore", "inherit", "inherit"], env: { ...process.env, CARGO_TARGET_DIR: cargoTarget } },
);

execFileSync(
  "cargo",
  ["run", "-q", "-p", "sariema-cli", "--release", "--", "web", join(repo, "examples", "demo"), "-o", join(fixtures, "game"), "--json"],
  { cwd: repo, stdio: ["ignore", "ignore", "inherit"], env: { ...process.env, CARGO_TARGET_DIR: cargoTarget } },
);

const mounts = ["--mount", `/fixtures/=${fixtures}`, ...(packs ? ["--mount", `/fixtures/packs/=${packs}`] : [])];
const server = spawn(process.execPath, [join(pkg, "scripts", "serve.mjs"), "--port", "0", ...mounts], {
  stdio: ["ignore", "pipe", "inherit"],
});
const base = await new Promise((ok, fail) => {
  createInterface({ input: server.stdout }).once("line", (line) => ok(JSON.parse(line).url));
  server.once("exit", (code) => fail(new Error(`the server exited with ${code}`)));
});

const checks = [];
function check(browser, name, pass, detail) {
  checks.push({ browser, name, pass, ...detail });
  console.log(`${pass ? "ok  " : "FAIL"} ${browser} ${name} ${JSON.stringify(detail)}`);
}

function saveRender(browser, scene) {
  if (!scene.png) return;
  const dir = scene.name.endsWith(".srma") && packs ? join(packs, "renders", browser) : join(out, "renders", browser);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${scene.name}.png`), Buffer.from(scene.png.split(",")[1], "base64"));
  delete scene.png;
}

const within = (d) => d.max_channel_delta <= MAX_DELTA || d.differing_pixels_ratio <= MAX_RATIO;

async function runBrowser(name, { engine, hideWebGpu }) {
  const launched = await engine.launch({
    headless: true,
    ...(engine === chromium ? { executablePath: chromiumPath, args: CHROMIUM_ARGS } : {}),
  });
  const logs = [];
  try {
    const context = await launched.newContext({ viewport: { width: 1000, height: 600 }, deviceScaleFactor: 1 });
    if (hideWebGpu) {
      await context.addInitScript(() => {
        Object.defineProperty(Navigator.prototype, "gpu", { get: () => undefined, configurable: true });
      });
    }
    const page = await context.newPage();
    page.on("console", (m) => (m.type() === "error" || m.type() === "warning") && logs.push(`${m.type()}: ${m.text()}`));
    page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
    page.on("response", (r) => r.status() >= 400 && logs.push(`http ${r.status()}: ${r.url()}`));
    await page.goto(new URL("test/page.html", base).href);
    await page.waitForFunction(() => window.suite);
    const started = await page.evaluate(() => window.suite.start());
    const caps = started.capabilities;
    check(name, "mount", true, { backend: caps.backend, path: caps.path, msaa: caps.msaa, variants: caps.texture_variants, mount_ms: Math.round(started.mount_ms) });
    if (hideWebGpu && caps.backend !== "webgl2") check(name, "webgl2 fallback", false, { backend: caps.backend });

    for (const scene of started.scenes) {
      const r = await page.evaluate(([n]) => window.suite.scene(n, true), [scene.name]);
      saveRender(name, r);
      if (r.skipped) {
        console.log(`skip ${name} ${scene.name}: ${r.skipped}`);
        continue;
      }
      const { max_channel_delta, differing_pixels_ratio, load_ms } = r;
      check(name, `draws ${scene.name}`, within(r), {
        max_channel_delta,
        differing_pixels_ratio: Number(differing_pixels_ratio.toFixed(5)),
        load_ms: Math.round(load_ms),
        passes: r.stats.passes,
      });
    }

    const game = await page.evaluate(() => window.suite.game());
    check(name, "loads a game folder", game.max_channel_delta === 0 && game.draws > 0 && game.model >= 0, {
      start: game.start,
      file: game.file,
      max_channel_delta: game.max_channel_delta,
    });

    const audio = await page.evaluate(() => window.suite.audio());
    const a = audio.after ?? {};
    check(name, "plays a sound through the worklet", a.received > 0 && a.played > (audio.before?.played ?? 0) && a.peak > 0.05, {
      received: a.received,
      played: a.played,
      peak: Number((a.peak ?? 0).toFixed(3)),
      underruns: a.underruns,
    });

    await page.evaluate(() => window.suite.scene("demo_lit"));
    const lost = await page.evaluate(() => window.suite.loseContext("demo_lit"));
    check(name, `recovers from a lost ${lost.backend === "webgl2" ? "context" : "device"}`, within(lost), {
      max_channel_delta: lost.max_channel_delta,
      differing_pixels_ratio: Number(lost.differing_pixels_ratio.toFixed(5)),
    });

    const size = await page.evaluate(() => window.suite.ui());
    const box = await page.locator("#stage").boundingBox();
    const [x, y] = [box.x + (size.width - 56), box.y + (size.height - 56)];
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.waitForTimeout(80);
    await page.mouse.up();
    const events = await page.evaluate(() => window.suite.uiEvents());
    const of = (t) => events.some((e) => e.type === t && e.node === "action");
    check(name, "a tap reaches the UI", of("press") && of("tap"), { events: events.map((e) => `${e.type}:${e.node}`) });

    await page.goto(new URL("dist/probe/?auto", base).href);
    const probe = await page.waitForFunction(() => window.probeResult?.smoke && window.probeResult, null, { timeout: 30_000 });
    const p = await probe.jsonValue();
    check(name, "the probe page draws and plays", p.smoke.ok && p.smoke.audio_ok === true, {
      webgpu: p.webgpu.available,
      webgl2: p.webgl2.available,
      backend: p.smoke.backend,
      first_frame_ms: Math.round(p.smoke.first_frame_ms ?? -1),
      lit: Number((p.smoke.lit_fraction ?? 0).toFixed(3)),
      audio: p.smoke.audio,
      error: p.smoke.error ?? p.smoke.audio_error,
    });
    await page.goto(new URL("test/page.html", base).href);
    await page.waitForFunction(() => window.suite);
    await page.evaluate(() => window.suite.start());

    const plainCaps = await page.evaluate(() => window.suite.remount({ compressedTextures: false }));
    const plain = await page.evaluate(() => window.suite.game());
    check(name, "takes the RGBA8 packs without compressed textures", plain.file === "demo.rgba8.srma" && plain.max_channel_delta === 0, {
      variants: plainCaps.texture_variants,
      file: plain.file,
    });
  } catch (e) {
    check(name, "runs", false, { error: String(e?.message ?? e).split("\n")[0] });
  } finally {
    await launched.close();
    if (logs.length) console.log(`${name} console:\n  ${logs.slice(0, 20).join("\n  ")}`);
  }
}

const wanted = (process.env.SARIEMA_BROWSERS ?? Object.keys(CONFIGS).join(",")).split(",");
try {
  for (const name of wanted) {
    if (!CONFIGS[name]) throw new Error(`unknown browser ${name}`);
    await runBrowser(name, CONFIGS[name]);
  }
} finally {
  server.kill();
}

const failed = checks.filter((c) => !c.pass);
writeFileSync(join(out, "results.json"), JSON.stringify({ tolerance: { MAX_DELTA, MAX_RATIO }, checks }, null, 2));
console.log(JSON.stringify({ ok: failed.length === 0, checks: checks.length, failed: failed.length, results: join(out, "results.json") }));
process.exit(failed.length ? 1 : 0);
