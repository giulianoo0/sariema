// SPDX-License-Identifier: Apache-2.0

// Reads what the browser offers without the engine, then on request mounts
// the engine on the demo game and plays a chime through the worklet. Every
// finding lands in one JSON object, shown and copied as it is; `?auto`
// runs the smoke test at once and leaves the result on window.probeResult.

const result = { probe: 1, at: new Date().toISOString() };
const summary = document.getElementById("summary");
const out = document.getElementById("json");

// From wasm-feature-detect (Apache-2.0): a v128 constant, and an atomic load
// on shared memory.
const SIMD = [0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11];
const THREADS = [0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 5, 4, 1, 3, 1, 1, 10, 11, 1, 9, 0, 65, 0, 254, 16, 2, 0, 26, 11];

const WEBGL_EXTENSIONS = [
  "EXT_color_buffer_float",
  "EXT_color_buffer_half_float",
  "EXT_float_blend",
  "OES_texture_float_linear",
  "WEBGL_compressed_texture_astc",
  "WEBGL_compressed_texture_etc",
  "WEBGL_compressed_texture_s3tc",
  "WEBGL_compressed_texture_s3tc_srgb",
  "EXT_texture_compression_bptc",
  "EXT_texture_compression_rgtc",
  "EXT_disjoint_timer_query_webgl2",
  "WEBGL_lose_context",
  "OVR_multiview2",
];

function show() {
  out.textContent = JSON.stringify(result, null, 2);
  const rows = [
    ["Secure context", result.page.secure_context],
    ["WebGPU", result.webgpu.available],
    ["WebGL2", result.webgl2.available],
    ["Float render targets (WebGL2)", result.webgl2.float_render_target],
    ["ASTC", result.webgpu.features?.includes("texture-compression-astc") || result.webgl2.extensions?.WEBGL_compressed_texture_astc],
    ["AudioWorklet", result.audio.worklet],
    ["WASM SIMD", result.wasm.simd],
    ["WASM threads", result.wasm.threads],
    ["Device pixel ratio", result.page.device_pixel_ratio],
  ];
  if (result.smoke) {
    rows.push(["Engine", result.smoke.ok ? `${result.smoke.backend}, ${result.smoke.path}` : result.smoke.error]);
    rows.push(["First frame", result.smoke.first_frame_ms != null ? `${Math.round(result.smoke.first_frame_ms)} ms` : "—"]);
    rows.push(["Chime through the worklet", result.smoke.audio_ok]);
  }
  summary.replaceChildren(
    ...rows.map(([name, value]) => {
      const tr = document.createElement("tr");
      const a = document.createElement("td");
      const b = document.createElement("td");
      a.textContent = name;
      b.textContent = typeof value === "boolean" ? (value ? "yes" : "no") : String(value ?? "—");
      if (typeof value === "boolean") b.className = value ? "ok" : "no";
      tr.append(a, b);
      return tr;
    }),
  );
}

function page() {
  return {
    user_agent: navigator.userAgent,
    secure_context: isSecureContext,
    cross_origin_isolated: self.crossOriginIsolated ?? false,
    device_pixel_ratio: devicePixelRatio,
    viewport: [innerWidth, innerHeight],
    screen: [screen.width, screen.height],
    hardware_concurrency: navigator.hardwareConcurrency ?? null,
    device_memory_gb: navigator.deviceMemory ?? null,
  };
}

async function webgpu() {
  if (!navigator.gpu) return { available: false, why: isSecureContext ? "no navigator.gpu" : "not a secure context" };
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
  if (!adapter) return { available: false, why: "no adapter" };
  const info = adapter.info ?? {};
  const limits = {};
  for (const key of [
    "maxTextureDimension2D",
    "maxBindGroups",
    "maxStorageBuffersPerShaderStage",
    "maxUniformBufferBindingSize",
    "maxStorageBufferBindingSize",
    "maxBufferSize",
    "maxVertexAttributes",
    "maxColorAttachments",
    "maxSampledTexturesPerShaderStage",
  ]) {
    limits[key] = adapter.limits[key];
  }
  return {
    available: true,
    vendor: info.vendor ?? "",
    architecture: info.architecture ?? "",
    description: info.description ?? "",
    preferred_format: navigator.gpu.getPreferredCanvasFormat(),
    features: [...adapter.features].sort(),
    limits,
  };
}

