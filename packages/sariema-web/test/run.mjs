// SPDX-License-Identifier: Apache-2.0

// The browser tests, one command:
//
//   npm test
//
// Writes the fixtures and native references (cargo), serves them with
// dist/, and in each browser draws the reference scenes and compares them
// with the native frames, plays a sound through the audio worklet, taps a
// UI button and loses and restores the GPU. Then it plays the climb game
// (examples/climb): its replay reaches the native hashes and frames and
// keeps the log it started with, a seed past 2^53 reaches native, its steps
// allocate where and as much as natively and nothing once steady (the
// counting build, which this command builds), its music plays without
// underruns, a touch on its joystick moves the hero, a game that never
// returns is stopped by its budget and one over it at a later step stops
// where the CLI stops it, and a lost GPU leaves the game at its step, even
// when the first restore fails. The guide game (examples/guide), with its
// particles, trail and highlight, reaches the native hashes and frames with
// the same seed. Results land in test/out/results.json; the exit code is
// the verdict.
//
//   SARIEMA_BROWSERS   comma list of chromium-webgpu, chromium-webgl2, webkit-webgpu,
//                      webkit-webgl2 (default all)
//   SARIEMA_GAME_PACKS folder of room packs to compare too; their native
//                      frames and the browser's are written beside them
//   SARIEMA_CHROMIUM   Chromium executable (default Playwright's chromium-1248)

import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { homedir, loadavg } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
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

const CLIMB_STEPS = [120, 300, 600, 900];
const climbDir = join(fixtures, "climb");
const climbLog = join(repo, "examples", "climb", "input", "play.json");
const cli = (...argv) =>
  JSON.parse(
    execFileSync(cargoBin("sariema"), [...argv, "--json"], { cwd: repo, stdio: ["ignore", "pipe", "inherit"] }).toString(),
  );
function cargoBin(name) {
  return join(cargoTarget, "release", name);
}
/** The `--json` error of a CLI run that must fail. */
function cliError(...argv) {
  try {
    execFileSync(cargoBin("sariema"), [...argv, "--json"], { cwd: repo, stdio: ["ignore", "pipe", "ignore"] });
  } catch (e) {
    return JSON.parse(e.stdout.toString()).error;
  }
  throw new Error(`sariema ${argv.join(" ")} succeeded`);
}
const nativeReplay = (...argv) =>
  JSON.parse(
    execFileSync("cargo", ["run", "-q", "-p", "sariema-web", "--example", "replay", "--release", "--", ...argv], {
      cwd: repo,
      stdio: ["ignore", "pipe", "inherit"],
      env: { ...process.env, CARGO_TARGET_DIR: cargoTarget },
    }).toString(),
  );
