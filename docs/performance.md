# Performance

How to measure what a frame costs, where the cost goes, what the renderer
does to keep it low, and what the numbers were on the reference machine.

## Measuring: `sariema bench`

```sh
sariema bench room.srma --frames 300 --warmup 60 --runs 3 \
  --width 1920 --height 1080 --quality medium --json
```

The command loads the pack offscreen, draws `--warmup` frames, then
`--runs` runs of `--frames` frames as fast as the CPU can submit them, then
one more run with every render pass timed on the GPU. Frame `n` is drawn at
`n / 60` seconds, so flickering lights move as they do in `run --headless`.

| Flag | Default | Meaning |
|---|---|---|
| `--frames N` | 300 | frames timed per run, at least 1 |
| `--warmup N` | 60 | untimed frames before the first run |
| `--runs N` | 3 | runs of `--frames` frames, at least 1 |
| `--width`, `--height` | 1920, 1080 | target size in pixels |
| `--quality` | `medium` | `low`, `medium` or `high` |
| `--msaa N` | the tier's | samples per pixel, clamped to the device |
| `--no-shadows` | off | neither the sun nor any light casts a shadow |
| `--no-ao` | off | no ambient occlusion |
| `--actor PACK@X,Y,Z,YAW` | none | an actor pack placed in a room pack; repeatable |
| `--clip NAME` | rest pose | the clip every placed actor plays from time 0 |

Placed actors are posed at each frame's time as `run --headless` poses
them, so `frame_stats.actors_drawn` and the scene pass count their draws;
an actor placed a few metres in front of the room's camera fills the
frame, the worst case for anything only actors pay.

