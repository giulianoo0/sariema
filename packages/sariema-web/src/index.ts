// SPDX-License-Identifier: Apache-2.0

import init, { Engine } from "./wasm/sariema_web.js";
import { fetchBytes, type Progress } from "./fetch.js";
import { AudioOut } from "./audio.js";

export type { Progress } from "./fetch.js";

/** The engine's settings; every field is optional. See docs/web.md. */
export interface Options {
  /** WebGPU where the browser has it, else WebGL2; or force one. */
  backend?: "auto" | "webgpu" | "webgl2";
  quality?: "low" | "medium" | "high";
  /** WebGPU takes the full path and WebGL2 the downlevel one unless forced. */
  path?: "auto" | "full" | "downlevel";
  msaa?: number;
  /** Physical pixels per CSS pixel at most. Default 1.5. */
  maxPixelRatio?: number;
  /** False loads the RGBA8 packs even where the GPU samples ASTC. Default true. */
  compressedTextures?: boolean;
  /** False draws no shadow maps. Default true. */
  shadows?: boolean;
  /** False turns screen-space ambient occlusion off. Default true. */
  ambientOcclusion?: boolean;
  /** Texels per row of the downlevel path's light, cluster and shadow tables. Default the tier's, 1024. */
  dataTextureWidth?: number;
  /** Rows of one actor's uniform record on the downlevel path. Default the tier's, 1024 (16 KiB). */
  actorUniformRows?: number;
  step?: { step_s?: number; max_steps_per_frame?: number };
  audio?: { lead_s?: number; block_frames?: number };
  /** Where `sariema_web_bg.wasm` is; by default next to this module. */
  wasmUrl?: string | URL;
  /** Where `audio-worklet.js` is; by default next to this module. */
  workletUrl?: string | URL;
  /** Starts the frame loop once mounted. Default true. */
  autoStart?: boolean;
}

export interface GameOptions {
  /** The game's random seed. Default 0, as `sariema run --game`. */
  seed?: number;
  /**
   * False mixes the sound with the steps instead of playing it, as a run to
   * compare with `sariema run --headless --game` needs. Default true.
   */
  audio?: boolean;
}

/** As `sariema run --headless --game` prints them. */
export interface GameHashes {
  steps: number;
  engine_hash: string;
  state_hash: string;
  physics_hash: string;
  audio_hash: string;
}

export interface UiEvent {
  step: number;
  type: "press" | "drag" | "release" | "click" | "stick" | "tap" | "held" | "focus";
  node: string | null;
  [extra: string]: unknown;
}

export interface Events {
  ui: UiEvent;
  /** The GPU was lost; the engine rebuilds itself and replays what was loaded. */
  lost: { backend: string };
  restored: { backend: string };
  error: { code: string; message: string };
  progress: Progress;
}

export interface ActorState {
  /** Model to world, 16 floats column-major. Default identity. */
  transform?: ArrayLike<number>;
  /** Clip to play; none holds the bind pose. */
  clip?: string;
  /** Clip time to start at, seconds. Default 0. */
  time?: number;
  /** Clip seconds per second; 0 holds the pose. Default 1. */
  speed?: number;
  visible?: boolean;
}

/** A pack per texture variant, as `sariema web` writes them. */
export type Variants = Partial<Record<"astc" | "rgba8", string>>;

/** A game folder's `sariema-web.json`, which `sariema web` writes. */
export interface GameManifest {
  /** A game project's pack, which carries its rooms, actors, sounds and HUD. */
  game?: Variants;
  rooms?: Record<string, Variants>;
  start?: string;
  actors?: Record<string, Variants>;
  sounds?: string;
  ui?: { description: string; files?: Record<string, string> };
}

type Source = string | URL | ArrayBuffer | Uint8Array;
type Load =
  | { kind: "room"; source: Source }
  | { kind: "actor"; source: Source }
  | { kind: "spawn"; model: number }
  | { kind: "actorState"; actor: number; state: ActorState }
  | { kind: "sounds"; source: Source }
  | { kind: "ui"; description: string; files: Record<string, Source> };

export class SariemaError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "SariemaError";
  }

  /** The engine's errors read `E_CODE: message`. */
  static from(e: unknown): SariemaError {
    if (e instanceof SariemaError) return e;
    const text = e instanceof Error ? e.message : String(e);
    const m = /^(E_[A-Z0-9_]+): ([\s\S]*)$/.exec(text);
    return m ? new SariemaError(m[1], m[2]) : new SariemaError("E_INTERNAL", text);
  }
}