execFileSync("cargo", ["build", "-q", "-p", "sariema-cli", "--release"], {
  cwd: repo,
  stdio: ["ignore", "inherit", "inherit"],
  env: { ...process.env, CARGO_TARGET_DIR: cargoTarget },
});
cli("web", join(repo, "examples", "climb"), "-o", climbDir);
copyFileSync(climbLog, join(climbDir, "play.json"));
const nativeClimb = {};
for (const [variant, file] of [
  ["astc", "game.srmg"],
  ["rgba8", "game.rgba8.srmg"],
]) {
  for (const n of CLIMB_STEPS) {
    const r = cli(
      "run", "--headless", "--game", join(climbDir, file), "--input", climbLog, "--steps", String(n),
      "--screenshot", join(climbDir, `${variant}-${n}.png`), "--width", "960", "--height", "540", "--quality", "high",
    );
    nativeClimb[n] ??= r;
    if (r.engine_hash !== nativeClimb[n].engine_hash) throw new Error(`the two variants' engine hashes differ at step ${n}`);
  }
}
const GUIDE_STEPS = [150, 300];
const guideDir = join(fixtures, "guide");
const guideLog = join(guideDir, "play.json");
cli("web", join(repo, "examples", "guide"), "-o", guideDir);
writeFileSync(guideLog, JSON.stringify({ changes: [] }));
const nativeGuide = {};
for (const [variant, file] of [
  ["astc", "game.srmg"],
  ["rgba8", "game.rgba8.srmg"],
]) {
  for (const n of GUIDE_STEPS) {
    const r = cli(
      "run", "--headless", "--game", join(guideDir, file), "--input", guideLog, "--steps", String(n),
      "--screenshot", join(guideDir, `${variant}-${n}.png`), "--width", "960", "--height", "540", "--quality", "high",
    );
    nativeGuide[n] ??= r;
    if (r.fx_hash !== nativeGuide[n].fx_hash) throw new Error(`the two variants' fx hashes differ at step ${n}`);
  }
}
const nativeCost = nativeReplay(join(climbDir, "game.rgba8.srmg"), climbLog, "900", "7");
// A seed past 2^53, which a JavaScript number cannot hold.
const BIG_SEED = "18446744073709551557";
const nativeSeeded = cli("run", "--headless", "--game", join(climbDir, "game.rgba8.srmg"), "--input", climbLog, "--steps", "120", "--seed", BIG_SEED);
// A game whose 30th tick never returns: where the CLI stops it, and the
// hashes a native replay ends with there.
const LATE_STEPS = "40";
const lateCli = cliError("run", "--headless", "--game", join(fixtures, "late.srmg"), "--steps", LATE_STEPS);
const lateNative = nativeReplay(join(fixtures, "late.srmg"), join(fixtures, "idle.json"), LATE_STEPS);
execFileSync(process.execPath, [join(pkg, "scripts", "build.mjs"), "--alloc-count"], { cwd: pkg, stdio: ["ignore", "ignore", "inherit"] });

const mounts = ["--mount", `/fixtures/=${fixtures}`, ...(packs ? ["--mount", `/fixtures/packs/=${packs}`] : [])];
const server = spawn(process.execPath, [join(pkg, "scripts", "serve.mjs"), "--port", "0", ...mounts], {
  stdio: ["ignore", "pipe", "inherit"],
});
const base = await new Promise((ok, fail) => {
  createInterface({ input: server.stdout }).once("line", (line) => ok(JSON.parse(line).url));
  server.once("exit", (code) => fail(new Error(`the server exited with ${code}`)));
});

// Another origin that gzips the game folder, as a CDN does. Cross-origin,
// the page sees Content-Length (the gzipped size) but not Content-Encoding.
const game = join(fixtures, "game");
const cdn = createServer((req, res) => {
  const file = resolve(game, decodeURIComponent(new URL(req.url, "http://cdn").pathname).slice(1));
  const cors = { "Access-Control-Allow-Origin": "*" };
  if (!file.startsWith(game + sep) || !existsSync(file)) {
    res.writeHead(404, cors).end();
    return;
  }
  const body = gzipSync(readFileSync(file));
  res.writeHead(200, { ...cors, "Content-Encoding": "gzip", "Content-Length": body.length }).end(body);
});
const cdnUrl = await new Promise((ok) => cdn.listen(0, "127.0.0.1", () => ok(`http://127.0.0.1:${cdn.address().port}/`)));

const checks = [];
function check(browser, name, pass, detail) {
  checks.push({ browser, name, pass, ...detail });
  console.log(`${pass ? "ok  " : "FAIL"} ${browser} ${name} ${JSON.stringify(detail)}`);
}