function webgl2() {
  const gl = document.createElement("canvas").getContext("webgl2");
  if (!gl) return { available: false };
  const extensions = {};
  for (const name of WEBGL_EXTENSIONS) extensions[name] = gl.getExtension(name) !== null;
  const debug = gl.getExtension("WEBGL_debug_renderer_info");
  const tex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA16F, 4, 4);
  const fb = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  const floatTarget = gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE;
  const floatSamples = extensions.EXT_color_buffer_float
    ? [...gl.getInternalformatParameter(gl.RENDERBUFFER, gl.RGBA16F, gl.SAMPLES)]
    : [];
  gl.getExtension("WEBGL_lose_context")?.loseContext();
  return {
    available: true,
    renderer: debug ? gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
    version: gl.getParameter(gl.VERSION),
    shading_language: gl.getParameter(gl.SHADING_LANGUAGE_VERSION),
    max_texture_size: gl.getParameter(gl.MAX_TEXTURE_SIZE),
    max_uniform_block_size: gl.getParameter(gl.MAX_UNIFORM_BLOCK_SIZE),
    max_vertex_attribs: gl.getParameter(gl.MAX_VERTEX_ATTRIBS),
    max_samples: gl.getParameter(gl.MAX_SAMPLES),
    max_texture_units: gl.getParameter(gl.MAX_COMBINED_TEXTURE_IMAGE_UNITS),
    float_render_target: floatTarget,
    rgba16f_sample_counts: floatSamples,
    extensions,
  };
}

function wasm() {
  return {
    available: typeof WebAssembly === "object",
    simd: WebAssembly.validate(new Uint8Array(SIMD)),
    threads: WebAssembly.validate(new Uint8Array(THREADS)) && typeof SharedArrayBuffer === "function",
    shared_array_buffer: typeof SharedArrayBuffer === "function",
  };
}

function audio() {
  const Context = self.AudioContext ?? self.webkitAudioContext;
  if (!Context) return { available: false, worklet: false };
  return {
    available: true,
    worklet: typeof AudioWorkletNode === "function" && "audioWorklet" in Context.prototype,
  };
}

async function smoke() {
  const s = { ok: false };
  result.smoke = s;
  const t0 = performance.now();
  try {
    const { mount } = await import("../index.js");
    const canvas = document.getElementById("stage");
    const sariema = await mount(canvas, { autoStart: false });
    s.mount_ms = performance.now() - t0;
    Object.assign(s, { backend: sariema.backend, ...pick(sariema.capabilities(), ["path", "msaa", "texture_variants", "format"]) });
    const t1 = performance.now();
    const manifest = await sariema.loadGame(new URL("../demo/", import.meta.url));
    s.load_ms = performance.now() - t1;
    s.room_file = sariema.pick(manifest.rooms[manifest.start], manifest.start);
    sariema.start();
    s.first_frame_ms = await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now() - t0))));
    sariema.stop();
    const frame = await sariema.capture();
    let lit = 0;
    for (let i = 0; i < frame.data.length; i += 4) if (frame.data[i] + frame.data[i + 1] + frame.data[i + 2] > 30) lit++;
    s.lit_fraction = lit / (frame.data.length / 4);
    s.stats = sariema.stats();
    s.ok = s.lit_fraction > 0.1;
    s.audio_ok = await tone(sariema);
  } catch (e) {
    s.error = String(e?.message ?? e);
  }
  show();
  return result;
}

/** The audio example's chime, mixed by the engine and played by its worklet. */
async function tone(sariema) {
  try {
    await sariema.loadSounds(new URL("../demo/sounds.srms", import.meta.url));
    await sariema.resumeAudio();
    sariema.play("chime");
    await new Promise((r) => setTimeout(r, 1200));
    const stats = await sariema.audioStats();
    result.smoke.audio = stats;
    return stats.received > 0 && stats.peak > 0.05;
  } catch (e) {
    result.smoke.audio_error = String(e?.message ?? e);
    return false;
  }
}

function pick(o, keys) {
  return Object.fromEntries(keys.map((k) => [k, o[k]]));
}

result.page = page();
result.webgl2 = webgl2();
result.wasm = wasm();
result.audio = audio();
result.webgpu = await webgpu().catch((e) => ({ available: false, why: String(e) }));
show();

document.getElementById("smoke").onclick = () => smoke();
document.getElementById("copy").onclick = () => navigator.clipboard?.writeText(JSON.stringify(result, null, 2));
if (new URLSearchParams(location.search).has("auto")) await smoke();
window.probeResult = result;
