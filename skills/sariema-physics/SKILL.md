---
name: sariema-physics
description: Use when making a Sariema room walkable, placing action spots or scripted routes for a scene, when `sariema check` fails with E_SPOT_BLOCKED, E_SPOT_NO_GROUND, E_PATH_NONE or warns W_ACTOR_BOUNDS, when a character clips into a door, gets stuck, floats or cannot reach a spot, or when tuning how the character moves.
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
sariema inspect /tmp/hall.srmc                                   # colliders by shape, disabled, triggers, spots, routes
sariema check examples/collision                                 # every room; --room hall for one
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
   `spots` and `routes`.
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
its collider on or off; paths are searched again around it:

```rust
let lamp = world.collider_by_name("a.lampada").unwrap();
world.apply(Intent::SetColliderEnabled { collider: lamp, enabled: true });
```

Step a fixed number of times per frame and interpolate between
`previous_position` and `position` for drawing. Never move an actor by
writing its position: use `Teleport`, which pushes it out of overlaps or
reports `TeleportRejected`.

## Reading `check`

Each error names the room, the spot and the actor. The failure's code is the
first error's; the message lists all of them, separated by `; `. A blocked
spot also fails every route that starts there.

| Code | What it means | Fix |
|---|---|---|
| `E_SPOT_BLOCKED` `... collider is 0.600 m inside a.caixa` | The actor at the spot overlaps that collider. | Move the spot clear of the named collider. |
| `E_SPOT_BLOCKED` `... it stands clear at y 1.000` | The spot is sunk into a slope, a step or a crate top. | Set the spot's y to the height given. |
| `E_SPOT_NO_GROUND` `... stands on nothing within 0.05 m` | Floating, or past the edge of the floor. | Lower the spot or add floor (a `primitives` box). |
| `E_SPOT_NO_GROUND` `... steeper than 40°` | The ground under the spot is not walkable. | Move the spot to flatter ground. |
| `E_PATH_NONE` `no path for hero from spot A to spot B` | Nothing walkable connects them: a closed door, a gap narrower than the collider, a step over 0.20 m, or B where the actor cannot stand. | `physics-map --path A:B` and look where the green ends. |
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
defaults). The ones games change:

| Want | Setting |
|---|---|
| Climb taller steps | `controller.step_height` (0.20 m) |
| Walk steeper ramps | `controller.max_slope_deg` (40°) |
| Less sticking on corners | `controller.corner_assist_angle_deg`, `corner_assist_distance` |
| Keep more speed along walls | `controller.wall_slide_preservation` (0.85) |
| Analog feel | `controller.stick_dead_zone`, `stick_full`, `stick_exponent`, `accel_steps` |
| Turn faster | `controller.yaw_rate` (14 /s) |
| Walk speed | `ActorDesc::walk_speed` (2.1 m/s) or `Intent::SetSpeedScale` |

`check` uses the default settings; after changing `step_height` or
`max_slope_deg`, also test the routes in the game.

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
- **Expecting props to be pushed.** Actors are kinematic: they slide along
  props and push each other, nothing tumbles.
