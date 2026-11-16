---
name: sariema-physics
description: Use when making a Sariema room walkable, placing action spots, scripted routes or traversals (climbing through a window, over a sill) for a scene, when `sariema check` fails with E_SPOT_BLOCKED, E_SPOT_NO_GROUND, E_PATH_NONE, E_TRAVERSAL_END, E_CONTACT_OFF_SURFACE or E_CONTACT_OUT_OF_REACH or warns W_ACTOR_BOUNDS, when a character clips into a door, gets stuck, floats or cannot reach a spot, or when tuning how the character moves.
---

# Sariema physics

## Overview

A room's collision is declared under the `collision` key of its room
description. `sariema build` bakes it into a collision pack (`.srmc`) beside
the room pack; `sariema check` proves every action spot and route before
anything runs; `sariema physics-map` draws the pack from above so you can see
what the character collides with. The game loads the pack into a
`sariema_physics::World`. Full reference: `docs/physics.md`.

Coordinates are the engine's: metres, Y up, north is −Z, `yaw_deg` 0 faces +Z
and turns about +Y. A spot is where the actor's feet are.

## Quick reference

Every command runs from the repository root; `sariema` is
`cargo run -q -p sariema-cli --`.

```sh
sariema build examples/collision --room hall -o /tmp/hall.srma   # also writes /tmp/hall.srmc
sariema inspect /tmp/hall.srmc                                   # colliders by shape, disabled, triggers, spots, routes, grid columns
sariema check examples/collision                                 # every room; --room hall for one
sariema check examples/collision --room sill                     # a traversal over a sill, with a foot contact
sariema physics-map /tmp/hall.srmc -o /tmp/hall-map.png --path start:back
sariema physics-map /tmp/hall.srmc -o /tmp/hall-cut.png --cut 0.5  # only what lies below 0.5 m
```

Add `--json` to any of them for one JSON object (`{"ok":true,...}`, or
`{"ok":false,"error":{"code","message"}}` with exit code 1).

## Workflow

1. **Declare.** Add `collision` to the room description
   (`schemas/room.schema.json`; `examples/collision/rooms/hall.json` is
   complete). Every placed piece becomes a collider named
   `<room>.<piece>` (`<room>.<piece>.<n>` for the n-th copy), where `<room>`
   is the montagem room in `show`, as loose draws are named; pieces that start
   hidden are disabled. Add `primitives` for what the pieces do not
   cover (the floor, invisible walls), `actors` (collider and model bounds),
   `spots` and `routes`. When the room's scenery reaches far around where
   actors walk (a ground plane, a backdrop), add `nav_bounds`
   (`{ "min": [x, y, z], "max": [x, y, z] }`, usually the inside of its
   boundary walls): a walkable grid costs a column per 10 cm square of the
   static colliders' bounds otherwise, and `inspect` prints how many.
2. **Exclude what must not block.** List in `exclude` the pieces a character
   walks through: rugs, grass, ropes and vines, hanging props, test scenery
   kept hidden in the room. Force a shape with `as_mesh` (arches, roots,
   anything you walk under or along), `as_hull` or `as_box`.
3. **Build and inspect.** Check the collider count and shapes, and that the
   pieces you expect disabled are listed.
4. **Check.** `sariema check <project>` until it says `no errors`. Fix spots
   in the room description, not in game code.
5. **Look.** `physics-map` with `--path <from>:<to>` for each route that
   matters; `--cut <height>` under a canopy or a roof.
6. **Load in the game.**

```rust
use glam::Vec2;
use sariema_format::collision::decode;
use sariema_physics::{EventKind, Intent, Settings, World, spot_actor};

let pack = decode(&std::fs::read("hall.srmc")?)?;
let mut world = World::from_pack(Settings::default(), &pack)?;
let hero = world.add_actor(spot_actor(&pack, pack.spot("start").unwrap()))?;
world.prepare_paths(hero); // the walkable grid, built while loading rather than on the first MoveTo
```

To build a large room's world within a frame budget instead, take it a
share at a time; each share is bounded, and the result is the world and
grid built whole:

```rust
let mut build = World::build_from_pack(Settings::default(), &pack);
let mut world = loop {
    if let Some(world) = build.advance(2048)? { break world; } // colliders, then the tree over their parts
};
let collider = sariema_physics::actor_collider(&pack.actors[0]);
if let Some(mut grid) = world.start_grid(collider) {
    while !world.grow_grid(&mut grid, 8) {} // columns, then the islands they join into
    world.finish_grid(grid);
}
```

Player input is a stick in world axes (x along +X, y along +Z), so north is
`(0, -1)`; turn gamepad input by the camera's yaw first. It stays until
replaced.

