# Effects: particles

Small things that make a room feel alive: sparkles on something to use,
dust where a foot lands, motes drifting in a beam of light. A game defines
its particle effects in its settings and starts, stops and bursts emitters
with commands; the engine simulates every particle on the step and the
renderer draws them all in one instanced draw.

`examples/guide` uses all of it: motes along a line across the room,
sparkles over the table while the hero is within reach, a puff of dust
where each walk stops.

## Commands

| Command | Does |
|---|---|
| `fx_start { emitter, effect, entity, position, points, seed }` | Starts emitter `emitter` (an id the game chooses) of the effect named `effect`. Its particles are born at `position`; from `entity`'s placement (turned and scaled with it) when it is not NONE; anywhere along `points`, evenly by length, when there are any. An emitter of that id still running is replaced. |
| `fx_stop { emitter, fade }` | No more particles; the ones alive fade out over `fade` seconds unless their life ends first. 0 removes them at once. An emitter not running is ignored, so a game can stop what may already be gone. |
| `fx_burst { effect, entity, position, count, seed }` | `count` particles at once, placed as `fx_start` places them; the burst frees its slot when the last one dies. |

`seed` decides every particle: the same commands with the same seeds give
the same particles on every host. Take it from `ctx.rng()` so it follows
the game's own seed. A stopped emitter keeps its slot while it fades; its
id is free at once, so starting it again fades the old particles out under
the new ones.

Refusals, as `rejected` events: `E_FX_UNKNOWN` (no effect of that name),
`E_FX_FULL` (every one of `fx.emitters` is in use), `E_FX_POINTS` (more
than `fx.path_points` points), `E_ENTITY_STALE` (the entity is gone). An
emitter whose entity dies stops emitting and frees its slot when its
particles are gone.

## Effects

`game.settings.fx.effects` is a list of effects. Ranges are
`[least, most]`, each particle picking within them; keys run over a
particle's life, 0 at birth and 1 at death, linear between them.

| Field | Default | |
|---|---|---|
| `name` | | what `fx_start` and `fx_burst` name |
| `sprite` | `dot` | `dot` (a soft round spot), `star` (four thin rays around a core), `disc` (a flat middle and a soft rim), `spark` (a spot stretched along x) |
| `flat` | false | lies on the ground facing up instead of facing the camera |
| `pool` | `fx.pool` | particles alive at once from one emitter; the ones past it are not born |
| `rate` | 0 | particles per second while the emitter runs |
| `puff`, `every` | 0, 0 | particles at once when the emitter starts, then every `every` seconds; `every` 0 puffs once |
| `life` | [1, 1] | seconds |
| `size` | [0.1, 0.1] | metres across at birth |
| `grow` | none | `[t, scale]` keys multiplying the size |
| `speed`, `direction`, `spread` | [0, 0], up, 0 | metres per second at birth, along `direction` within `spread` degrees of it |
| `radius` | 0 | metres around the emitter's place a particle is born within |
| `gravity` | [0, 0, 0] | metres per second squared; positive y rises |
| `drag` | 0 | speed lost as `e^(−drag·t)`; LA's 0.94 a frame at 60 Hz is 3.7 |
| `drift`, `drift_hz` | 0, 0.5 | a sideways sway of `drift` metres per second, `drift_hz` times a second, each particle its own way |
| `spin` | [0, 0] | degrees per second the sprite turns |
| `color` | white, opaque | `[t, r, g, b, a]` keys, linear RGB |
| `intensity` | 1 | multiplies the colour; above the look's bloom threshold it blooms |
| `additive` | 1 | 1 adds light to what is behind (sparkles, motes), 0 covers it (dust, crumbs), between both |

A bad effect (no name, a name twice, a life of 0, a range most first, keys
out of order, a negative rate) fails loading with `E_SETTINGS`.

| Setting | Default | |
|---|---|---|
| `fx.emitters` | 32 | emitters alive at once, bursts and fading ones included |
| `fx.pool` | 64 | particles one emitter keeps, unless its effect says |
| `fx.path_points` | 32 | points an emitter's path may have |

