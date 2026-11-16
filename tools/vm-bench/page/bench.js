// SPDX-License-Identifier: Apache-2.0

// Recorded ticks run three ways: wasmi inside the page's wasm (A), the
// browser's own WebAssembly with injected gas (B) and without (B0, the
// floor). The fuel is VmSettings' default.
const FIRST = 200_000_000n;
const PER = 20_000_000n;

function parseTicks(b) {
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const ticks = [];
  let at = 0;
  while (at < b.length) {
    const n = v.getUint32(at, true);
    at += 4;
    const events = b.slice(at, at + n);
    at += n;
    const m = v.getUint32(at, true);
    at += 4;
    const ends = new Uint32Array(m);
    for (let i = 0; i < m; i++, at += 4) ends[i] = v.getUint32(at, true);
    ticks.push({ events, ends });
  }
  return ticks;
}

const bytes = async (url) => new Uint8Array(await (await fetch(url)).arrayBuffer());

/** The `sariema` imports over JavaScript, as sariema-vm's host.rs. */
function host(buttons) {
  const s = { mem: null, view: null, t: null, cursor: 0, out: new Uint8Array(1 << 16), outLen: 0, regions: [] };
  const view = () => {
    if (!s.view || s.view.buffer !== s.mem.buffer) s.view = new Uint8Array(s.mem.buffer);
    return s.view;
  };
  const range = (m, ptr, len, what) => {
    if (ptr + len > m.length) throw new Error(`${what}: outside memory`);
  };
  const sariema = {
    submit(ptr, len) {
      const m = view();
      range(m, ptr, len, "submit");
      if (s.outLen + len > s.out.length) {
        const bigger = new Uint8Array(Math.max(s.out.length * 2, s.outLen + len));
        bigger.set(s.out.subarray(0, s.outLen));
        s.out = bigger;
      }
      s.out.set(m.subarray(ptr, ptr + len), s.outLen);
      s.outLen += len;
    },
    events(dst, cap) {
      const m = view();
      range(m, dst, cap, "events");
      const { events, ends } = s.t;
      const start = s.cursor === 0 ? 0 : ends[s.cursor - 1];
      let end = start;
      let next = s.cursor;
      while (next < ends.length && ends[next] - start <= cap) end = ends[next++];
      if (next === s.cursor && next < ends.length) return -(ends[next] - start);
      m.set(events.subarray(start, end), dst);
      s.cursor = next;
      return end - start;
    },
    register(kind, ptr, len) {
      if (kind !== 1 || ptr + len > view().length || s.regions.length >= 64) return -1;
      s.regions.push([kind, ptr, len]);
      return s.regions.length - 1;
    },
    log() {},
    button(name) {
      return buttons.indexOf(name >>> 0);
    },
  };
  return { s, imports: { sariema } };
}

async function browserRunner(module, ticks, buttons, copies, metered) {
  const engineSide = new Uint8Array(1 << 16);
  const machines = [];
  for (let c = 0; c < copies; c++) {
    const { s, imports } = host(buttons);
    const instance = await WebAssembly.instantiate(module, imports);
    s.mem = instance.exports.memory;
    machines.push({ s, tick: instance.exports.sariema_tick, gas: instance.exports.sariema_gas });
  }
  return () => {
    let bytes = 0;
    for (const m of machines) {
      for (let i = 0; i < ticks.length; i++) {
        m.s.t = ticks[i];
        m.s.cursor = 0;
        m.s.outLen = 0;
        if (metered) m.gas.value = i === 0 ? FIRST : PER;
        m.tick();
        engineSide.set(m.s.out.subarray(0, m.s.outLen));
        bytes += m.s.outLen;
      }
    }
    return bytes;
  };
}

async function measure(make, n, reps) {
  const per = [];
  let bytes = 0;
  for (let r = 0; r < reps; r++) {
    const run = await make();
    const t0 = performance.now();
    bytes = run();
    per.push((performance.now() - t0) / n);
  }
  return { per_tick_ms: per, bytes };
}

window.run = async (reps = 7) => {
  const result = { ua: navigator.userAgent, workloads: {} };
  const a = await import("../target/bench/a/vm_bench.js");
  await a.default({ module_or_path: new URL("../target/bench/a/vm_bench_bg.wasm", import.meta.url) });
  for (const [name, copies] of [["climb", 10], ["synthetic", 1]]) {
    const wasm = await bytes(`../target/bench/data/${name}.wasm`);
    const gasWasm = await bytes(`../target/bench/data/${name}.gas.wasm`);
    const tickBytes = await bytes(`../target/bench/data/${name}.ticks`);
    const buttons = Uint32Array.from(await (await fetch(`../target/bench/data/${name}.buttons.json`)).json());
    const ticks = parseTicks(tickBytes);
    const n = ticks.length * copies;
    const plain = await WebAssembly.compile(wasm);
    const metered = await WebAssembly.compile(gasWasm);
    const w = (result.workloads[name] = { ticks: ticks.length, copies });
    for (const [label, lib] of [["A", a]]) {
      w[label] = await measure(async () => {
        const b = new lib.Bench(wasm, buttons, tickBytes, copies);
        return () => b.run();
      }, n, reps);
    }
    w.B = await measure(() => browserRunner(metered, ticks, Array.from(buttons), copies, true), n, reps);
    w.B0 = await measure(() => browserRunner(plain, ticks, Array.from(buttons), copies, false), n, reps);
  }
  // A tick that never returns: how long each way takes to stop it on the first tick's budget.
  const forever = await bytes("../target/bench/data/forever.wasm");
  const stop = {};
  for (const [label, lib] of [["A", a]]) {
    const b = new lib.Bench(forever, new Uint32Array(), new Uint8Array(8), 1);
    const t0 = performance.now();
    try {
      b.run();
      stop[label] = { error: null };
    } catch (e) {
      stop[label] = { error: String(e.message ?? e).slice(0, 80), ms: performance.now() - t0 };
    }
  }
  const run = await browserRunner(await WebAssembly.compile(await bytes("../target/bench/data/forever.gas.wasm")), [{ events: new Uint8Array(), ends: new Uint32Array() }], [], 1, true);
  const t0 = performance.now();
  try {
    run();
    stop.B = { error: null };
  } catch (e) {
    stop.B = { error: String(e.message ?? e).slice(0, 80), ms: performance.now() - t0 };
  }
  result.stop_first_tick = stop;
  document.getElementById("out").textContent = JSON.stringify(result, null, 1);
  return result;
};