```rust
world.apply(Intent::Move { actor: hero, stick: Vec2::new(0.0, -1.0) });
```

A scripted walk to a spot, with collision all the way, ending in `Arrived` or
`Blocked(NoPath | Stuck)`:

```rust
let back = pack.spot("back").unwrap();
world.apply(Intent::MoveTo { actor: hero, target: back.position.into(), face: Some(back.yaw) });
```

Each fixed step (1/60 s):

```rust
let mut events = Vec::new();
world.step(&mut events);
for e in &events {
    if let EventKind::Blocked(reason) = e.kind {
        eprintln!("{:?} cannot reach its target: {reason:?}", e.actor);
    }
}
events.clear();
```

A piece that appears or disappears (a door closing, a crate put down) turns
its collider on or off, and paths are searched again around it. If the
collider is enabled where an actor stands, the next step pushes the actor
out (`PushedOut`). When the actor cannot be pushed out, the collider stays
disabled (`EnableRefused`): the door stays open, and the script decides
what happens next.

```rust
let lamp = world.collider_by_name("a.lampada").unwrap();
world.apply(Intent::SetColliderEnabled { collider: lamp, enabled: true });
```

Step a fixed number of times per frame and interpolate between
`previous_position` and `position` for drawing. Never move an actor by
writing its position. Use `Teleport`, which pushes it out of overlaps or
reports `TeleportRejected`, or use `Pose` for a traversal.

## Traversals and intended contact

Climbing in through a window, vaulting a sill or dropping from a ledge is a
traversal. A clip carries the body through places it could never walk.

- **Collider off.** Each step, send `Intent::Pose` with the point the clip
  has reached at that step's time. Seek the clip to the step clock; never
  let it run free. A posed actor is not pushed out of geometry, gravity
  does not pull it, and other actors do not push it. Triggers still see it.
- **Ends on free floor.** The last pose must leave the collider clear and on
  walkable ground. Then send `Stop` or `Move` to turn the collider back on.
- **Intended contact is authored.** A foot hooked on the sill is a clip pose
  plus a declared contact point on the sill. Place the actor so the foot
  lands exactly there. Any other overlap with the room is an error.
- **Hinged leaves are moving colliders.** A window leaf or door the actor
  opens swings with the clip: a platform collider whose yaw the game sets
  from the same clip time. Packs cannot declare hinges yet (see
  `docs/physics.md`, known limits).

Declare the geometry under `collision.traversals`, as in
`examples/collision/rooms/sill.json`. `from` is the spot it starts at, and
its actor is the one carried. The last of `points` is the end, facing
`yaw_deg`. `contacts` are the points touched on purpose.

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

Playing it is one `Pose` per step, then a release. The loop below poses
once per point; a game poses at every step and interpolates between points
by the clip's time.

```rust
let window = &pack.traversals[0];
for point in &window.points {
    world.apply(Intent::Pose { actor: hero, position: (*point).into(), yaw: window.yaw });
    world.step(&mut events);
}
world.apply(Intent::Stop { actor: hero });
```

## Reading `check`

Each error names the room, the spot or traversal, and the actor. The
failure's code is the first error's, and the message lists every error,
separated by `; `. A blocked spot also fails every route that starts there.

| Code | What it means | Fix |
|---|---|---|
| `E_SPOT_BLOCKED` `... collider is 0.600 m inside a.caixa` | The actor at the spot overlaps that collider. | Move the spot clear of the named collider. |
| `E_SPOT_BLOCKED` `... it stands clear at y 1.000` | The spot is sunk into a slope, a step or a crate top. | Set the spot's y to the height given. |
| `E_SPOT_NO_GROUND` `... stands on nothing within 0.05 m` | Floating, or past the edge of the floor. | Lower the spot or add floor (a `primitives` box). |
| `E_SPOT_NO_GROUND` `... steeper than 40°` | The ground under the spot is not walkable. | Move the spot to flatter ground. |
| `E_PATH_NONE` `no path for hero from spot A to spot B` | Nothing walkable connects them: a closed door, a gap narrower than the collider, a step over 0.20 m, or B where the actor cannot stand. | `physics-map --path A:B` and look where the green ends. |
| `E_TRAVERSAL_END` `traversal window: at the end, hero's collider is 0.500 m inside sill` | The traversal ends inside geometry; the collider would come back on inside it. | Move the last point to free floor beyond the sill. |
| `E_TRAVERSAL_END` `... at the end, hero stands on nothing within 0.05 m` | The traversal ends in the air. | Put the last point's y on the floor. |
| `E_CONTACT_OFF_SURFACE` `traversal window: contact foot is not within 0.02 m of a surface` | The declared contact floats: pose and set do not meet. | Put the contact on the sill, and the traversal's point where the pose's foot lands on it. `--contact-tolerance` sets the distance. |
| `E_CONTACT_OFF_SURFACE` `... contact foot is 0.400 m inside sill` | The contact is buried in the set. | Move it to the surface. |
| `E_CONTACT_OUT_OF_REACH` `... contact foot lies outside hero's visual bounds at every point of the traversal` | No point of the traversal puts the actor where it could touch that contact. | Add the point where the pose touches it, such as the feet on the sill, or fix the contact. |
| `W_ACTOR_BOUNDS` `... reaches 0.16 m past its collider at the left` | The model is wider than its collider, so it visibly enters walls. | Widen the collider or add a `head` sphere; `--bounds-margin` sets the threshold. |
| `E_SCHEMA` `exclude lists sofa, which no room in show places` | A piece name that is not placed, a piece in two shape lists, a spot naming an unknown actor, a bad bake setting. | Fix the name or the value the message gives. |

