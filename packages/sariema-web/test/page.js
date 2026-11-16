// SPDX-License-Identifier: Apache-2.0

// What the browser tests drive: each call loads, draws or plays something
// with the engine and returns what came of it. The runner (run.mjs) holds
// the expectations.

import { mount } from "../dist/index.js";

const canvas = document.getElementById("stage");
const fixtures = new URL("/fixtures/", location.href);
const at = (path) => new URL(path, fixtures);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let sariema = null;
let manifest = null;
const uiEvents = [];
const keys = [];

async function referencePixels(url) {
  const blob = await (await fetch(url)).blob();
  const bitmap = await createImageBitmap(blob, { colorSpaceConversion: "none", premultiplyAlpha: "none" });
  const c = new OffscreenCanvas(bitmap.width, bitmap.height);
  const g = c.getContext("2d", { willReadFrequently: true });
  g.drawImage(bitmap, 0, 0);
  return g.getImageData(0, 0, bitmap.width, bitmap.height).data;
}

/** As `sariema_render::compare_images`: the largest channel delta, and the share of pixels over 6. */
function compare(a, b) {
  let max = 0;
  let over = 0;
  for (let i = 0; i < a.length; i += 4) {
    const d = Math.max(Math.abs(a[i] - b[i]), Math.abs(a[i + 1] - b[i + 1]), Math.abs(a[i + 2] - b[i + 2]), Math.abs(a[i + 3] - b[i + 3]));
    if (d > max) max = d;
    if (d > 6) over++;
  }
  return { max_channel_delta: max, differing_pixels_ratio: over / (a.length / 4) };
}

async function pngDataUrl({ width, height, data }) {
  const c = new OffscreenCanvas(width, height);
  c.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(data), width, height), 0, 0);
  const blob = await c.convertToBlob({ type: "image/png" });
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.readAsDataURL(blob);
  });
}

function placement({ scale, rotate_y, translate: [x, y, z] }) {
  const c = Math.cos(rotate_y) * scale;
  const s = Math.sin(rotate_y) * scale;
  return [c, 0, -s, 0, 0, scale, 0, 0, s, 0, c, 0, x, y, z, 1];
}

