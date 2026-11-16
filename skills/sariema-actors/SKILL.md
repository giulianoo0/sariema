---
name: sariema-actors
description: Use when bringing an animated character or prop into the Sariema engine, writing or fixing an actor description, when `sariema build --actor` fails with E_ACTOR_NAME, E_GLTF_EXTENSION or E_SCHEMA, or when an actor's motion, face, held prop, shadow or foot speed looks wrong and needs checking without video.
---

# Sariema actors

## Overview

An actor goes glTF → actor description (JSON) → `sariema build --actor` →
`.srmk` pack → `sariema run --actor`. Judge motion from filmstrips (frames
tiled into one PNG), never from a single screenshot. The full reference is
`docs/actors.md`.

## Quick reference

Every command below runs from the repository root as written; `sariema` is
`cargo run -q -p sariema-cli --`.

```sh
# Build and inspect (joints, clips with durations, loops and ground speed, parts, sockets, faces)
sariema build examples/demo --actor arm -o arm.srmk --json
sariema inspect arm.srmk --json

# A room to stand in
sariema build examples/demo --room demo -o demo.srma

# One pose: the moment --time of the clip
sariema run demo.srma --headless --frames 30 --screenshot bent.png \
  --actor arm.srmk@0.6,0,1.2,30 --clip bend --time 0.5 --json

# Motion: 6 frames over 1 s of a crossfade, tiled 3 per row
sariema run demo.srma --headless --frames 1 --screenshot strip.png \
  --width 320 --height 180 --actor arm.srmk@0.6,0,1.2,30 \
  --clip bend --then wave@0.4 --fade 0.2 --filmstrip 6 --columns 3 --span 1 --json

# A prop held at a socket of actor 0, a closer camera
sariema run demo.srma --headless --frames 1 --screenshot prop.png \
  --actor arm.srmk@0.6,0,1.2,30 --prop arm.srmk@0:tip \
  --focus 0.6,1,1.2 --distance 4 --pitch 15 --json
```

The JSON of `run` gives `actors_drawn`, `actors_culled`,
`actors_ms_per_frame` and the filmstrip's `times_s`.

## Exporting a character that imports cleanly

- **One skin**, one armature. Apply the armature's scale in the exporter, or
  keep it uniform: a non-uniform scale on a node above the root joint is
  refused.
- **No transforms on nodes between two joints.** Parent bones directly.
- **Name parts `part__material`** (`helmet__MI_helmet`): the part name is
  what game code shows or hides. Mark parts that start hidden with node
  extras `visible: false`, or list them in `hidden`.
- **Sockets**: an empty node under a bone becomes a socket of that name; or
  declare one with `{"joint", "translation", "rotation"}`.
- **Clips**: name loops so a pattern catches them (`*_lp`, `idle*`, `walk`,
  `run`) and list the patterns in `loop`; or set glTF `extras.loop`.
- **Faces**: one atlas per face, a material per eye or mouth. Give `grid`,
  `frames` in cell order from `first_cell`, and `step`. The vertical step
  may need to be negative (one actor's is `[0.25, -0.25]`, another's
  `[0.25, 0.25]`): check the offsets against the game's own
  (`origin + (c % columns, c / columns) × step`).
- **Textures**: build from the source GLB. gltfpack's `-tc` output uses
  `KHR_texture_basisu`, which is refused (`E_GLTF_EXTENSION`).
- **Feet**: list the foot joints in `feet` when their names lack `foot`, or
  ground speeds come out wrong.

## Checking motion

1. `inspect` first: are the clip durations, loops and the walk's
   `ground_speed_m_s` plausible? Game code that moves the actor at another
   speed makes the feet slide.
2. Filmstrip the transition you care about: idle → walk → idle is
   `--clip idle --then walk@0.5 --then idle@2.1 --span 2.6 --filmstrip 12`.
   A pop shows as one frame unlike its neighbours.
3. Turn the actor with the yaw (`@x,y,z,90`) to see a walk from the side,
   and bring the camera close with `--focus`, `--distance`, `--pitch`.
4. Read the strip image; compare with the game's frames at the same times.

## Common mistakes

| Symptom | Cause | Fix |
|---|---|---|
| `--face` has no effect | the clip has a face track for that face, which wins | pick a clip without one, or change `clip_faces` |
| actor seen from behind | the model faces −Z or +Z | turn it with the yaw, 180 |
| E_ACTOR_NAME | a name the model lacks; the message lists those it has | copy the name from the list |
| huge disc shadow or culling box | a clip (a fall) moves far; bounds hold every clip | expected; the disc comes from the bind pose height |
| no shadow from an actor | `Quality::actor_shadow_slots` is 0 or the room has no shadowed light reaching it | a disc is drawn instead; raise the setting |
| feet slide | the game moves the actor faster than the clip's stride | move at the clip's ground speed, or scale the clip's speed |
| prop floats beside the hand | wrong socket, or the socket's offset is in another space | `inspect` the sockets; offsets are in the joint's space |