const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

/** Yields to the event loop without setTimeout's 4 ms clamp. */
function nextTask(): Promise<void> {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => resolve();
    channel.port2.postMessage(null);
  });
}
let wasmReady: Promise<unknown> | null = null;

/** Mounts the engine on `canvas`, sized by its CSS box. */
export async function mount(canvas: HTMLCanvasElement, options: Options = {}): Promise<Sariema> {
  wasmReady ??= init({ module_or_path: options.wasmUrl ?? new URL("./wasm/sariema_web_bg.wasm", import.meta.url) });
  await wasmReady;
  const s = new Sariema(canvas, options);
  await s.rebuild();
  if (options.autoStart ?? true) s.start();
  return s;
}

export class Sariema {
  #canvas: HTMLCanvasElement;
  #options: Options;
  #engine: Engine | null = null;
  #raf = 0;
  #running = false;
  #loads: Load[] = [];
  #listeners = new Map<keyof Events, Set<(e: never) => void>>();
  #audio: AudioOut | null = null;
  /** The game pack playing, which a new GPU loads its room and actors from. */
  #gamePack: Uint8Array | null = null;
  #detach: (() => void)[] = [];
  backend = "";

  constructor(canvas: HTMLCanvasElement, options: Options) {
    this.#canvas = canvas;
    this.#options = options;
    this.#listen();
  }

  get running(): boolean {
    return this.#running;
  }

