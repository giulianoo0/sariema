// SPDX-License-Identifier: Apache-2.0

// What test/bench.mjs drives: load a room pack or play a game folder on a
// full-window canvas, timing each step to the first frame, then time a
// burst of draws.

import { mount } from "../dist/index.js";

const canvas = document.getElementById("stage");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const frame = () => new Promise((r) => requestAnimationFrame(() => r()));
let sariema = null;

window.bench = {
  async load(pack, options, log, steps) {
    const t0 = performance.now();
    sariema = await mount(canvas, { autoStart: false, ...options });
    const mounted = performance.now();
    let bytes;
    if (pack.endsWith("/")) {
      const manifest = await (await fetch(new URL("sariema-web.json", pack))).json();
      bytes = new Uint8Array(await (await fetch(new URL(sariema.pick(manifest.game, "the game"), pack))).arrayBuffer());
    } else {
      bytes = new Uint8Array(await (await fetch(pack)).arrayBuffer());
    }
    const fetched = performance.now();
    let replay;
    if (pack.endsWith("/")) {
      await sariema.playGame(bytes, { audio: false });
      if (log) replay = sariema.replayGame(await (await fetch(log)).text(), steps);
    } else {
      await sariema.loadRoom(bytes);
    }
    const loaded = performance.now();
    sariema.start();
    await frame();
    await frame();
    const first = performance.now();
    sariema.stop();
    const caps = sariema.capabilities();
    return {
      backend: sariema.backend,
      path: caps.path,
      msaa: caps.msaa,
      size: [canvas.width, canvas.height],
      bytes: bytes.length,
      script_ms: t0,
      mount_ms: mounted - t0,
      fetch_ms: fetched - mounted,
      load_ms: loaded - fetched,
      first_frame_after_load_ms: first - loaded,
      navigation_to_first_frame_ms: first,
      replay,
    };
  },

  async run(warmMs, frames) {
    sariema.start();
    await sleep(warmMs);
    sariema.stop();
    const paced = [];
    await new Promise((resolve) => {
      const tick = (t) => {
        const start = performance.now();
        sariema.frame(t);
        paced.push(performance.now() - start);
        if (paced.length < 120) requestAnimationFrame(tick);
        else resolve();
      };
      requestAnimationFrame(tick);
    });
    paced.sort((a, b) => a - b);
    const burst = await sariema.bench(frames);
    return { ...burst, paced_cpu_ms: paced[59], paced_p90_cpu_ms: paced[107], paced_frames: paced.length, stats: sariema.stats() };
  },
};
