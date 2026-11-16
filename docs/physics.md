# Physics: movement and collision

`crates/sariema-physics` moves characters through a room: a deterministic
kinematic character controller over 3D collision queries, with triggers,
moving platforms, scripted moves along searched paths, and a camera helper.
It is not a rigid-body engine: nothing tumbles or stacks. Rooms get their
colliders at packaging time (`sariema build` writes a collision pack beside
the room pack), and `sariema check` proves every action spot and scripted
route before a game ships.

The public API takes and returns `glam` types and plain ids. It depends on
`glam`, `libm` and `sariema-format` only.

## The model

### World, colliders and triggers

A `World` is built from colliders (`World::new`, or `World::from_pack` for a
collision pack, which `World::build_from_pack` builds a share at a time:
`WorldBuild::advance(units)` adds colliders, then folds, sorts and writes
the tree over their parts, and gives the world `from_pack` would). A collider is a box, sphere, capsule, convex hull or
two-sided triangle mesh, static or a platform whose pose the game sets each
step. Colliders can be enabled and disabled (a door that opens, a piece that
appears); a disabled one blocks nothing and the walkable grid around it is
rebuilt on the next search. Triggers are named volumes of the same shapes that
report actors entering and leaving.

Shapes are decomposed at build time into triangles and rounded segments
(sphere and capsule), plus the face planes of solid shapes so the inside of a
box or hull counts as inside. One bounding-volume hierarchy (median split,
ties broken by index) holds every static part.

### Actors

An actor's collider is a vertical body capsule standing on the actor's
position (its feet), plus an optional head sphere for characters whose head is
wider than their body. `ActorCollider::fit` sizes one from the model's bounds.
The head sphere is what keeps a big-headed hero's head out of a door while its
feet stay on the floor.

### A step

`World::apply` takes intents; `World::step` advances every actor by one fixed
step, in id order, and appends events to a buffer the caller owns.

| Intent | Effect |
|---|---|
| `Move { stick }` | Stick input on the ground plane; stays until replaced. |
| `MoveTo { target, face }` | Walk along a searched path, with collision, then turn to `face`. |
| `Stop` | Drop the stick and any scripted move. |
| `Knockback { push, steps }` | Push over `steps` steps, fastest first; input ignored meanwhile. |
| `Teleport { position, yaw }` | Place, then push out of overlaps; rejected if that fails. |
| `Pose { position, yaw }` | Place exactly, collider off, for a traversal: no collision, gravity or pushing until the next `Move`, `MoveTo`, `Stop`, `Knockback` or `Teleport`. |
| `SetColliderEnabled` | Enable or disable a collider. Actors inside a collider being enabled are pushed out of it at the next step, or it stays disabled. |
| `SetSpeedScale` | Multiply the walk speed. |
| `Face { yaw }` | Turn in place towards `yaw`; walking turns towards where the actor goes instead. |

`World::set_actor_active(id, false)` leaves an actor out of every step (it
leaves its triggers) without changing any other id; the engine uses it for
an actor whose entity is gone, and reuses the id for the next one.

| Event | When |
|---|---|
| `HitWall { collider, normal }` | The actor starts pressing against a wall. |
| `Landed { speed }` | The actor reaches ground after falling; an actor added or teleported onto ground is standing, not landing. |
| `TriggerEnter`, `TriggerExit` | Once per entry and exit, teleports included. |
| `Arrived` | A scripted move reached its target and facing. |
| `Blocked(NoPath)` | A scripted move has no path; the actor does not move. |
| `Blocked(Stuck)` | A scripted move stopped making progress (a door closed on it). |
| `TeleportRejected` | The push-out failed; the actor stays where it was. |
| `PushedOut { collider }` | A collider enabled where the actor stood (a door closing on it) pushed it out. |
| `EnableRefused { collider }` | The actor stood where the collider was being enabled and could not be pushed out; the collider stays disabled. |

Per actor, a step: carries it with the platform it stands on; applies a
teleport; turns the stick into a wanted velocity (dead zone, full-deflection
point, exponent, acceleration ramp, speed scale); adds knockback and gravity;
moves; snaps to ground; reports events; turns towards the facing; follows its
route. Then overlapping actors are pushed apart (sweep and prune on x, shares
weighted by mass; an infinite mass never moves), triggers are updated, and
platform poses settle.