  #settings(): string {
    const o = this.#options;
    return JSON.stringify({
      backend: o.backend,
      quality: o.quality,
      path: o.path,
      msaa: o.msaa,
      max_pixel_ratio: o.maxPixelRatio,
      compressed_textures: o.compressedTextures,
      shadows: o.shadows,
      ambient_occlusion: o.ambientOcclusion,
      data_texture_width: o.dataTextureWidth,
      actor_uniform_rows: o.actorUniformRows,
      step: o.step,
      audio: o.audio,
    });
  }

  #fit(engine: Engine): void {
    const css = this.#canvas.getBoundingClientRect();
    const [w, h] = engine.physical_size(css.width, css.height, devicePixelRatio);
    if (this.#canvas.width !== w || this.#canvas.height !== h) {
      this.#canvas.width = w;
      this.#canvas.height = h;
    }
    engine.resize(w, h, css.width || w);
  }

  /**
   * Makes the engine anew on the canvas and replays every load; a game goes
   * on at the step it had reached.
   */
  async rebuild(): Promise<void> {
    const old = this.#engine;
    this.#engine = null;
    const css = this.#canvas.getBoundingClientRect();
    const [width, height] = [css.width || this.#canvas.width, css.height || this.#canvas.height];
    let engine: Engine;
    try {
      engine =
        old && this.#gamePack
          ? await old.restored(this.#canvas, width, height, devicePixelRatio, this.#gamePack)
          : (old?.free(), await Engine.create(this.#canvas, this.#settings(), width, height, devicePixelRatio));
    } catch (e) {
      this.#gamePack = null;
      throw SariemaError.from(e);
    }
    this.#fit(engine);
    this.#engine = engine;
    this.backend = engine.backend();
    if (this.#gamePack) return;
    this.#audio?.restart();
    const loads = this.#loads;
    this.#loads = [];
    for (const load of loads) await this.#replay(load);
  }

  async #replay(load: Load): Promise<void> {
    switch (load.kind) {
      case "room":
        return this.loadRoom(load.source);
      case "actor":
        await this.loadActor(load.source);
        return;
      case "spawn":
        this.spawnActor(load.model);
        return;
      case "actorState":
        return this.setActor(load.actor, load.state);
      case "sounds":
        return this.loadSounds(load.source);
      case "ui":
        return this.setUi(load.description, load.files);
    }
  }

  #call<T>(f: (e: Engine) => T): T {
    if (!this.#engine) throw new SariemaError("E_USAGE", "the engine is not running");
    try {
      return f(this.#engine);
    } catch (e) {
      throw SariemaError.from(e);
    }
  }

  /** The backend, adapter, limits, compressed formats, path and texture variants. */
  capabilities(): Record<string, unknown> {
    return JSON.parse(this.#call((e) => e.capabilities()));
  }

  async #bytes(source: Source): Promise<Uint8Array> {
    if (source instanceof Uint8Array) return source;
    if (source instanceof ArrayBuffer) return new Uint8Array(source);
    return fetchBytes(source, (p) => this.#emit("progress", p));
  }

  async loadRoom(source: Source): Promise<void> {
    const bytes = await this.#bytes(source);
    this.#call((e) => e.load_room(bytes));
    this.#loads = this.#loads.filter((l) => l.kind !== "room");
    this.#loads.push({ kind: "room", source });
  }

  /** The file this device should load of `files`: ASTC where it samples ASTC, else RGBA8. */
  pick(files: Variants | undefined, what: string): string {
    const variants = this.capabilities().texture_variants as ("astc" | "rgba8")[];
    const variant = variants.find((v) => files?.[v]);
    if (!variant) throw new SariemaError("E_USAGE", `${what} has no pack in ${variants.join(" or ")}`);
    return files![variant]!;
  }

  #game: { root: URL; manifest: GameManifest } | null = null;

  /**
   * Loads a game folder: a game project's pack, played from its first step,
   * or else the manifest's start room, sounds and UI.
   */
  async loadGame(base: string | URL, options: GameOptions = {}): Promise<GameManifest> {
    const root = new URL(base, location.href);
    if (!root.pathname.endsWith("/")) root.pathname += "/";
    const manifest = JSON.parse(new TextDecoder().decode(await fetchBytes(new URL("sariema-web.json", root)))) as GameManifest;
    this.#game = { root, manifest };
    if (manifest.game) {
      await this.playGame(new URL(this.pick(manifest.game, "the game"), root), options);
      return manifest;
    }
    const start = manifest.start ?? Object.keys(manifest.rooms ?? {})[0];
    if (start) await this.enterRoom(start);
    if (manifest.sounds) await this.loadSounds(new URL(manifest.sounds, root));
    if (manifest.ui) {
      const description = new TextDecoder().decode(await fetchBytes(new URL(manifest.ui.description, root)));
      const files: Record<string, Source> = {};
      for (const [name, file] of Object.entries(manifest.ui.files ?? {})) files[name] = new URL(file, root);
      await this.setUi(description, files);
    }
    return manifest;
  }

  #inGame(): { root: URL; manifest: GameManifest } {
    if (!this.#game) throw new SariemaError("E_USAGE", "no game is loaded");
    return this.#game;
  }

  /** Loads room `id` of the loaded game in this device's texture variant. */
  async enterRoom(id: string): Promise<void> {
    const { root, manifest } = this.#inGame();
    await this.loadRoom(new URL(this.pick(manifest.rooms?.[id], `room ${id}`), root));
  }

  /**
   * Plays game pack `source` (`.srmg`) from its first step, in place of any
   * room or actors shown: its logic, collision, animation, sound and HUD.
   */
  async playGame(source: Source, options: GameOptions = {}): Promise<void> {
    const bytes = await this.#bytes(source);
    let rate: number | undefined;
    if (options.audio ?? true) {
      rate = await this.#output().open();
      this.#audio!.restart();
    }
    this.#call((e) => e.load_game(bytes, options.seed ?? 0, rate));
    this.#gamePack = bytes;
  }

  /**
   * Steps the game to step `to` with input log `log` (its JSON or the
   * object), as `sariema run --headless --game --input` does; returns the
   * hashes that prints.
   */
  replayGame(log: string | object, to: number): GameHashes {
    const text = typeof log === "string" ? log : JSON.stringify(log);
    this.#call((e) => e.replay_game(text, to));
    return this.gameHashes()!;
  }

  /** The game's steps and hashes; null with no game playing. */
  gameHashes(): GameHashes | null {
    const json = this.#call((e) => e.game_hashes());
    return json ? JSON.parse(json) : null;
  }

  /** Loads actor `id` of the loaded game; returns the model to spawn. */
  async loadGameActor(id: string): Promise<number> {
    const { root, manifest } = this.#inGame();
    return this.loadActor(new URL(this.pick(manifest.actors?.[id], `actor ${id}`), root));
  }

  /** Loads an actor pack; returns the model to spawn actors of. */
  async loadActor(source: Source): Promise<number> {
    const bytes = await this.#bytes(source);
    const model = this.#call((e) => e.load_actor(bytes));
    this.#loads.push({ kind: "actor", source });
    return model;
  }

  spawnActor(model: number): number {
    const actor = this.#call((e) => e.spawn_actor(model));
    this.#loads.push({ kind: "spawn", model });
    return actor;
  }

  setActor(actor: number, state: ActorState): void {
    const t = new Float32Array(state.transform ?? IDENTITY);
    this.#call((e) =>
      e.set_actor(actor, t, state.clip ?? "", state.time ?? 0, state.speed ?? 1, state.visible ?? true),
    );
    this.#loads = this.#loads.filter((l) => !(l.kind === "actorState" && l.actor === actor));
    this.#loads.push({ kind: "actorState", actor, state });
  }

  #output(): AudioOut {
    this.#audio ??= new AudioOut(
      this.#options.workletUrl ?? new URL("./audio-worklet.js", import.meta.url),
      this.#call((e) => e.audio_block_frames()),
      (played) => this.#engine?.mix(played) ?? new Float32Array(),
    );
    return this.#audio;
  }

  /** Loads a sound bank (`.srms`), mixed at the page's audio rate. */
  async loadSounds(source: Source): Promise<void> {
    const bytes = await this.#bytes(source);
    const rate = await this.#output().open();
    this.#audio!.restart();
    this.#call((e) => e.load_sounds(bytes, rate));
    this.#loads = this.#loads.filter((l) => l.kind !== "sounds");
    this.#loads.push({ kind: "sounds", source });
  }

  /** Starts the audio output; call it from a user gesture where the page requires one. */
  async resumeAudio(): Promise<void> {
    await this.#audio?.resume();
  }

  play(sound: string): void {
    this.#call((e) => e.play(sound));
  }

  /** What the audio worklet has played and received, for tests and diagnostics. */
  audioStats(): Promise<Record<string, number> | null> {
    return this.#audio?.stats() ?? Promise.resolve(null);
  }

  /** Builds the UI of `description` (JSON); `files` gives its fonts and images by name. */
  async setUi(description: string | object, files: Record<string, Source> = {}): Promise<void> {
    const text = typeof description === "string" ? description : JSON.stringify(description);
    for (const [name, source] of Object.entries(files)) {
      const bytes = await this.#bytes(source);
      this.#call((e) => e.add_file(name, bytes));
    }
    this.#call((e) => e.set_ui(text));
    this.#loads = this.#loads.filter((l) => l.kind !== "ui");
    this.#loads.push({ kind: "ui", description: text, files });
  }

  on<K extends keyof Events>(type: K, listener: (e: Events[K]) => void): () => void {
    let set = this.#listeners.get(type);
    if (!set) this.#listeners.set(type, (set = new Set()));
    set.add(listener as (e: never) => void);
    return () => set!.delete(listener as (e: never) => void);
  }

  #emit<K extends keyof Events>(type: K, event: Events[K]): void {
    for (const l of this.#listeners.get(type) ?? []) (l as (e: Events[K]) => void)(event);
  }

  start(): void {
    if (this.#running) return;
    this.#running = true;
    const tick = (now: number) => {
      if (!this.#running) return;
      this.#raf = requestAnimationFrame(tick);
      this.#frame(now);
    };
    this.#raf = requestAnimationFrame(tick);
  }

  /** One frame at `now` (ms), for a page that runs its own loop instead of `start()`. */
  frame(now: number = performance.now()): void {
    this.#frame(now);
  }

  stop(): void {
    this.#running = false;
    cancelAnimationFrame(this.#raf);
    this.#engine?.pause();
  }

  #frame(now: number): void {
    const engine = this.#engine;
    if (!engine) return;
    let steps: number;
    try {
      steps = engine.frame(now);
    } catch (e) {
      this.#fail(e);
      return;
    }
    if (steps < 0) {
      const wasRunning = this.#running;
      this.stop();
      this.#emit("lost", { backend: this.backend });
      void this.#restore(wasRunning);
      return;
    }
    const events = steps > 0 ? engine.take_events() : undefined;
    if (events) for (const event of JSON.parse(events) as UiEvent[]) this.#emit("ui", event);
  }

  #fail(e: unknown): void {
    const err = SariemaError.from(e);
    this.stop();
    this.#emit("error", { code: err.code, message: err.message });
  }

  async #restore(start: boolean): Promise<void> {
    try {
      await this.rebuild();
    } catch (e) {
      this.#fail(e);
      return;
    }
    this.#emit("restored", { backend: this.backend });
    if (start) this.start();
  }

  /** Draws the current frame offscreen and reads it back as RGBA rows. */
  async capture(): Promise<{ width: number; height: number; data: Uint8Array }> {
    this.#call((e) => e.capture(1));
    const data = await this.#readBack();
    return { width: this.#canvas.width, height: this.#canvas.height, data };
  }

  async #readBack(): Promise<Uint8Array> {
    const deadline = performance.now() + 10_000;
    while (performance.now() < deadline) {
      const bytes = this.#call((e) => e.take_capture());
      if (bytes) return bytes;
      await nextTask();
    }
    throw new SariemaError("E_GPU", "the frame was not read back within 10 s");
  }

  /**
   * Times `frames` draws of the current frame back to back, after one read
   * back that drains the GPU and up to the read back of the last: per
   * frame, the CPU to encode and submit, and the wall time with the GPU.
   */
  async bench(frames: number): Promise<{ frames: number; cpu_ms: number; wall_ms: number }> {
    await this.capture();
    const start = performance.now();
    this.#call((e) => e.capture(frames));
    const submitted = performance.now();
    await this.#readBack();
    const end = performance.now();
    return { frames, cpu_ms: (submitted - start) / frames, wall_ms: (end - start) / frames };
  }

  /** The last frame's steps, path, passes, draws and uploads. */
  stats(): Record<string, unknown> {
    return JSON.parse(this.#call((e) => e.stats()));
  }

  /** Destroys the device as a lost one would be, to exercise recovery. */
  loseDevice(): void {
    this.#call((e) => e.lose_device());
  }

  #listen(): void {
    const c = this.#canvas;
    c.style.touchAction = "none";
    const add = <K extends keyof HTMLElementEventMap>(
      target: HTMLElement | Window | Document,
      type: K | string,
      f: (e: never) => void,
      opts?: AddEventListenerOptions,
    ) => {
      target.addEventListener(type, f as EventListener, opts);
      this.#detach.push(() => target.removeEventListener(type, f as EventListener, opts));
    };
    const pointer = (e: PointerEvent) => {
      if (e.type === "pointerdown") {
        // Throws for a pointer the browser does not know, as a synthetic one.
        try {
          c.setPointerCapture(e.pointerId);
        } catch {}
        void this.#audio?.resume();
      }
      this.#engine?.pointer(e.type, e.pointerId, e.offsetX, e.offsetY);
    };
    for (const type of ["pointerdown", "pointermove", "pointerup", "pointercancel"]) add(c, type, pointer);
    // Keys are the engine's while nothing else on the page has the focus;
    // a key released elsewhere still reaches it, so none stays held.
    const key = (e: KeyboardEvent) => {
      const ours = e.target === document.body || e.target === document.documentElement || e.target === c;
      if (!ours && e.type === "keydown") return;
      if (this.#engine?.key(e.code, e.type === "keydown") && ours) e.preventDefault();
    };
    add(window, "keydown", key);
    add(window, "keyup", key);
    const resize = () => this.#engine && this.#fit(this.#engine);
    const observer = new ResizeObserver(resize);
    observer.observe(c);
    this.#detach.push(() => observer.disconnect());
    const watchRatio = () => {
      const query = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
      const changed = () => {
        resize();
        watchRatio();
      };
      query.addEventListener("change", changed, { once: true });
      this.#detach.push(() => query.removeEventListener("change", changed));
    };
    watchRatio();
    let wasRunning = false;
    add(document, "visibilitychange", () => {
      if (document.hidden) {
        wasRunning = this.#running;
        this.stop();
        void this.#audio?.suspend();
      } else if (wasRunning) {
        this.start();
        void this.#audio?.resume();
      }
    });
    add(c, "webglcontextlost", (e: Event) => {
      e.preventDefault();
      this.stop();
      this.#emit("lost", { backend: this.backend });
    });
    add(c, "webglcontextrestored", () => void this.#restore(true));
  }

  /** Stops everything and lets go of the GPU and the audio. */
  destroy(): void {
    this.stop();
    for (const d of this.#detach) d();
    this.#detach = [];
    this.#engine?.free();
    this.#engine = null;
    this.#gamePack = null;
    void this.#audio?.close();
    this.#audio = null;
  }
}
