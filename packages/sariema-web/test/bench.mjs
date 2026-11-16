// SPDX-License-Identifier: Apache-2.0

// Times the engine in a browser on one room pack or game folder: the steps
// to its first frame; the CPU of a frame at the display's pace (median of
// 120, steps and draw); then a burst of draws of the frame back to back up
// to one read back, as the perf lane timed three.js's render call. In a
// burst WebGL2's CPU also waits on the browser's command buffer once the
// GPU falls behind. A game is replayed with `--log` to step `--steps`
// first, its hashes printed as `replay` to compare with `sariema run
// --headless`, then plays on with no input. `--timeline <frames>` instead
// plays the game from its first frame at the display's pace and prints,
// for each frame, the step reached, the frame's CPU, the time since the
// last frame and the frame's passes and draws, to see what a moment of the game (a room switch) costs. One
// JSON line a run.
//
//   node test/bench.mjs --pack <room.srma> | --game <folder> [--log <input.json>]
//     [--steps 120] [--browser chromium|webkit] [--backend webgpu|webgl2]
//     [--viewport 1920,1080,1] [--quality medium] [--msaa 1] [--no-shadows]
//     [--no-ao] [--frames 300] [--warm 1000] [--timeline <frames>] [--reps 3]
//
// Chromium is Playwright's chromium-1248 with ANGLE on Metal, as in
// test/run.mjs; SARIEMA_CHROMIUM overrides it. Run `npm run build` first.

import { spawn } from "node:child_process";
import { homedir, loadavg } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium, webkit } from "playwright";

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const CHROMIUM =
  process.env.SARIEMA_CHROMIUM ??
  join(
    homedir(),
    "Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
  );
export const CHROMIUM_ARGS = ["--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist", "--enable-unsafe-webgpu"];

/** Serves the package with `mounts` ({prefix: dir}); resolves to its base URL and process. */
export async function serve(mounts) {
  const argv = [join(pkg, "scripts", "serve.mjs"), "--port", "0"];
  for (const [prefix, dir] of Object.entries(mounts)) argv.push("--mount", `${prefix}=${dir}`);
  const server = spawn(process.execPath, argv, { stdio: ["ignore", "pipe", "inherit"] });
  const url = await new Promise((ok, fail) => {
    createInterface({ input: server.stdout }).once("line", (line) => ok(JSON.parse(line).url));
    server.once("exit", (code) => fail(new Error(`the server exited with ${code}`)));
  });
  return { url, server };
}

/** One run in a fresh context of `browser`: load `pack` (a URL; a game folder's ends in /), warm up, burst. */
export async function engineRun(browser, base, { pack, log, steps, viewport, backend, settings, frames = 300, warmMs = 1000, timeline }) {
  const [width, height, dpr] = viewport;
  const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: dpr });
  try {
    if (backend === "webgl2") {
      await context.addInitScript(() => {
        Object.defineProperty(Navigator.prototype, "gpu", { get: () => undefined, configurable: true });
      });
    }
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(new URL("test/bench.html", base).href);
    await page.waitForFunction(() => window.bench);
    const load = await page.evaluate(([p, o, l, n]) => window.bench.load(p, o, l, n), [pack, settings, log, steps]);
    const run = timeline
      ? await page.evaluate((n) => window.bench.timeline(n), timeline)
      : await page.evaluate(([w, n]) => window.bench.run(w, n), [warmMs, frames]);
    return { ...load, ...run, settings, viewport, load_average: loadavg(), errors };
  } finally {
    await context.close();
  }
}

function args() {
  const a = { browser: "chromium", backend: "webgpu", viewport: "1920,1080,1", quality: "medium", steps: 120, frames: 300, warm: 1000, reps: 3 };
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i].replace(/^--/, "");
    if (flag === "no-shadows" || flag === "no-ao") a[flag] = true;
    else a[flag] = argv[++i];
  }
  if (!a.pack === !a.game) throw new Error("one of --pack <room.srma> or --game <folder> is required");
  return a;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const a = args();
  const pack = resolve(a.pack ?? a.game);
  const dir = a.game ? pack : dirname(pack);
  const mounts = { "/packs/": dir, ...(a.log ? { "/log/": dirname(resolve(a.log)) } : {}) };
  const { url, server } = await serve(mounts);
  const browser =
    a.browser === "webkit"
      ? await webkit.launch({ headless: true })
      : await chromium.launch({ headless: true, executablePath: CHROMIUM, args: CHROMIUM_ARGS });
  try {
    const settings = {
      quality: a.quality,
      ...(a.msaa ? { msaa: Number(a.msaa) } : {}),
      ...(a["no-shadows"] ? { shadows: false } : {}),
      ...(a["no-ao"] ? { ambientOcclusion: false } : {}),
    };
    for (let r = 0; r < Number(a.reps); r++) {
      const run = await engineRun(browser, url, {
        pack: new URL(a.game ? "packs/" : `packs/${basename(pack)}`, url).href,
        log: a.log ? new URL(`log/${basename(a.log)}`, url).href : undefined,
        steps: Number(a.steps),
        viewport: a.viewport.split(",").map(Number),
        backend: a.backend,
        settings,
        frames: Number(a.frames),
        warmMs: Number(a.warm),
        timeline: a.timeline ? Number(a.timeline) : undefined,
      });
      console.log(JSON.stringify({ pack: basename(pack), browser: a.browser, rep: r + 1, ...run }));
    }
  } finally {
    await browser.close();
    server.kill();
  }
}