## Reading `physics-map`

North is up. Grey is collision, lighter is higher. Green is where the actor
can stand (on top of walls too: isolated green the actor cannot reach is
harmless). Orange stripes are disabled colliders, yellow are triggers, blue
discs are spots with a white line for facing, magenta is the path. One faint
line per metre. The JSON gives `origin` (world x, z of the top-left pixel)
and `pixels_per_metre`: world x = origin x + px / ppm, world z = origin z +
py / ppm.

## Tuning the controller

Every number is a field of `Settings` (`docs/physics.md` lists all, with
defaults). A game sets them in its project's `game.settings`, under
`physics`, and its pack carries them to the collision module; a key left
out keeps its default, an unknown one fails the build with `E_SCHEMA`, and
a size that cannot run (`nav.cell_size` 0) with `E_SETTINGS`:

```json
"game": { "settings": { "physics": { "controller": { "step_height": 0.26, "accel_steps": 6 } } } }
```

`step_seconds` and `controller.max_yaw_speed` are the engine's (the runtime's
step and `motion.turn_rate`), so a game's settings cannot name them.
`sariema api --json` lists every `physics.*` key with its default. The ones
games change:

| Want | Setting |
|---|---|
| Climb taller steps | `controller.step_height` (0.20 m) |
| Walk steeper ramps | `controller.max_slope_deg` (40°) |
| Less sticking on corners | `controller.corner_assist_angle_deg`, `corner_assist_distance` |
| Keep more speed along walls | `controller.wall_slide_preservation` (0.85) |
| Analog feel | `controller.stick_dead_zone`, `stick_full`, `stick_exponent`, `accel_steps` |
| Turn faster | `controller.yaw_rate` (14 /s) |
| Walk speed | `ActorDesc::walk_speed` (2.1 m/s) or `Intent::SetSpeedScale` |

`check` walks the routes and judges the ground with the game's settings,
so a project whose `game.settings.physics.controller.step_height` is 0.26
passes routes up 0.25 m risers that fail at the default 0.2.

## Common mistakes

- **Spot copied from another engine's coordinates.** Convert to Y up, north
  −Z first; `check` then reports the spot as blocked or with no ground.
- **Spot on a step or ramp at y 0.** Use the height `check` gives.
- **A `MoveTo` target inside geometry.** The path ends exactly at the target,
  so a target the actor cannot stand on gives `Blocked(NoPath)` and the actor
  does not move. Put scripted targets on spots that pass `check`.
- **Big head near a door.** The head sphere stops at the door leaf even when
  the feet could go further. Place the spot where the head clears it.
- **Hidden test scenery, ropes, hanging props baked as walls.** Add their
  piece names to `exclude`; `physics-map` shows them as grey (or striped,
  when hidden) where nothing should block.
- **A hidden actor blocking the way.** Hiding an actor leaves its body:
  other actors bump into nothing visible and a door stops at it. Switch its
  body off with `set_collider` on the actor (`Intent::SetSolid` in the
  module) while it is hidden.
- **Expecting props to be pushed.** Actors are kinematic: they slide along
  props and push each other, nothing tumbles.
- **`Teleport` or `MoveTo` for a climb.** `Teleport` pushes the body out of
  the sill (or is rejected), and `MoveTo` finds no path over it. Declare a
  traversal and play it with `Pose`.
- **Forgetting the release.** A posed actor stays where it was posed, with
  no gravity and no collision, until `Stop`, `Move`, `MoveTo`, `Knockback`
  or `Teleport`.
- **A clip left running free during a traversal.** In slow motion or on a
  slow frame, the body and the clip drift apart, and the foot misses the
  sill. Drive the clip time from the step.
