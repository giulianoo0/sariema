// SPDX-License-Identifier: Apache-2.0

// Times the engine in a browser on one room pack: the steps to its first
// frame, then a burst of draws of the frame back to back up to one read
// back, as the perf lane timed three.js's render call. One JSON line a run.
//
//   node test/bench.mjs --pack <room.srma> [--backend webgpu|webgl2]
//     [--viewport 1920,1080,1] [--quality medium] [--msaa 1] [--no-shadows]
//     [--no-ao] [--frames 300] [--warm 1000] [--reps 3]
//
// Chromium is Playwright's chromium-1248 with ANGLE on Metal, as in
// test/run.mjs; SARIEMA_CHROMIUM overrides it. Run `npm run build` first.

import { spawn } from "node:child_process";
import { homedir, loadavg } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright";

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

/** One run in a fresh context of `browser`: load `pack` (a URL), warm up, burst. */
export async function engineRun(browser, base, { pack, viewport, backend, settings, frames = 300, warmMs = 1000 }) {
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
    const load = await page.evaluate(([p, o]) => window.bench.load(p, o), [pack, settings]);
    const run = await page.evaluate(([w, n]) => window.bench.run(w, n), [warmMs, frames]);
    return { ...load, ...run, settings, viewport, load_average: loadavg(), errors };
  } finally {
    await context.close();
  }
}

function args() {
  const a = { backend: "webgpu", viewport: "1920,1080,1", quality: "medium", frames: 300, warm: 1000, reps: 3 };
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i].replace(/^--/, "");
    if (flag === "no-shadows" || flag === "no-ao") a[flag] = true;
    else a[flag] = argv[++i];
  }
  if (!a.pack) throw new Error("--pack <room.srma> is required");
  return a;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const a = args();
  const pack = resolve(a.pack);
  const { url, server } = await serve({ "/packs/": dirname(pack) });
  const browser = await chromium.launch({ headless: true, executablePath: CHROMIUM, args: CHROMIUM_ARGS });
  try {
    const settings = {
      quality: a.quality,
      ...(a.msaa ? { msaa: Number(a.msaa) } : {}),
      ...(a["no-shadows"] ? { shadows: false } : {}),
      ...(a["no-ao"] ? { ambientOcclusion: false } : {}),
    };
    for (let r = 0; r < Number(a.reps); r++) {
      const run = await engineRun(browser, url, {
        pack: new URL(`packs/${basename(pack)}`, url).href,
        viewport: a.viewport.split(",").map(Number),
        backend: a.backend,
        settings,
        frames: Number(a.frames),
        warmMs: Number(a.warm),
      });
      console.log(JSON.stringify({ pack: basename(pack), rep: r + 1, ...run }));
    }
  } finally {
    await browser.close();
    server.kill();
  }
}