`--msaa 1 --no-shadows --no-ao` matches the Origem three.js game's
settings: no anti-aliasing, no shadows, no occlusion. It does not match its
content: the game also draws its characters and runs its logic every frame,
and its post-processing is shorter (one bloom level, 6–7 passes against the
engine's 9–13).

The JSON report holds:

- `load_ms`: reading the file, decoding the pack, creating the device,
  creating the renderer (every pipeline), uploading the room and waiting for
  the upload, and the first frame's CPU and wall time.
- `cpu_ms`: per frame, the median, 90th and 99th percentiles, minimum and
  maximum of the whole `Renderer::render` call (`frame`) and of its stages:
  `cull` (the actors' upload, light culling, shadow slots, and the
  culling of room draws and actors), `bin` (light clustering and
  buffer writes), `record` (the shadow and scene passes), `post` (the
  post-processing passes), `finish` (`CommandEncoder::finish`) and `submit`
  (`Queue::submit`). wgpu records a pass's commands as they come and turns
  them into the backend's commands in `finish`, so most of a pass's CPU cost
  shows there, not in `record` or `post`.
- `wall_ms_per_frame`: per run, the time from its first frame to the GPU
  finishing its last, divided by the frames; the median of the runs.
- `per_frame`: render passes, post passes, draw calls of every pass, bytes
  written to GPU buffers, and allocations and bytes allocated during
  `render`. The allocations are counted only by a CLI built with
  `cargo build --release -p sariema-cli --features alloc-count`, whose
  global allocator wraps the system's for the whole binary; any other build
  reports them as `null`. They are the allocations made through Rust's
  allocator, on every thread, a `realloc` counting as one allocation of its
  new size; memory that Metal or another system framework allocates itself
  is not in them.
- `gpu`: `"timestamps": "unsupported"` where the device lacks
  `TIMESTAMP_QUERY`. Otherwise `frame_ms`, from the start of a frame's first
  pass to the end of its last, and per pass the median and 90th percentile
  of how far the pass moves the end of the frame. A tile-based GPU starts a
  pass's vertex stage as soon as it can and runs passes side by side, so a
  pass's own start-to-end span overstates it; `last_frame` lists the raw
  spans of one frame.
- `shadow_slots_drawn`: shadow map slots drawn over the whole bench; a
  static room draws them in its first frame only.
- `load_average`: the system's 1, 5 and 15 minute load averages, since the
  numbers mean little without them.

### A game pack's frame

```sh
sariema bench game.srmg --input examples/climb/input/play.json --json
```

Given a game pack, each frame bench draws is one engine step, with the
input log's actions when `--input` names one, and the frame at its end: the
room, the actors and the HUD ([engine.md](engine.md)). `cpu_ms.frame` then
covers all of it, `cpu_ms.step` the step alone, `cpu_ms.feed` the audio
feed after it (decoding the music, mixing the step) and `cpu_ms.hud` the
HUD's layout, draw list and pass. `per_frame` counts the HUD pass with the
rest, and `game.hud_pass` gives its passes, draws and bytes uploaded apart.
The steps run as `run --headless` runs them, so `per_frame.allocations`
includes the feed's decoder; a steady step allocates nothing on every host
(`crates/sariema-engine/tests/steady.rs`). The cost of a step with
a crowd walking under the collision module is
`cargo run --release -p sariema-engine --example crowd -- 2000`.

The renderer exposes the same data to code: `Renderer::last_frame_stats`
(passes, draws, bytes uploaded), `Renderer::last_frame_timings` (the CPU
stages, once `Renderer::time_cpu_stages` has given the renderer a clock;
without one a frame reads no clock) and `Renderer::time_gpu_passes` with `Renderer::gpu_pass_times`.

Timings on a shared machine move by tens of percent from run to run. Pass,
draw, byte and allocation counts do not: compare those first, and compare
timings only between builds run interleaved in the same session.

## Where a frame's CPU goes

On wgpu 30 with Metal, the CPU cost of a frame follows the number of render
passes far more than the number of draws. wgpu-core keeps a pass's commands
in its own list while the pass is open and, in `CommandEncoder::finish`,
turns each pass into a Metal render command encoder in a Metal command
buffer of its own. `finish` is two thirds to four fifths of
`Renderer::render`; in a `sample` profile of a 19-pass frame, creating the
per-pass Metal encoders was about two fifths of `finish` and creating the
per-pass command buffers about a quarter. Each pass cost 0.02–0.05 ms of
CPU on an Apple M5; 24 or 127 draws changed the total by under 0.2 ms.

The allocations left in a frame are wgpu's, about 16 per render pass: pass
and barrier bookkeeping in wgpu-core and the Metal encoders in wgpu-hal.
Traced with a backtrace per allocation on two reference rooms, the
renderer itself made none.

So the renderer's main lever on CPU is drawing fewer passes, and its lever
on a tile-based GPU is the same: every pass stores its target to memory and
the next pass loads it back.

## Fewer passes

- An effect the look turns off draws none of its passes, and the shadow
  atlas is drawn only in frames where a slot went stale.
- **One-pass blurs for small levels.** Each post-processing blur is a
  Gaussian of `Quality::blur_taps` bilinear samples per direction (each
  sample past the centre weighs two texels). Bilinear filtering is
  separable, so one pass of `blur_taps`² samples, each at a horizontal and a
  vertical tap, gives what the two one-axis passes give, up to the rounding
  of the intermediate RGBA16F target (at most 1/255 in the final 8-bit
  image on the reference rooms). The one-pass form samples `blur_taps`
  times more, so it only pays on small levels: a level of at most
  `Quality::blur_one_pass_max_texels` texels is blurred in one pass.
- **Reduce and blur in one pass.** Each bloom level after the first is the
  level above reduced to half its size, then blurred. A level small enough
  that four times its texels fit under `blur_one_pass_max_texels` is reduced
  and blurred in one pass whose every sample takes the four texels of the
  level above that the reduction would have averaged.
- The bloom's first level stays two passes: its horizontal pass keeps only
  the light past the threshold, which is not linear, so it cannot be folded
  into a two-dimensional sample without changing the image.

`blur_one_pass_max_texels` is the same in every tier, `1 << 17` (131 072).
At 1920×1080 that puts the 1/4 level (129 600 texels) and every bloom level
after the first in one pass, and keeps the 1/2 level (518 400) in two: on
the M5 its one-pass form took 0.26 ms of GPU against 0.06 ms for two passes.
At the 1266×585 phone buffer the counts are the same. A device whose passes
cost more than its samples can take a larger value; measure it with
`sariema bench` there first.

Passes per frame, shadows already drawn:

| Look | Before | After |
|---|---|---|
| Full look, bloom, miniature blur and occlusion (s11) | 23 | 16 |
| Full look, bloom and occlusion (s5) | 19 | 13 |
| Like for like, bloom and miniature blur (one s11 room) | 20 | 13 |
| Like for like, bloom only (s5) | 15 | 9 |

## Reference numbers

Apple M5, Metal, `medium`, the Origem rooms, with frustum culling and
actors in the renderer; medians of three repetitions of three runs of 300
frames, with the load average between 6 and 16 (the machine was shared),
so timings carry about ±10%. Wall is the time from a run's first frame to
the GPU finishing its last, per frame. "Like for like" is `--msaa 1
--no-shadows --no-ao` on one room. The three.js column is the Origem game
in Chrome with ANGLE on Metal, measured in the same session: its frame's
render call alone, called back to back, with the game's tick not run and
its characters hidden, up to one `readPixels`, per frame.

| Buffer | Look | Room | Passes | CPU ms | GPU ms | Wall ms | three.js wall ms |
|---|---|---|---|---|---|---|---|
| 1920×1080 | like for like | s5 | 9 | 0.45 | 0.95 | 0.99 | 0.81 |
| 1920×1080 | like for like | s11 | 13 | 0.65 | 0.97 | 1.13 | 1.61 |
| 1920×1080 | like for like | s12 | 13 | 0.56 | 0.86 | 0.92 | 0.99 |
| 1920×1080 | full | s11, nine rooms | 16 | 0.91 | 1.53 | 1.75 | |
| 1266×585 | like for like | s5 | 9 | 0.38 | 0.53 | 0.51 | 0.32 |
| 1266×585 | like for like | s11 | 13 | 0.51 | 0.59 | 0.76 | 0.61 |
| 1266×585 | like for like | s12 | 13 | 0.49 | 0.58 | 0.67 | 0.38 |
| 1266×585 | full | s11, nine rooms | 16 | 0.63 | 0.63 | 0.85 | |

Like for like still differs in content: the game's post-processing is 6–7
passes (one bloom level, one half-size blur) against the engine's 9–13, and
each side draws its own build of the room. Before the pass reductions the
engine rows drew 15, 20 and 23 passes and took 0.58–1.29 ms of CPU. Of the full look's extra cost over like for like, the
occlusion (three passes and its GPU work) is the largest share, then MSAA;
the shadow atlas, drawn once for a static room, and the eight extra rooms
cost at most 0.2 ms of CPU or GPU each.

### The whole engine

Apple M5, Metal, `medium`, release builds, medians of three repetitions,
load average 7.7–14.5 (shared machine; the one-s11 row 6.6–9.2).
Offscreen frames, since the window needs an unlocked screen; the window
draws through the same `GameView`.

| What | Buffer | CPU ms | GPU ms | Wall ms |
|---|---|---|---|---|
| `examples/climb`, one step and its frame (`bench`) | 1920×1080 | 0.88 (step 0.006, audio feed 0.045, HUD 0.065) | 1.21 | 1.68 |
| `examples/climb`, one step and its frame (`bench`) | 1280×720 | 0.69 | 0.66 | 0.98 |
| one-s11 with two actors looping a clip (`run --headless`, 300 frames) | 1920×1080 | 1.30 | | 2.10 |

The climb frame is 15 passes and 21 draws; its HUD pass is one pass, one
draw and 7 KB of quads. A step alone over the climb's 900-step replay (wasm
VM, collision, animation, HUD): median 0.0006 ms, mean 0.005 ms, the
cutscene's steps the dearest; its audio feed, which decodes the music and
mixes the step, a mean 0.035 ms a step. 100 actors walking under collision,
each looping a clip, 0.042 ms a step; 2,000 of them, 1.36 ms
(`examples/crowd`).

