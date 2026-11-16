// SPDX-License-Identifier: Apache-2.0

/**
 * The page's audio output: an AudioWorklet that plays what the engine mixes
 * on the main thread. The worklet reports the frames it has played; each
 * report asks the engine for the blocks that keep its lead and posts them
 * across. No SharedArrayBuffer, so no cross-origin isolation is needed.
 */
export class AudioOut {
  #context: AudioContext | null = null;
  #node: AudioWorkletNode | null = null;
  #opening: Promise<number> | null = null;
  #pending: ((s: Record<string, number>) => void)[] = [];
  #played = 0;
  #base = 0;

  constructor(
    readonly workletUrl: string | URL,
    readonly blockFrames: number,
    readonly pull: (played: number) => Float32Array,
  ) {}

  /** Opens the output once; returns its sample rate. */
  open(): Promise<number> {
    this.#opening ??= this.#open();
    return this.#opening;
  }

  async #open(): Promise<number> {
    const context = new AudioContext({ latencyHint: "interactive" });
    await context.audioWorklet.addModule(String(this.workletUrl));
    const node = new AudioWorkletNode(context, "sariema-out", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { reportFrames: this.blockFrames },
    });
    node.port.onmessage = (e: MessageEvent) => {
      if (e.data.stats) {
        for (const resolve of this.#pending.splice(0)) resolve(e.data.stats);
      } else if (typeof e.data.played === "number") {
        this.#played = e.data.played;
        this.#send(this.pull(this.#played - this.#base));
      }
    };
    node.connect(context.destination);
    this.#context = context;
    this.#node = node;
    this.#send(this.pull(0));
    return context.sampleRate;
  }

  #send(blocks: Float32Array): void {
    if (blocks.length) this.#node?.port.postMessage({ block: blocks }, [blocks.buffer]);
  }

  /** Drops what the old engine queued; a new one counts frames from here. */
  restart(): void {
    this.#base = this.#played;
    this.#node?.port.postMessage({ flush: true });
  }

  async resume(): Promise<void> {
    if (this.#context?.state === "suspended") await this.#context.resume();
  }

  async suspend(): Promise<void> {
    if (this.#context?.state === "running") await this.#context.suspend();
  }

  /** Frames played and received, frames queued, underruns, the loudest sample and the rate. */
  stats(): Promise<Record<string, number> | null> {
    if (!this.#node) return Promise.resolve(null);
    return new Promise((resolve) => {
      this.#pending.push(resolve);
      this.#node!.port.postMessage({ stats: true });
    });
  }

  get state(): string {
    return this.#context?.state ?? "closed";
  }

  async close(): Promise<void> {
    this.#node?.disconnect();
    await this.#context?.close();
    this.#context = null;
    this.#node = null;
    this.#opening = null;
  }
}