### Moving: sweep and slide

Each move sweeps the actor's collider against the room by conservative
advancement: a sweep advances by the current gap, which can never pass
through a surface, so a 30 m/s actor does not tunnel through a 3 cm wall. A
hit stops the actor one skin short of contact, measured along the normal, and
the rest of the motion slides along the surface, up to `slide_iterations`
times.

- **Ground.** On walkable ground (slope up to `max_slope_deg`), horizontal
  speed is kept constant up and down slopes, and walls are treated as
  vertical so the actor never climbs them by sliding.
- **Walls.** Sliding along a wall keeps `wall_slide_preservation` of the
  speed when the input is within `wall_slide_full_angle_deg` of the wall,
  fading to the plain projection near head-on. Between two walls (a crease)
  the actor stops instead of jittering.
- **Corners.** Pushing nearly head-on into a corner (within
  `corner_assist_angle_deg`) probes sideways up to `corner_assist_distance`;
  if a side is free, the actor is nudged round at `corner_assist_speed`.
- **Steps.** A blocked move tries up by `step_height`, across, and down; the
  step is taken only if the new floor is walkable and no higher than
  `step_height` above the old one. A support ray decides edge contacts, so a
  rounded capsule on a step edge reads as standing on the step.
- **Snap.** An actor that was standing looks down `ground_snap` for ground and
  stays on it, so walking down stairs and slopes does not become falling.
- **Seams.** A contact normal within `flat_edge_deg` of a triangle's face is
  taken as the face normal, so sliding over the seam between two coplanar
  triangles, or a vertex, does not jitter.
- **Touching.** A shape already touching a surface is stopped only if it moves
  into it beyond `tangent_tolerance`; sliding along is not a hit.
- **Teleports.** After a teleport the actor is pushed out of overlaps in up to
  `depenetration_iterations` passes and `max_depenetration` metres in total;
  otherwise the teleport is rejected.
- **Turning.** A head sphere ahead of the body's axis swings when the actor
  turns. If the turned head would overlap the room (turning to face a wall
  the actor stands close to), the actor is pushed level out of it by a sweep,
  so the body never passes through anything. If that does not clear it within
  `max_depenetration`, the actor keeps its facing for that step.
- **Colliders enabled over an actor.** A door closing onto an actor pushes it
  out at the next step, by sweeps within the same depenetration limits, and
  the actor gets `PushedOut`. If the actor cannot be pushed out, the collider
  is disabled again and the actor gets `EnableRefused`: the door stays open,
  and the game decides what happens next.

### Paths and scripted moves

Each actor collider shape gets a walkable grid, built on first use, or ahead
by `World::prepare_paths` (for an actor) or `World::prepare_grid` (for a
collider no actor has yet). `World::start_grid`, `grow_grid` and
`finish_grid` build one a share of its columns at a time, then join a share
of its columns' floors into islands at a time, and give the grid
`prepare_grid` would (an island is named by its lowest floor, whatever the
shares); a collider switched on or off meanwhile has it built
again, whole, when it is finished. The engine builds the grid of each body of
a room's collision pack with the room's world, off the step
(`docs/rooms.md`). Like Recast's heightfield, each grid
column holds the floors where the actor stands (up to `max_layers`), found by
casting down; unlike Recast, a floor is accepted by placing the actor's own
collider there, raised by the step height so ledges it can step onto do not
block, instead of eroding voxels by a radius. A floor with anything right
above it within the step height (the ground inside a dais) is no floor.
Neighbouring floors within the step height are linked; linked floors are
grouped into islands, so a search between islands is refused in constant time.

A grid covers the static colliders' bounds, a column per `nav.cell_size`
square: the cost of a room's grid grows with how far its geometry reaches,
not with where actors walk. A room with scenery far around the walk (a
ground plane, a backdrop) declares `collision.nav_bounds`, the box in x and
z where its grids are built (`World::set_nav_bounds`); a path to a target
outside it is refused. `sariema inspect <room>.srmc` prints the columns a
grid has. A tree-house room whose ground reaches 470 m across has 22.2
million columns, 17–32 s for one body; within its 40 × 25 m of walls,
100,651 columns, 0.37 s.
`World::reachable(actor, target)` runs the same search without moving the
actor: the engine asks it when a skipped cutscene walk would have landed the
actor somewhere no path leads.