Every emitter slot and its pool are allocated when the game loads: no step
allocates, whatever the game starts and stops (`tests/allocations.rs` in
`sariema-runtime` pins it).

## Simulation

Each step, after the world has moved: every particle ages one step and dies
at the end of its life; gravity accelerates it, drag slows it, the sway
pushes it sideways and it moves; then each running emitter's owed
particles (rate times the step, the remainder carried) and puffs are born
where its entity stands now, until its pool is full. A stopped emitter's
fade counts down with the steps.

`Fx::sprites(alpha, out)` gives what a frame shows between the last two
steps: each particle between its two last places, its colour, opacity and
size from the keys at its age then, the fade of a stopped emitter as of
then. Particles live in the world: one born from a walking hero stays
where it was born.

Everything that decides a particle runs on the step with `f32` and `libm`
(the runtime's rule), from the emitter's own PCG32 seeded by the command:
`fx_hash` (printed by `sariema run --headless` and by the web package's
`replayGame`) is the same on every host for the same replay. It is not part
of `engine_hash`: particles never change what the game sees.

## Drawing

The renderer takes the frame's sprites (`Renderer::set_sprites`, which
`sariema-view` calls with `Fx::sprites`) and draws them all, after the
translucent actors, in one instanced draw of four vertices per sprite:

- each sprite's place, size, colour, rotation, how additive it is, its
  shape and whether it lies flat, 48 bytes;
- the shapes are one 64 × 64 `R8Unorm` texture array with its mips, made
  when the renderer starts, so sprites of every shape share the draw;
- premultiplied blending: an additive sprite writes no opacity and only
  adds light, a covering one darkens what is behind by its opacity; the
  scene's mask in alpha is left as the room drew it;
- depth tested against the scene, not written;
- a sprite smaller than `Quality::sprite_min_px` pixels is drawn that size
  and as much fainter, so a far sparkle neither vanishes nor shimmers.

No compute and no storage buffer: the same draw runs on WebGL2.
`Quality::max_sprites` (4096) sizes the instance buffer when the renderer
starts; the frame's sprites past it are left out and counted in
`FrameStats::sprites_dropped`. `FrameStats::sprites` counts the drawn ones.

## What we took from other engines

- **Godot's `CPUParticles3D`**: a fixed array of particles per emitter
  (`amount`) reused as they die, a fixed step (`fixed_fps`), particles in
  world space by default, a colour ramp and a scale curve sampled at the
  particle's age, and every particle drawn as one `MultiMesh`. Its
  per-particle seed (`seed + 1 + i + cycle·amount`) is replaced here by the
  emitter's own generator advanced in birth order, which gives the same
  stream for the same commands without depending on slot order.
- **Godot's `GPUParticles3D`** on its GLES3 renderer simulates with
  transform feedback and feeds the wall clock and frame delta to the
  shader: not deterministic across devices, which is why the simulation
  here is on the step, on the CPU.
- **Bevy Hanabi**: an effect is an asset with a fixed capacity, spawners
  (rate, burst, repeated burst), init and update modifiers (position on a
  shape, velocity in a cone, acceleration, linear drag) and a colour
  gradient over the lifetime. It needs compute shaders, so on the web only
  WebGPU runs it; the same model here runs on the CPU.
- **PixiJS v8's `ParticleContainer`**: one texture source for every
  particle so they batch into one draw, and only the properties that change
  uploaded each frame. Here the shapes share one texture array, and the
  frame's sprites are one upload of 48 bytes each.
- **The Link's Awakening study** (`04-vfx-detalhes`, technique and numbers
  only): opacity that rises fast (peak at 10–25 % of the life), holds and
  fades in the last 20–30 %; dust that grows from about 0.3 to 1 with a
  drag of 0.94–0.98 a frame and a light upward pull; item sparkles that
  rise slowly; four-pointed stars among the most used sprites; glows made
  with colours above 1 left to the bloom rather than only with additive
  blending, which the `additive` share and `intensity` allow.