## Phones and the web

- The blur no longer reads its kernel from a storage buffer: the weights
  are constants in the shader. WebGL2 has no storage buffers
  (`Limits::downlevel_webgl2_defaults` allows none), so the post chain's
  blurs need nothing WebGL2 lacks. The scene pass's lights, clusters and
  shadow slots are still storage buffers.
- WebGL2 and some phones have no `TIMESTAMP_QUERY`; `bench` then reports
  `"timestamps": "unsupported"` and every count and CPU stage still holds.
- Fewer passes is the change that helps tile-based GPUs most: each pass
  removed is one store and one load of a whole target. Nothing here was
  measured on a phone.

## What was taken from other engines

- **Bevy** (`bevy_core_pipeline::bloom`): one pass per mip level for the
  bloom, with the threshold applied in the first downsample rather than in
  a pass of its own. Sariema keeps its Gaussian levels (part of the look)
  but takes the one-pass-per-level shape for the small levels.
- **Filament** (`PostProcessManager`): every effect that reads the full-size
  image (bloom mix, vignette, grading, dithering) is folded into one final
  pass, which Sariema's composite already did; and the downsample chain
  works in one pass per level.
- **Linear-sampled Gaussian** (Daniel Rákos, "Efficient Gaussian blur with
  linear sampling"): each bilinear sample between two texels weighs both,
  which the blur already did and which the one-pass form extends to two
  axes.
- Not taken: Bevy's and Filament's 13-tap downsample and tent upsample
  chain in place of blurred levels. It would draw fewer and cheaper passes
  but changes the bloom's shape, which is a decision about the look.
