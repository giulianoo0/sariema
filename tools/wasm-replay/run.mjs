// SPDX-License-Identifier: Apache-2.0

// node run.mjs <wasm_replay.wasm> <pack.srmg> <log.json> <steps>
// Replays the game under this Node's WebAssembly and prints its hashes as
// JSON, named as `sariema run --headless --game --json` names them.

import { readFileSync } from "node:fs";

const [wasmPath, packPath, logPath, steps] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const { memory, alloc, replay, hashes } = instance.exports;

function place(bytes) {
  const ptr = alloc(bytes.length);
  new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
  return ptr;
}

const pack = readFileSync(packPath);
const log = readFileSync(logPath);
const started = performance.now();
const code = replay(place(pack), pack.length, place(log), log.length, BigInt(steps));
const ms = performance.now() - started;
if (code !== 0) {
  console.log(JSON.stringify({ ok: false, error: { code: "E_REPLAY", message: `replay returned ${code}` } }));
  process.exit(1);
}
const hex = (v) => v.toString(16).padStart(16, "0");
const out = new BigUint64Array(memory.buffer, hashes(), 4);
console.log(JSON.stringify({
  ok: true,
  runtime: `node ${process.version} WebAssembly`,
  steps: Number(steps),
  engine_hash: hex(out[0]),
  state_hash: hex(out[1]),
  physics_hash: hex(out[2]),
  audio_hash: hex(out[3]),
  total_ms: ms,
}));