A search snaps the start and target to the nearest floors (preferring the
start's island for the target), runs A* on 8 neighbours without cutting
corners, then pulls the string: a waypoint is dropped while the straight
segment past it has floor all along and the raised collider, one skin
thinner, sweeps along it untouched. The last segment ends exactly at the
target, so a target the actor cannot stand on has no path.

`MoveTo` follows the waypoints with the same controller as the stick, so
nothing ever snaps through geometry. It emits `Arrived` within
`arrive_distance` and `face_tolerance`, or `Blocked(Stuck)` after
`stuck_steps` steps of less than `stuck_progress` of the expected travel.

### Traversals and intended contact

Walking is not the only way across a room. Climbing in through a window,
vaulting a sill or dropping from a ledge are traversals: a clip carries the
body over geometry it could never walk through. They follow five rules.

1. **A traversal is a kinematic segment, with the collider off.** Each step,
   the game samples the traversal at the clip's time and sends
   `Intent::Pose`. The clip is sought to the step clock and never runs
   free, so slow motion and frame hitches cannot desynchronise body and
   clip. A posed actor is not pushed out of the sill, gravity does not pull
   it, and other actors neither push it nor are pushed by it. Triggers
   still see it.
2. **A traversal ends on free floor.** At its last point, the actor's
   collider must be clear of the room and stand on walkable ground. The
   next `Move`, `MoveTo`, `Stop`, `Knockback` or `Teleport` turns the
   collider back on, and walking resumes with collision from there.
3. **Intended contact is authored, not penetration.** A foot hooked on the
   sill is a pose of the clip plus a declared contact point where the foot
   meets the sill. The traversal places the actor's feet so that the foot
   lands exactly on that point. The contact is a fact of the animation, not
   something the collider resolves.
4. **Any other overlap is an error.** Apart from its declared contacts, the
   actor's visual bounds must not enter static geometry, whether during the
   traversal or at rest.
5. **Hinged set pieces are moving colliders.** A window leaf or a door that
   the actor opens is a platform collider, with its hinge at the origin of
   its own frame. The game sets its `Pose` yaw each step from the same clip
   time that drives the actor. The leaf swings over the clip's frames and
   never pops open, and actors and queries see it where it is drawn.

A room declares a traversal's geometry under `collision.traversals`, so
`check` can prove it. The clip timing (which points the clip reaches at
which time, easing and speed) belongs to the timeline that plays the clip,
not to the collision pack. The fixture below is in
`examples/collision/rooms/sill.json`: an opening whose sill is 0.9 m high,
climbed from the spot `start` to free floor on the far side, with a foot
resting on the sill on the way.

```json
"traversals": [
  {
    "name": "window",
    "from": "start",
    "points": [[3.5, 0.9, -3.0], [3.5, 0.0, -4.2]],
    "yaw_deg": 180.0,
    "contacts": [{ "name": "foot", "at": [3.5, 0.9, -3.2] }]
  }
]
```

- `from`: the spot the traversal starts at. Its actor is the one carried,
  and `check` already proves it as a spot.
- `points`: the feet along the way. The last point is where the actor
  stands again, facing `yaw_deg`.
- `contacts`: the declared points where the pose touches the room on
  purpose.

`check` reports three problems:

- `E_TRAVERSAL_END` when the end is not free floor.
- `E_CONTACT_OFF_SURFACE` when a contact lies farther than
  `--contact-tolerance` from every surface, or deeper than that inside one.
- `E_CONTACT_OUT_OF_REACH` when a contact lies outside the carried actor's
  visual bounds (its collider bounds when it has none) at every point of the
  traversal, the start spot included. The traversal does not say which way
  the actor faces between points, so the bounds count turned to any facing.

The check is coarse: any point inside those turned bounds passes, including
points the pose never touches. Sampling the clip's pose is the next step
(see "Known limits").

### Queries and the camera

`raycast`, `shapecast` (a capsule), `overlap`, `closest_point` and
`actor_overlap` see the enabled colliders. `camera_clearance(target, camera,
radius, out)` lists every static collider between the target and the camera,
nearest first, and returns how far from the target a camera sphere can sit
without touching anything, so camera code can fade occluders or pull in.

### Determinism

The same intents in the same order give the same state, bit for bit, on
every platform, whatever the number of steps run per frame, and whatever
queries and path searches the game runs between them:

- one fixed step (`step_seconds`), no clock, seeded randomness only in tests;
- scalar `f32` arithmetic through `glam`'s `Vec3` and `Mat3` (no SIMD types,
  whose results can differ between targets) and `libm` for every
  transcendental function;
