// SPDX-License-Identifier: Apache-2.0

// Plays the interleaved stereo blocks the page mixes, in order, and tells it
// how many frames have gone out so it can keep its lead. Silence fills any
// gap; each gap counts as an underrun.

const REPORT_FRAMES = 1024;

class SariemaOut extends AudioWorkletProcessor {
  constructor() {
    super();
    this.blocks = [];
    this.offset = 0;
    this.played = 0;
    this.received = 0;
    this.underruns = 0;
    this.peak = 0;
    this.sinceReport = 0;
    this.port.onmessage = (e) => {
      if (e.data.block) {
        this.blocks.push(e.data.block);
        this.received += e.data.block.length / 2;
      } else if (e.data.flush) {
        this.blocks = [];
        this.offset = 0;
      } else if (e.data.stats) {
        this.port.postMessage({ stats: this.stats() });
      }
    };
  }

  stats() {
    const queued = this.blocks.reduce((n, b) => n + b.length / 2, 0) - this.offset / 2;
    return { played: this.played, received: this.received, queued, underruns: this.underruns, peak: this.peak };
  }

  process(_inputs, outputs) {
    const [left, right] = outputs[0];
    const frames = left.length;
    let i = 0;
    while (i < frames && this.blocks.length) {
      const block = this.blocks[0];
      while (i < frames && this.offset < block.length) {
        const l = block[this.offset];
        const r = block[this.offset + 1];
        left[i] = l;
        if (right) right[i] = r;
        const loud = Math.max(Math.abs(l), Math.abs(r));
        if (loud > this.peak) this.peak = loud;
        this.offset += 2;
        i++;
      }
      if (this.offset >= block.length) {
        this.blocks.shift();
        this.offset = 0;
      }
    }
    if (i < frames) {
      left.fill(0, i);
      right?.fill(0, i);
      if (this.received > 0) this.underruns++;
    }
    this.played += frames;
    this.sinceReport += frames;
    if (this.sinceReport >= REPORT_FRAMES) {
      this.sinceReport = 0;
      this.port.postMessage({ played: this.played });
    }
    return true;
  }
}

registerProcessor("sariema-out", SariemaOut);