const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const spread = (xs) => ({ median: median(xs), min: Math.min(...xs), max: Math.max(...xs), n: xs.length });
const numbers = { native_step_ms: spread(nativeCost.per_step_ms), load_average: loadavg(), browsers: {} };

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

    const there = await page
      .evaluate(([url]) => window.suite.crossOrigin(url), [cdnUrl])
      .catch((e) => ({ error: e.message.split("\n")[0] }));
    check(name, "loads a gzipped game folder from another origin", there.max_channel_delta === 0, there);

    const audio = await page.evaluate(() => window.suite.audio());
    const a = audio.after ?? {};
    check(name, "plays a sound through the worklet", a.received > 0 && a.played > (audio.before?.played ?? 0) && a.peak > 0.05 && a.underruns === 0, {
      received: a.received,
      played: a.played,
      peak: Number((a.peak ?? 0).toFixed(3)),
      underruns: a.underruns,
    });

    const second = await page.evaluate(() => window.suite.secondBank());
    const lead = Math.ceil((0.1 * (second.rate ?? 48000)) / 1024) * 1024 + 1024;
    check(name, "a second sound bank keeps the lead", second.queued <= lead && second.underruns === 0, {
      queued: second.queued,
      lead,
      underruns: second.underruns,
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

    await page.evaluate(() => window.suite.listen());
    await page.focus("#field");
    await page.keyboard.type("ab");
    await page.keyboard.press("Tab");
    const typed = await page.evaluate(() => ({ value: document.getElementById("field").value, focus: document.activeElement?.id ?? "" }));
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("KeyW");
    const keys = await page.evaluate(() => window.suite.keys());
    const inField = keys.filter((k) => k.target === "field");
    check(name, "keys typed into a page input stay there", typed.value === "ab" && typed.focus !== "field" && inField.every((k) => !k.taken), {
      ...typed,
      taken: inField.filter((k) => k.taken).map((k) => k.code),
    });
    check(name, "keys reach the engine while the page has no focus", keys.some((k) => k.code === "KeyW" && k.target === "BODY" && k.taken), {
      keys: keys.map((k) => `${k.code}@${k.target}${k.taken ? "!" : ""}`),
    });

    const small = await page.evaluate(() => window.suite.smallLead({ lead_s: 0.018, block_frames: 128 }));
    check(name, "a lead shorter than 1024 frames plays without underruns", small.received > 0 && small.underruns === 0, {
      received: small.received,
      queued: small.queued,
      underruns: small.underruns,
    });

    const failed = await page.evaluate(() => window.suite.failedRebuild());
    check(name, "a rebuild that fails emits error", failed.error !== null, failed);

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

    await climbChecks(name, page);
    await guideChecks(name, page);
  } catch (e) {
    check(name, "runs", false, { error: String(e?.message ?? e).split("\n")[0] });
  } finally {
    await launched.close();
    if (logs.length) console.log(`${name} console:\n  ${logs.slice(0, 20).join("\n  ")}`);
  }
}

async function guideChecks(name, page) {
  await page.evaluate(() => window.suite.remount({}));
  const replay = await page.evaluate(([s]) => window.suite.guideReplay(s), [GUIDE_STEPS]);
  for (const r of replay.at) {
    const native = nativeGuide[r.n];
    saveRender(name, { name: `guide-${replay.variant}-${r.n}`, png: r.png });
    const same = ["engine_hash", "state_hash", "fx_hash"].every((h) => r.hashes[h] === native[h]);
    check(name, `the guide replay reaches the native hashes, its effects' included, at step ${r.n}`, same && r.hashes.steps === r.n, {
      fx_hash: r.hashes.fx_hash,
      native: native.fx_hash,
    });
    check(name, `the guide's frame at step ${r.n} matches the native one`, within(r), {
      file: replay.file,
      max_channel_delta: r.max_channel_delta,
      differing_pixels_ratio: Number(r.differing_pixels_ratio.toFixed(5)),
    });
  }
}

