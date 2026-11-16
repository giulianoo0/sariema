// SPDX-License-Identifier: Apache-2.0

"use client";

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/fui/button";
import { IconPause, IconPlay } from "@/components/fui/icons-base";
import styles from "./demo.module.css";

/** The part of @sariema/web's handle the landing uses (packages/sariema-web/src/index.ts). */
interface Engine {
  backend: string;
  loadGame(folder: string, options?: { audio?: boolean }): Promise<unknown>;
  start(): void;
  stop(): void;
  destroy(): void;
}

type Phase = "still" | "loading" | "ready" | "playing" | "paused";

const ENGINE = "/engine/index.js";
const GAME = "/engine/demo/";

function graphics(): boolean {
  if ("gpu" in navigator) return true;
  return !!document.createElement("canvas").getContext("webgl2");
}

/**
 * examples/climb in the hero, drawn by the engine compiled to WebAssembly.
 * Until it draws, and where it cannot, the same frame rendered natively shows.
 */
export function Demo() {
  const canvas = useRef<HTMLCanvasElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const engine = useRef<Engine | null>(null);
  const [phase, setPhase] = useState<Phase>("still");
  const [backend, setBackend] = useState("");
  const [note, setNote] = useState("");
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  const pending = useRef<Promise<boolean> | null>(null);
  const load = () => (pending.current ??= mountEngine());

  async function mountEngine(): Promise<boolean> {
    if (!graphics()) {
      setNote("This browser has neither WebGPU nor WebGL2: the frame above was drawn natively.");
      return false;
    }
    setPhase("loading");
    try {
      if (!(await fetch(ENGINE, { method: "HEAD" })).ok) throw new Error("no engine build");
      const { mount } = await import(/* webpackIgnore: true */ /* turbopackIgnore: true */ ENGINE);
      const s: Engine = await mount(canvas.current!, { quality: "medium", autoStart: false });
      await s.loadGame(GAME, { audio: false });
      engine.current = s;
      setBackend(s.backend === "webgpu" ? "WebGPU" : "WebGL2");
      return true;
    } catch (e) {
      setNote(`The engine did not start here (${(e as { code?: string }).code ?? "load"}): the frame above was drawn natively.`);
      setPhase("still");
      return false;
    }
  }

  useEffect(() => {
    const el = frame.current!;
    const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const seen = new IntersectionObserver(([entry]) => {
      const s = engine.current;
      if (!entry.isIntersecting) {
        s?.stop();
        return;
      }
      if (s && phaseRef.current !== "paused") s.start();
      if (!s && !reduced) {
        void load().then((ok) => {
          if (!ok || phaseRef.current !== "loading") return;
          engine.current!.start();
          setPhase("ready");
        });
      }
    });
    // The engine is megabytes: fetch it once the page itself has loaded.
    const begin = () => seen.observe(el);
    if (document.readyState === "complete") begin();
    else window.addEventListener("load", begin, { once: true });

    // @sariema/web takes the game's keys whenever the page itself has the
    // focus, which would steal the arrows and Space from scrolling: here the
    // game only hears them once its canvas has the focus.
    const guard = (e: KeyboardEvent) => {
      const page = e.target === document.body || e.target === document.documentElement;
      if (page && document.activeElement !== canvas.current) e.stopPropagation();
    };
    window.addEventListener("keydown", guard, { capture: true });
    return () => {
      window.removeEventListener("load", begin);
      seen.disconnect();
      window.removeEventListener("keydown", guard, { capture: true });
      engine.current?.destroy();
      engine.current = null;
    };
  }, []);

  async function play() {
    if (!(await load())) return;
    engine.current!.start();
    setPhase("playing");
    canvas.current!.focus({ preventScroll: true });
  }

  function pause() {
    engine.current?.stop();
    setPhase("paused");
  }

  const drawn = phase === "ready" || phase === "playing" || phase === "paused";
  return (
    <figure className={styles.figure}>
      <div ref={frame} className={styles.frame} data-phase={phase}>
        <canvas
          ref={canvas}
          className={styles.canvas}
          tabIndex={0}
          aria-label="The climb example game. WASD or the arrow keys walk, E waves."
        />
        <img className={styles.still} src="/climb-still.webp" alt="" data-hidden={drawn || undefined} />
        {phase !== "playing" && (
          <div className={styles.scrim}>
            <Button variant="secondary" size="lg" icon={<IconPlay size={16} />} loading={phase === "loading"} onClick={play}>
              {phase === "paused" ? "Resume" : "Play the demo"}
            </Button>
          </div>
        )}
      </div>
      <figcaption className={styles.caption}>
        <span className={styles.label}>
          <code>examples/climb</code>
          <span className={styles.chip} data-live={drawn || undefined}>
            {drawn ? `Live · ${backend}` : "Still frame"}
          </span>
        </span>
        {phase === "playing" ? (
          <span className={styles.controls}>
            <span className={styles.hint}>WASD or arrows to walk · E to wave</span>
            <Button variant="ghost" size="sm" icon={<IconPause size={16} />} onClick={pause}>
              Pause
            </Button>
          </span>
        ) : (
          <span className={styles.hint}>{note || "The engine in your browser, as WebAssembly"}</span>
        )}
      </figcaption>
    </figure>
  );
}