- no Rust float method whose precision the language leaves unspecified
  (`powi`, `powf`, `sin` and the like); squares are written as products;
- actors, colliders and contacts processed in id order; the hierarchy and
  the grid built deterministically; no hash-map iteration.

`World::state_hash` (FNV-1a over every actor's state) makes a replay
comparable; `tests/determinism.rs` pins one replay's hash, and checks that
runs batching 2 or 5 steps per frame, with path searches, walkable-cell
listings, ray casts and overlap queries between steps, give the same hash at
every step as the plain run.

A step allocates nothing once warm: buffers are kept between steps. The walkable
grid allocates when built or rebuilt after a collider changes.

## Collision packs

A collision pack (`SRMC`, `sariema_format::collision`) holds a room's
colliders, triggers, actor colliders, action spots (feet and facing, for one
actor), the routes scripts walk between spots and the traversals scripts
carry actors along. Encoding follows the room
pack: magic, version, postcard body; `decode` validates numbers, sizes,
indices and unique names and never panics.

### Baking

`sariema build` bakes a room when its description has a `collision` key
(`schemas/room.schema.json`), and writes `<room pack>.srmc` beside the room
pack. Each placed piece becomes one collider named like a loose draw
(`<room>.<piece>`, `<room>.<piece>.<n>` for the n-th copy), so the game can
enable and disable it by name; pieces that start hidden are baked disabled;
additive surfaces (light cones, glows) are left out. The shape is chosen per
piece:

1. A flat piece (thinner than `simplify_error`) keeps its triangles.
2. Otherwise its convex hull is taken, with at most `hull_points` vertices
   (the farthest vertex along each of `hull_points` directions).
3. If the hull does not hug the piece (some point of its faces is farther
   than `hull_tolerance` from the piece's surface: a door frame, a chair, an
   arch), the piece keeps its triangles, simplified within `simplify_error`.
4. If the hull fills at least `box_fill` of the piece's box, the box stands
   for it; otherwise the hull.

`exclude`, `as_mesh`, `as_hull` and `as_box` override the choice by piece name.

```json
"collision": {
  "exclude": ["tapete", "grama"],
  "as_mesh": ["raiz_musgo"],
  "primitives": [
    { "name": "floor", "box": { "center": [3.0, -0.05, -3.0], "half_extents": [3.2, 0.05, 3.2] } },
    { "name": "root", "capsule": { "a": [1.0, 0.0, -2.0], "b": [2.5, 0.8, -3.0], "radius": 0.2 }, "enabled": false }
  ],
  "triggers": [{ "name": "doorway", "box": { "center": [3.5, 1.0, -3.0], "half_extents": [0.5, 1.0, 0.5], "yaw_deg": 0 } }],
  "actors": [
    { "name": "hero", "radius": 0.3, "height": 1.0, "head": { "radius": 0.38, "height": 1.15 },
      "visual_bounds": { "min": [-0.43, 0.0, -0.35], "max": [0.43, 1.55, 0.4] } }
  ],
  "spots": [{ "name": "desk", "actor": "hero", "at": [1.5, 0.0, -0.5], "yaw_deg": 180 }],
  "routes": [["desk", "back"]],
  "bake": { "simplify_error": 0.02 }
}
```

Positions are on the engine's axes: Y up, north is −Z, metres; `yaw_deg`
turns about +Y with 0 facing +Z. An actor without `radius` and `height` is
fitted to its `visual_bounds` (with a head sphere around `head_bounds`).
`examples/collision` is a complete project.

## Command line

```sh
sariema build <project> --room <id> -o room.srma   # also writes room.srmc when the room has `collision`
sariema inspect room.srmc                           # colliders by shape, disabled ones, triggers, actors, spots, routes, traversals
sariema check <project> [--room <id>]               # every room when --room is absent
sariema physics-map room.srmc -o map.png [--path <from>:<to>] [--actor <name>] [--cut <height>]
```

Every command takes `--json`.

`check` places each spot's actor collider at the spot, and the carried
actor's collider at each traversal's end. It reports every problem as one
failure, naming the room and the spot or traversal:

| Code | Meaning |
|---|---|
| `E_SPOT_BLOCKED` | The collider is inside the room by more than `--penetration-tolerance`. When dropping it from a step's height lands it clear (a slope, a ledge), the message gives that height. |
| `E_SPOT_NO_GROUND` | No walkable ground within `--ground-depth` below the feet, or the ground is steeper than `max_slope_deg`. |
| `E_PATH_NONE` | A route's two spots have no path for the first spot's actor. |
| `E_TRAVERSAL_END` | A traversal's last point is not free floor: the collider is inside the room, or there is no walkable ground under it, under the same rules as a spot. |
| `E_CONTACT_OFF_SURFACE` | A traversal's declared contact point is farther than `--contact-tolerance` from every surface, or deeper than that inside one. |
| `E_CONTACT_OUT_OF_REACH` | A traversal's declared contact point lies outside the carried actor's visual bounds, turned to any facing, at every point of the traversal. |
| `W_ACTOR_BOUNDS` (warning) | An actor's `visual_bounds` reach past its collider by more than `--bounds-margin`. |

The failure's code is the first error's; the message lists every error.
`check` is built to take more checks: each returns findings with a stable
code and a severity, and the command aggregates them.

`physics-map` draws the pack from above, north up: colliders grey (lighter is
higher, shaded by slope), disabled colliders striped orange, green where the
actor can stand, triggers yellow, spots as blue discs with a white facing
line, the path magenta, a faint line every metre. Its JSON gives the image
size, the world `origin` (x, z) of the top-left corner and the pixels per
metre, and each spot's pixel, so a reader can map pixels to metres.
`--cut <height>` draws only what lies below a height, to see the floor under a
canopy or a ceiling.

## Settings

Every number that bounds or tunes the simulation is a field of
`sariema_physics::Settings`. Defaults come from the game this engine replaces:
its measured hero speed, collider, stick curve and turning rate, and, for what
a 2D game does not have (steps, slopes, snapping), values chosen to match how
it feels.

| Setting | Default | Meaning |
|---|---|---|
| `step_seconds` | 1/60 | Seconds one step simulates. |
| `collision.skin` | 0.01 m | Gap left between an actor and what stopped it. |
| `collision.contact_tolerance` | 1e-4 m | Closer than this counts as touching. |
| `collision.sweep_tolerance` | 1e-5 m | A sweep stops refining below this gap. |
| `collision.sweep_iterations` | 24 | Refinement steps per sweep and part. |
| `collision.tangent_tolerance` | 1e-4 | Cosine above which a touching shape moving into a surface is stopped. |
| `collision.flat_edge_deg` | 0.5° | Contact normals this close to a face normal become the face normal. |
| `collision.depenetration_iterations` | 8 | Push-out passes after a teleport. |
| `collision.max_depenetration` | 1.0 m | Total push-out before a teleport is rejected. |
| `controller.gravity` | 9.81 m/s² | |
| `controller.max_fall_speed` | 30 m/s | |
| `controller.slide_iterations` | 4 | Sweeps per move. |
| `controller.step_height` | 0.20 m | Tallest ledge walked onto. |
| `controller.ground_snap` | 0.25 m | How far down a standing actor looks for ground. |
| `controller.max_slope_deg` | 40° | Steepest walkable ground. |
| `controller.corner_assist_angle_deg` | 25° | Input this close to head-on may be helped round a corner. |
| `controller.corner_assist_distance` | 0.15 m | How far sideways the corner may be. |
| `controller.corner_assist_speed` | 0.6 | Sideways speed of the help, as a fraction of the speed. |
| `controller.corner_assist_probes` | 3 | Sideways offsets tried. |
| `controller.wall_slide_preservation` | 0.85 | Speed kept sliding along a wall at a shallow angle. |
| `controller.wall_slide_full_angle_deg` | 45° | Angle up to which the full preservation applies. |
| `controller.stick_dead_zone` | 0.15 | Deflection that moves nothing. |
| `controller.stick_full` | 0.7 | Deflection that gives full speed. |
| `controller.stick_exponent` | 1.0 | Speed curve between the two; 1 is linear. |
| `controller.accel_steps` | 0 | Steps to full speed and to stop; 0 is instant. |
| `controller.yaw_rate` | 14 /s | Facing closes `1 − e^(−rate·dt)` of the remaining angle per step. |
| `controller.yaw_filter` | 0.1 rad | Smaller changes of the wanted facing are ignored. |
| `controller.max_yaw_speed` | infinite | Radians per second an actor turns at most; the engine sets it to the runtime's `motion.turn_rate`, so no turn is fast enough to count as a jump. |
| `nav.cell_size` | 0.1 m | Walkable-grid cell side. |
| `nav.max_layers` | 4 | Floors per grid column. |
| `nav.snap_radius` | 0.5 m | How far a start or target may be from a floor. |
| `move_to.arrive_distance` | 0.02 m | Counts as arrived. |
| `move_to.waypoint_distance` | 0.1 m | Distance at which the next waypoint is taken. |
| `move_to.face_tolerance` | 0.05 rad | Counts as turned. |
| `move_to.stuck_steps` | 30 | Steps of too little progress before `Blocked(Stuck)`. |
| `move_to.stuck_progress` | 0.2 | Fraction of the expected travel below which progress is too little. |

Actor defaults (`ActorDesc`): collider radius 0.30 m, height 1.0 m, no head;
walk speed 2.1 m/s; mass 1.

Baking (`collision.bake` in the room description):

| Setting | Default | Meaning |
|---|---|---|
| `simplify_error` | 0.02 m | How far simplification may move a mesh collider's surface; thinner pieces are flat. |
| `hull_points` | 32 | Most vertices of a hull collider. |
| `box_fill` | 0.85 | Share of its box a piece's hull must fill to be baked as the box. |
| `hull_tolerance` | 0.05 m | How far a hull's faces may stand off the piece for the hull to be used. |

`check`: `--penetration-tolerance` 0.01 m, `--ground-depth` 0.05 m,
`--bounds-margin` 0.05 m, `--contact-tolerance` 0.02 m. `physics-map`: `--pixels-per-metre` 40,
`--max-side` 8192 px (larger rooms are drawn at fewer pixels per metre), no
`--cut`, a 0.5 m border.

## Performance

`cargo bench -p sariema-physics --bench step` walks actors with random stick
input through a 39 m × 24 m room (the tree house's size) with partitions,
1 m doors, 160 props (boxes, hulls, roots as capsules, low steps) and a
7,488-triangle bumpy floor mesh. Times are the thread's CPU time on an Apple
Silicon Mac under load from other builds; medians, as the range of two runs.

| Actors | Per step | Share of a 16.6 ms frame | Allocations |
|---|---|---|---|
| 1 | 4.7 µs | 0.03 % | 0 |
| 100 | 0.60–0.86 ms | 3.6–5.1 % | 0 |
| 2,000 (packed, constantly touching) | 11.3–11.8 ms | 68–71 % | 0 |

The walkable grid of that room builds in 107–134 ms (once, at load). A 20 m
path search takes 0.4–1.4 ms; a 61 m detour 4.4–6.0 ms; an unreachable target
1 µs, refused by islands.

The game this replaces (`collide.ts`: a 0.30 m circle on a 2D boolean grid,
timed with `bun` in a grid of the same size, wall-clock medians) costs 1.3 µs
per step for one hero, 30 µs for 100 circles and 0.48 ms for 2,000, with no
actor-to-actor pushing; its 20 m path search (`guidePath`) takes 6.5 ms median,
17.5 ms worst. The 3D controller costs about 4–28 times more per actor than
the 2D circle and stays below 0.1 % of a frame for a room's handful of
actors; path search is about 5–15 times faster.

A walkable grid costs about 0.8–5 µs a column on an Apple M5 (release, one
core), less where nothing stands: 100,651 columns of a tree house in 0.37 s,
526,000 of a house and its street in 2.6 s, 22.2 million mostly empty ones
in 17–32 s.

Baking is fast enough to run on every build: a tree-house room (12 mesh
colliders, 11,421 triangles after exclusions) bakes in 0.3–2.5 s, a house
room with 135 colliders in 0.2 s.

## Known limits

- Actors are kinematic. Nothing is simulated as a rigid body; props do not
  move unless the game moves them as platforms.
- An actor is a vertical capsule and an optional head sphere: it does not
  crouch, lie down or lean, and its collider does not follow animation.
- Mesh colliders are two-sided surfaces. An actor teleported into a closed
  mesh is pushed out only if it overlaps the surface; boxes and hulls are
  solid.
- The walkable grid is per collider shape, regular and 2.5D (`max_layers`
  floors per column). Cells are 10 cm by default, so a gap narrower than the
  actor by less than a cell may still pass on the grid and be refused by the
  sweep, and very large outdoor areas cost memory and time unless the room
  declares `nav_bounds`; grids are built whole, not a tile at a time on
  demand; there is no navmesh,
  no off-mesh links (jumps, ladders) and no partial path to an unreachable
  target.
- Platforms move and turn about Y only (`Pose` is a translation and a yaw);
  a platform that tilts or rolls is not supported.
- Rounding a convex corner met nearly head-on, the slowest step keeps about
  0.43 of the walk speed with the default 25° assist angle (0.20 without the
  assist): above the assist angle the plain slide takes over. A
  `corner_assist_angle_deg` of 37° raises that to about 0.58.
- Traversals are checked at their end and at their declared contacts, not
  along the way. Rule 4 during a traversal (no undeclared overlap of the
  actor's visual bounds) needs the clip's pose in each frame, and those
  poses live in actor packs, which this crate does not read. The next step
  is to sample the clip along the traversal's timing and test each frame's
  bounds against the room, allowing only the declared contacts. At rest,
  rule 4 is covered by the collider check and `W_ACTOR_BOUNDS`.
- Collision packs do not declare hinged pieces yet. `World::from_pack`
  builds static colliders only, so a hinged leaf is a platform that only
  `World::new` can create. The next step is a hinge (pivot and angle range)
  in the pack, which `from_pack` builds as a platform, plus a binding from
  clip time to hinge angle in the runtime.
- `physics-map` does not draw traversals or contact points.
- Actors push each other apart as vertical circles (radius of the body
  capsule, overlapping heights); head spheres do not push.
- `check` uses the default controller settings; a game that tunes them should
  check with its own (not yet exposed on the command line).
- The pinned replay hash was produced on aarch64 macOS and reproduced on
  x86_64-apple-darwin and on wasm32 under node, in debug and release builds.
  Other targets are untested.

## What we took from other engines

- **Rapier** (`rapier3d` 0.36 `KinematicCharacterController`, read from
  source): the offset kept between the character and obstacles, applied by
  stopping short of contact rather than inflating shapes; step-up only
  against walls and only when the new floor is walkable; ground snap only for
  a character that was grounded; on slopes, removing the vertical part of the
  motion against walls so sliding never climbs them; a bounded number of
  depenetration passes with a correction budget.
- **Godot** (`CharacterBody3D::move_and_slide`, read from source): the
  floor/wall/ceiling classification by the contact normal against the
  maximum slope, constant speed along the floor, stopping in a crease between
  two walls instead of oscillating, carrying the character with the platform
  it stands on, and the safe margin that recovers a character a little inside
  geometry.
- **Jolt** (`CharacterVirtual` documentation): the character as a virtual
  shape moved by queries rather than a body in the solver, with platforms
  carrying it, and the idea behind active edges: on a flat surface split into
  triangles, contacts at internal edges take the face normal (here,
  `flat_edge_deg`).
- **Recast** (documented parameters): a heightfield of walkable spans per
  column, a climb limit between neighbouring cells (here the step height), a
  height clearance check (here, the raised fit test and the covered-floor
  test), and regions as connected walkable areas (here, islands).
- **parry3d** (0.31, read from source) was the candidate collision library.
  Not used: it brings a second `glam` (0.33, through `glamx`) and `simba`, so
  the public API would convert at every call, and its determinism depends on
  feature flags that change `glam` for the whole workspace. The queries this
  controller needs (capsules and spheres against triangles, boxes, hulls and
  each other) are a few hundred lines on `glam` and `libm`.
- Textbook algorithms: closest points between segments and triangles from
  Ericson, *Real-Time Collision Detection* (5.1.5, 5.1.9), with the face case
  projected along the unit normal for precision on large triangles;
  ray–triangle intersection from Möller and Trumbore (1997); sweeps by
  conservative advancement (Mirtich, 1996); string pulling over the grid path
  as in most grid-based path smoothers.
- **meshopt** (already a dependency of the importer) simplifies mesh
  colliders with an absolute error bound.