async function climbChecks(name, page) {
  await page.evaluate(() => window.suite.remount({}));
  const replay = await page.evaluate(([s]) => window.suite.climbReplay(s), [CLIMB_STEPS]);
  for (const r of replay.at) {
    const native = nativeClimb[r.n];
    saveRender(name, { name: `climb-${replay.variant}-${r.n}`, png: r.png });
    const same = ["engine_hash", "state_hash", "physics_hash"].every((h) => r.hashes[h] === native[h]);
    check(name, `the climb replay reaches the native hashes at step ${r.n}`, same && r.hashes.steps === r.n, {
      engine_hash: r.hashes.engine_hash,
      native: native.engine_hash,
      audio_hash_same: r.hashes.audio_hash === native.audio_hash,
    });
    check(name, `the climb's frame at step ${r.n} matches the native one`, within(r), {
      file: replay.file,
      max_channel_delta: r.max_channel_delta,
      differing_pixels_ratio: Number(r.differing_pixels_ratio.toFixed(5)),
    });
  }

  const other = await page.evaluate(() => window.suite.climbReplayOtherLog());
  check(name, "a replay asked to go on with another input log is E_USAGE and stays where it was", other.code === "E_USAGE" && other.stayed === 60 && other.after === 120, other);

  const seeds = await page.evaluate(([big]) => window.suite.climbSeeds(big, 120), [BIG_SEED]);
  const seeded = ["engine_hash", "state_hash", "physics_hash"].every((h) => seeds.hashes[h] === nativeSeeded[h]);
  check(name, "seeds out of range are E_USAGE, and one past 2^53 reaches the native hashes", seeds.codes.every((c) => c === "E_USAGE") && seeded && nativeSeeded.engine_hash !== nativeClimb[120].engine_hash, {
    codes: seeds.codes,
    engine_hash: seeds.hashes.engine_hash,
    native: nativeSeeded.engine_hash,
    seed_0: nativeClimb[120].engine_hash,
  });

  const late = await page.evaluate(([n]) => window.suite.lateBudget(n), [Number(LATE_STEPS)]);
  const lateStep = (message) => Number(/step (\d+): /.exec(message ?? "")?.[1]);
  check(name, "a game over its budget at a later step stops there with the native hashes", late.error?.code === "E_GAME_BUDGET" && lateCli.code === "E_GAME_BUDGET" && lateStep(late.error.message) === lateStep(lateCli.message) && lateStep(late.error.message) === lateNative.stopped?.step && JSON.stringify(late.hashes) === JSON.stringify(lateNative.hashes), {
    browser: { step: lateStep(late.error?.message), hashes: late.hashes },
    cli: { code: lateCli.code, step: lateStep(lateCli.message) },
    native: lateNative.stopped,
  });

  const kept = await page.evaluate(() => window.suite.saves(200));
  const loadedAt = Number(kept.echoed.find((e) => e.endsWith("loaded best"))?.split(" ")[0]);
  check(name, "the page keeps a game's save and answers its load frames later", kept.answered === true && kept.sent.includes("1 load_saves") && kept.sent.includes("2 save") && JSON.stringify(kept.kept) === '{"best":[12]}' && loadedAt > 4 && kept.echoed.includes(`${loadedAt} done`), kept);
  const unheard = await page.evaluate(() => window.suite.saves(null));
  check(name, "with no listener for loads a load is answered as no save at once", unheard.answered === false && JSON.stringify(unheard.echoed) === JSON.stringify(["2 done"]), unheard);

  const cost = await page.evaluate(() => window.suite.climbStepCost(900, 7));
  numbers.browsers[name] = { step_ms: spread(cost) };
  check(name, "times the climb's step", cost.length === 7, { median_ms: Number(median(cost).toFixed(4)), native_ms: Number(numbers.native_step_ms.median.toFixed(4)) });

  const audio = await page.evaluate(() => window.suite.climbAudio(3000));
  check(name, "the climb's music plays without underruns at the default lead", audio.received > 0 && audio.peak > 0.05 && audio.underruns === 0, {
    received: audio.received,
    peak: Number((audio.peak ?? 0).toFixed(3)),
    underruns: audio.underruns,
    steps: audio.steps,
  });

  await page.evaluate(() => window.suite.climbLive());
  const touch = await page.evaluate(() => window.suite.touchJoystick(60, 600));
  const moved = touch.before && touch.held ? Math.hypot(touch.held[0] - touch.before[0], touch.held[2] - touch.before[2]) : 0;
  check(name, "a touch on the HUD joystick moves the hero", moved > 0.2, { moved: Number(moved.toFixed(3)), ...touch });

  const forever = await page.evaluate(() => window.suite.forever());
  numbers.browsers[name].budget_stall_ms = forever.longest_gap_ms;
  check(name, "a game that never returns raises E_GAME_BUDGET and the page goes on", forever.error.code === "E_GAME_BUDGET" && forever.timer_ms < 200 && !forever.running, {
    code: forever.error.code,
    error_ms: Math.round(forever.error_ms),
    longest_gap_ms: Math.round(forever.longest_gap_ms),
    timer_ms: Math.round(forever.timer_ms),
  });

  const retried = await page.evaluate(() => window.suite.climbRestoreRetry());
  check(name, "a restore that fails keeps the game, and a later one goes on from its step", retried.error.code === "E_GPU" && retried.atFailure.steps === retried.atRetry.steps && retried.atFailure.engine_hash === retried.atRetry.engine_hash && retried.after.steps > retried.atRetry.steps && retried.drawn, {
    error: retried.error,
    at_failure: retried.atFailure.steps,
    at_retry: retried.atRetry.steps,
    after: retried.after.steps,
    drawn: retried.drawn,
  });

  const lost = await page.evaluate(() => window.suite.climbLoseContext());
  check(
    name,
    `a lost ${lost.backend === "webgl2" ? "context" : "device"} leaves the game at its step`,
    lost.atLoss.steps === lost.atRestore.steps && lost.atLoss.engine_hash === lost.atRestore.engine_hash && lost.after.steps > lost.atRestore.steps && lost.drawn,
    { at_loss: lost.atLoss.steps, at_restore: lost.atRestore.steps, after: lost.after.steps, drawn: lost.drawn },
  );

  await page.goto(new URL("test/page.html?counting", base).href);
  await page.waitForFunction(() => window.suite);
  await page.evaluate(() => window.suite.start());
  // The native pins: each step of the climb that allocates, and the audio
  // feed's allocations in each second (crates/sariema-cli/tests).
  const pins = JSON.parse(readFileSync(join(repo, "crates", "sariema-cli", "tests", "climb-allocations.json"), "utf8"));
  const allocs = await page.evaluate(() => window.suite.climbAllocations(1200));
  numbers.browsers[name].allocations = allocs;
  const stepsIn = (list, from, to) => list.filter(([step]) => step >= from && step < to);
  check(name, "the climb's steps allocate as natively, step by step, through its events (0..600)", JSON.stringify(stepsIn(allocs.steps, 0, 600)) === JSON.stringify(stepsIn(pins.steps, 0, 600)), {
    browser: stepsIn(allocs.steps, 0, 600),
    native: stepsIn(pins.steps, 0, 600),
  });
  check(name, "the climb's steps allocate nothing once steady (600..1200)", stepsIn(allocs.steps, 600, 1200).length === 0, { steps: stepsIn(allocs.steps, 600, 1200) });
  // The cinema bars slide in over 215..237 and out over 486..507, the
  // objective's panel animating with them; nothing else happens then.
  const sliding = allocs.steps.filter(([step]) => (step >= 215 && step <= 237) || (step >= 486 && step <= 507));
  check(name, "steps that only animate the HUD allocate nothing", sliding.length === 0, { steps: sliding });
  // Live frames step the game and feed the audio; the feed's typical
  // second (the music decoder's packets) bounds them, scaled to the steps
  // run, with a few packets of slack for where the seconds cut.
  const seconds = [...pins.feed_per_second].sort((a, b) => a - b);
  const steady = seconds[seconds.length >> 1];
  const cap = Math.ceil((steady * allocs.live_steps) / 60) + Math.ceil(steady / 10);
  check(name, "a second of live frames allocates no more than the audio feed's steady second", allocs.live_frame_allocations <= cap, {
    live_frame_allocations: allocs.live_frame_allocations,
    live_steps: allocs.live_steps,
    live_seconds: allocs.live_seconds,
    cap,
  });
}

const wanted = (process.env.SARIEMA_BROWSERS ?? Object.keys(CONFIGS).join(",")).split(",");
try {
  for (const name of wanted) {
    if (!CONFIGS[name]) throw new Error(`unknown browser ${name}`);
    await runBrowser(name, CONFIGS[name]);
  }
} finally {
  server.kill();
  cdn.close();
}

const failed = checks.filter((c) => !c.pass);
writeFileSync(join(out, "results.json"), JSON.stringify({ tolerance: { MAX_DELTA, MAX_RATIO }, numbers, checks }, null, 2));
console.log(JSON.stringify({ ok: failed.length === 0, checks: checks.length, failed: failed.length, results: join(out, "results.json") }));
process.exit(failed.length ? 1 : 0);
