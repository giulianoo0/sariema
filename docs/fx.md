# Effects: particles, trails and highlights

Small things that make a room feel alive and show the way: sparkles on
something to use, dust where a foot lands, motes drifting in a beam of
light, a trail of soft dots to where the hero should go, and a piece that
glows faintly in its own colour while it is within reach. A game defines
its particle effects in its settings and drives all of it with commands;
the engine keeps it on the step and the renderer draws every particle and
dot in one instanced draw, and a highlight in the scene pass it already
runs.

`examples/guide` uses all of it: motes along a line across the room, a
trail to the desk while the hero walks there, sparkles over the table and
the table glowing while the hero is within reach, a puff of dust where
each walk stops.

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
| `drag` | 0 | speed lost as `e^(−drag·t)`; keeping 0.94 of it a frame at 60 Hz is 3.7 |
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

## Trails

A trail is a row of soft dots on the floor from where someone stands to a
goal, with a wave of light running along it towards the goal.

| Command | Does |
|---|---|
| `trail { trail, from, points, spacing, size, color, intensity, wave_speed, fade_from, fade_to, fade }` | Shows trail `trail` (an id the game chooses) along `points`, from `from`'s place when it is not NONE, over `fade` seconds. Dots `size` metres across lie `spacing` metres apart, counted back from the last point, in `color` (linear RGB) times `intensity`; they fade out within `fade_from` metres of the start and `fade_to` of the goal; the wave runs to the goal at `wave_speed` m/s. A trail of that id takes the new route and look at once, its opacity eased from where it is. |
| `trail_off { trail, fade }` | Fades the trail out over `fade` seconds and frees it; one not shown is ignored. |

The dots are measured from the goal, so they stay put while the start
follows a walking hero: the ones the hero passes fade and go, the others
never move. Each dot is a flat sprite, `fx.trail.lift` above the route,
in the shape `fx.trail.sprite`. The wave is a bump of
`exp(−2·(d / wave_length)²)` around crests `wave_every` metres apart,
where `d` is a dot's distance to the nearest crest: a dot at a crest is
`1 + wave_gain` times as bright, `1 + wave_grow` times as large and fully
opaque, and one far from every crest keeps `1 − wave_alpha` of its
opacity, so the wave reads as light and as presence. It
moves with the step's time, so every host shows it in the same place.

The owner's values for a route to a goal: `spacing` 1, `size` 0.2,
amber `[1, 0.6, 0.2]`, `intensity` 0.55, a wave a few metres a second;
`fade_from` and `fade_to` around half a metre so the dots neither start
under the hero's feet nor end in a hard edge at the goal.

Refusals: `E_FX_FULL` (every one of `fx.trails` is shown),
`E_FX_POINTS` (more than `fx.trail_points` points), `E_ENTITY_STALE`
(`from` is gone). A trail whose `from` dies runs from its first point
from then on. Every trail's points are allocated when the game loads.

| Setting | Default | |
|---|---|---|
| `fx.trails` | 4 | trails shown at once, fading ones included |
| `fx.trail_points` | 64 | points a trail's route may have |
| `fx.trail_dots` | 256 | dots one trail draws at most, the ones nearest the goal |
| `fx.trail.sprite` | `disc` | the dots' shape |
| `fx.trail.lift` | 0.02 | metres above the route |
| `fx.trail.wave_length` | 1 | metres from a crest to where the wave has fallen to e⁻² of it |
| `fx.trail.wave_every` | 8 | metres between crests |
| `fx.trail.wave_gain`, `fx.trail.wave_grow` | 1, 0.25 | how much brighter and larger a dot is at a crest |
| `fx.trail.wave_alpha` | 0.4 | how much of its opacity a dot far from every crest loses |
| `fx.trail.additive` | 1 | 1 adds light to the floor, 0 covers it |

## Highlights

| Command | Does |
|---|---|
| `highlight { entity, node, fill, rim, pulse, fade }` | `entity` (an actor, or a room node's entity), or the active room's node named `node` when `entity` is NONE, glows in its own colour: added over the whole of it at `fill` and at its rim at `rim`, pulsing every `pulse` seconds (0 is steady), eased there over `fade` seconds from where it is. Both 0 fade it out and free it. |

The glow's colour is the surface's own: its material's base colour times
its texture times its vertex colour, scaled so its brightest channel is 1
(never over less than 0.08, so a near-black piece glows a faint grey).
Nothing white is added: a dark brown chair glows brown, a cream bag
glows cream, a blue box glows blue (`tests/glow.rs` in `sariema-render`
checks the hue, and that a white glow fails it). The rim is
`(1 − n·v)³`, wider than a material's rim so it reads at a distance. A
pulse dims the glow by up to `fx.pulse_depth` along a cosine, so it
breathes instead of blinking. Changing a highlight mid-fade starts from
the level it was at: no jump.

How much to use: a faint `fill` (0.1 to 0.25) and a `rim` two or three
times that read as "this can be used" on dark and light pieces alike;
more washes a dark piece towards its lightest tone under the tone curve.

Refusals: `E_SCENE_UNKNOWN` (no node of that name in the active room),
`E_ENTITY_STALE`, `E_FX_FULL` (every one of `fx.highlights` is lit).
Turning off one never lit is not an error. A highlight on an entity that
dies ends with it.

| Setting | Default | |
|---|---|---|
| `fx.highlights` | 16 | actors and nodes lit at once, fading ones included |
| `fx.pulse_depth` | 0.4 | how far a pulse dims: 0.4 is down to 60 % |

The renderer takes it per draw, in place of padding it already uploads: a
room node's draws carry their glow in the w of their normal matrix's
second and third columns (`Renderer::set_node_glow`), an actor in rows 5's
and 6's w of its record (`Renderer::set_actor_glow`); `sariema-view` sets
both from `Fx::glows(alpha)` each frame and puts back to 0 the ones no
longer lit. No new uniform, pipeline or draw.

Trails and highlights live in `fx_hash` with the particles, never in
`engine_hash`.

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
- **Measurements of hand-made effects in a top-down action game**
  (technique and numbers only): opacity that rises fast (peak at 10–25 % of the life), holds and
  fades in the last 20–30 %; dust that grows from about 0.3 to 1 with a
  drag of 0.94–0.98 a frame and a light upward pull; item sparkles that
  rise slowly; four-pointed stars among the most used sprites; glows made
  with colours above 1 left to the bloom rather than only with additive
  blending, which the `additive` share and `intensity` allow; a power-up
  shown as an animated rim in the surface's colour, and an item's rim
  falling off as a power of `1 − n·v` (5 there; 3 here, wider, for a
  piece seen from a room's distance).
- **Trails and highlights in Godot and Bevy**: neither ships one. Godot's
  usual answers are a `next_pass` material with a fresnel term or a
  second outline pass, and Bevy's are outline crates that draw the mesh
  again; both cost a draw and a material per lit object. Here the glow
  rides on the draw already made, and a trail is sprites in the particles'
  draw.