function once(type, ms) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no ${type} event in ${ms} ms`)), ms);
    const off = sariema.on(type, (e) => {
      clearTimeout(timer);
      off();
      resolve(e);
    });
  });
}

let arm = null;
// `?counting` loads the build whose allocator counts.
const counting = new URLSearchParams(location.search).has("counting");
const wasmUrl = counting ? new URL("../dist/wasm/sariema_web_counting_bg.wasm", import.meta.url) : undefined;
const climb = at("climb/");
let playLog = null;

async function playClimb(options) {
  playLog ??= await (await fetch(at("climb/play.json"))).text();
  const m = await sariema.loadGame(climb, options);
  return sariema.pick(m.game, "climb");
}

/** The game in `dir` replayed with `log`, its hashes and frame at each of `steps`. */
async function replayFrames(dir, file, log, steps) {
  const variant = file.includes(".rgba8.") ? "rgba8" : "astc";
  const at_ = [];
  for (const n of steps) {
    const hashes = sariema.replayGame(log, n);
    const frame = await sariema.capture();
    const diff = compare(frame.data, await referencePixels(at(`${dir}/${variant}-${n}.png`)));
    at_.push({ n, hashes, ...diff, png: await pngDataUrl(frame) });
  }
  return { file, variant, at: at_ };
}

window.suite = {
  async start(options = {}) {
    manifest = await (await fetch(at("fixtures.json"))).json();
    const t0 = performance.now();
    sariema = await mount(canvas, { quality: manifest.quality, maxPixelRatio: 1, autoStart: false, wasmUrl, ...options });
    sariema.on("ui", (e) => uiEvents.push(e));
    return { mount_ms: performance.now() - t0, capabilities: sariema.capabilities(), scenes: manifest.scenes };
  },

  async scene(name, keepRender = false) {
    const scene = manifest.scenes.find((s) => s.name === name);
    const variants = sariema.capabilities().texture_variants;
    const variant = scene.room.includes(".rgba8.") ? "rgba8" : "astc";
    if (scene.room.startsWith("packs/") && !variants.includes(variant)) {
      return { name, skipped: `this device has no ${variant} textures` };
    }
    const t0 = performance.now();
    await sariema.loadRoom(at(scene.room));
    const load_ms = performance.now() - t0;
    if (scene.actor) {
      arm ??= sariema.spawnActor(await sariema.loadActor(at(scene.actor.pack)));
      const a = scene.actor;
      sariema.setActor(arm, { transform: placement(a), clip: a.clip, time: a.time, speed: a.speed });
    } else if (arm !== null) {
      sariema.setActor(arm, { visible: false });
    }
    const frame = await sariema.capture();
    const diff = compare(frame.data, await referencePixels(at(scene.reference)));
    const stats = sariema.stats();
    return { name, load_ms, ...diff, stats, png: keepRender ? await pngDataUrl(frame) : undefined };
  },

  async remount(options) {
    sariema.destroy();
    arm = null;
    sariema = await mount(canvas, { quality: manifest.quality, maxPixelRatio: 1, autoStart: false, wasmUrl, ...options });
    return sariema.capabilities();
  },

  /** The climb replayed with its input log, its hashes and frame at each of `steps`. */
  async climbReplay(steps) {
    const file = await playClimb({ audio: false });
    return replayFrames("climb", file, playLog, steps);
  },

  /** The guide replayed without input, its hashes and frame at each of `steps`. */
  async guideReplay(steps) {
    const log = await (await fetch(at("guide/play.json"))).text();
    const m = await sariema.loadGame(at("guide/"), { audio: false });
    return replayFrames("guide", sariema.pick(m.game, "guide"), log, steps);
  },

  /** Mean milliseconds a replayed step takes, over `steps` steps, for each of `reps` fresh games. */
  async climbStepCost(steps, reps) {
    const per_step_ms = [];
    for (let r = 0; r < reps; r++) {
      await playClimb({ audio: false });
      const t0 = performance.now();
      sariema.replayGame(playLog, steps);
      per_step_ms.push((performance.now() - t0) / steps);
    }
    return per_step_ms;
  },

  /**
   * With the counting build: the allocations the replayed steps have made
   * by each of `steps`, then those of a second of live frames (steps and
   * audio feed) after the last.
   */
  async climbAllocations(steps) {
    await playClimb({ audio: false });
    const at_ = [];
    for (const n of steps) {
      sariema.replayGame(playLog, n);
      at_.push(sariema.stats().game.step_allocations);
    }
    sariema.start();
    await sleep(1000);
    const live = sariema.stats().game;
    sariema.stop();
    return { steps, step_allocations: at_, live_frame_allocations: live.frame_allocations, live_steps: live.steps - steps.at(-1) };
  },

  /** The climb played live for `ms`, its music through the worklet. */
  async climbAudio(ms) {
    await playClimb({});
    sariema.start();
    await sariema.resumeAudio();
    await sleep(ms);
    const stats = await sariema.audioStats();
    const game = sariema.stats().game;
    sariema.stop();
    return { ...stats, steps: game.steps };
  },

  async climbLive() {
    await playClimb({ audio: false });
    sariema.start();
    await sleep(300);
    return sariema.stats().game;
  },

  /** A synthetic touch drag on the HUD's joystick zone, held `ms`. */
  async touchJoystick(dx, ms) {
    const box = canvas.getBoundingClientRect();
    const [x, y] = [box.left + 80, box.bottom - 80];
    const fire = (type, cx) =>
      canvas.dispatchEvent(new PointerEvent(type, { pointerId: 7, pointerType: "touch", isPrimary: true, clientX: cx, clientY: y, bubbles: true }));
    const before = sariema.stats().game.hero;
    fire("pointerdown", x);
    for (let i = 1; i <= 5; i++) {
      await sleep(16);
      fire("pointermove", x + (dx * i) / 5);
    }
    await sleep(ms);
    const held = sariema.stats().game.hero;
    fire("pointerup", x + dx);
    await sleep(100);
    sariema.stop();
    return { before, held, after: sariema.stats().game.hero };
  },

  /** A game whose logic never returns: the error, how long the page stalled, and a timer after it. */
  async forever() {
    const errored = once("error", 20000);
    await sariema.playGame(at(manifest.forever), { audio: false });
    let longest = 0;
    let last = performance.now();
    const watch = setInterval(() => {
      const now = performance.now();
      longest = Math.max(longest, now - last);
      last = now;
    }, 10);
    const t0 = performance.now();
    sariema.start();
    const error = await errored.catch((e) => ({ code: null, message: e.message }));
    const error_ms = performance.now() - t0;
    const t1 = performance.now();
    await sleep(50);
    const timer_ms = performance.now() - t1;
    clearInterval(watch);
    return { error, error_ms, longest_gap_ms: longest, timer_ms, running: sariema.running, stats: sariema.stats().game };
  },

  /** Loses the GPU mid-game: the game's step and hash when lost and when restored. */
  async climbLoseContext() {
    await playClimb({ audio: false });
    sariema.replayGame(playLog, 300);
    const when = (type, ms) =>
      new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(`no ${type} event in ${ms} ms`)), ms);
        const off = sariema.on(type, () => {
          clearTimeout(timer);
          off();
          resolve(sariema.gameHashes());
        });
      });
    const lost = when("lost", 5000);
    const restored = when("restored", 15000);
    sariema.start();
    await sleep(50);
    const backend = sariema.backend;
    if (backend === "webgl2") {
      const ext = canvas.getContext("webgl2").getExtension("WEBGL_lose_context");
      ext.loseContext();
      await lost;
      await sleep(50);
      ext.restoreContext();
    } else {
      sariema.loseDevice();
    }
    const [atLoss, atRestore] = [await lost, await restored];
    await sleep(200);
    sariema.stop();
    const after = sariema.gameHashes();
    const frame = await sariema.capture();
    return { backend, atLoss, atRestore, after, drawn: frame.data.some((v, i) => i % 4 !== 3 && v > 30) };
  },

  async game() {
    const manifest = await sariema.loadGame(at("game/"));
    const viaGame = await sariema.capture();
    const file = sariema.pick(manifest.rooms[manifest.start], manifest.start);
    await sariema.loadRoom(at(`game/${file}`));
    const direct = await sariema.capture();
    const model = await sariema.loadGameActor("arm");
    return { start: manifest.start, file, model, ...compare(viaGame.data, direct.data), draws: sariema.stats().draws };
  },

  async audio() {
    await sariema.loadSounds(at(manifest.sounds));
    await sariema.resumeAudio();
    const before = await sariema.audioStats();
    sariema.play("beep");
    await sleep(800);
    const after = await sariema.audioStats();
    return { before, after };
  },

  async secondBank() {
    await sariema.loadSounds(at(manifest.sounds));
    await sleep(500);
    return sariema.audioStats();
  },

  async smallLead(audio) {
    await this.remount({ audio });
    await sariema.loadSounds(at(manifest.sounds));
    await sariema.resumeAudio();
    sariema.play("beep");
    await sleep(1000);
    return sariema.audioStats();
  },

  async crossOrigin(url) {
    const manifest = await sariema.loadGame(url);
    const there = await sariema.capture();
    await sariema.loadGame(at("game/"));
    const here = await sariema.capture();
    return { start: manifest.start, ...compare(there.data, here.data) };
  },

  async listen() {
    keys.length = 0;
    window.addEventListener("keydown", (e) => keys.push({ code: e.code, target: e.target.id || e.target.tagName, taken: e.defaultPrevented }));
    sariema.start();
    await sleep(50);
  },

  keys() {
    sariema.stop();
    return keys.slice();
  },

  async failedRebuild() {
    const scene = manifest.scenes.find((s) => s.name === "demo_lit");
    const bytes = await (await fetch(at(scene.room))).arrayBuffer();
    const url = URL.createObjectURL(new Blob([bytes]));
    await sariema.loadRoom(url);
    URL.revokeObjectURL(url);
    sariema.start();
    await sleep(50);
    const backend = sariema.backend;
    const failed = once("error", 8000);
    const lost = once("lost", 5000);
    if (backend === "webgl2") {
      const ext = canvas.getContext("webgl2").getExtension("WEBGL_lose_context");
      ext.loseContext();
      await lost;
      await sleep(50);
      ext.restoreContext();
    } else {
      sariema.loseDevice();
      await lost;
    }
    try {
      return { backend, error: await failed };
    } catch (e) {
      return { backend, error: null, why: e.message };
    }
  },

  async ui() {
    const description = await (await fetch(at(manifest.ui.description))).text();
    await sariema.setUi(description, { [manifest.ui.font]: at(manifest.ui.font_file) });
    uiEvents.length = 0;
    sariema.start();
    await sleep(100);
    return { width: canvas.width, height: canvas.height };
  },

  async uiEvents() {
    await sleep(200);
    sariema.stop();
    return uiEvents.slice();
  },

  async loseContext(reference) {
    sariema.start();
    await sleep(50);
    const lost = once("lost", 5000);
    const restored = once("restored", 15000);
    const backend = sariema.backend;
    if (backend === "webgl2") {
      const ext = canvas.getContext("webgl2").getExtension("WEBGL_lose_context");
      ext.loseContext();
      await lost;
      await sleep(50);
      ext.restoreContext();
    } else {
      sariema.loseDevice();
      await lost;
    }
    await restored;
    await sleep(100);
    sariema.stop();
    const frame = await sariema.capture();
    const scene = manifest.scenes.find((s) => s.name === reference);
    return { backend, ...compare(frame.data, await referencePixels(at(scene.reference))), stats: sariema.stats() };
  },
};

window.dispatchEvent(new Event("suite-ready"));
