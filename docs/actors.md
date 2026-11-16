# Actors

An actor is a character or a prop that moves: a skinned mesh, its skeleton,
its clips, its sockets and its face atlases. This page covers the whole path,
from a glTF file to an animated actor drawn in a room:

1. an actor description (JSON) names the model and what the engine should
   know about it;
2. `sariema build --actor` turns it into an actor pack (`.srmk`);
3. `sariema-anim` samples and blends its clips on a fixed step;
4. `sariema-render` draws it with the room's lights and shadows;
5. `sariema run` places actors in a room, for screenshots, filmstrips and the
   window player.

## The actor pack (`SRMK`)

`crates/sariema-format/src/actor.rs`. A pack starts with the magic `SRMK` and
a version (1), followed by a postcard body that `decode` validates:

- **meshes** of `SkinnedVertex` (position, normal, UV, colour, four joint
  indices and four weights in unorm16 that sum to 65535), split into named
  **parts** that point at a mesh and a material and can start hidden;
- **materials** and **textures**, the same types as a room pack;
- **joints** in parents-first order: name, parent, rest transform and
  inverse bind matrix;
- **clips**: name, duration, loop flag, channels (one joint property with
  step or linear keys; rotations may be stored as 16-bit integers), markers,
  face tracks, the bounds of the clip's motion and the ground speed of its
  feet;
- **sockets**: a name, a joint and an offset in that joint's space;
- **faces**: atlases whose cell moves the UVs of some materials;
- the pack's **bounds**, holding the rest pose and every clip.

The pack holds no root motion: an actor's position belongs to physics.

## Writing an actor description

The description is a JSON file checked against `schemas/actor.schema.json`
and listed in the project file under `actors`:

```json
{
  "name": "arm",
  "model": "actors/arm.glb",
  "markers": { "wave": { "tap": 0.25 } },
  "sockets": { "grip": { "joint": "elbow", "translation": [0.0, 0.1, 0.0] } },
  "materials": { "mat_arm": { "rim": 0.3 } }
}
```

| Key | Meaning | Default |
|---|---|---|
| `model` | GLB with the meshes, the skin and clips, relative to the project's `assets_root`. Without a skin it is a rigid prop with one joint, `root`. | required |
| `clips_from` | More GLBs whose clips animate joints of the same names. A clip name twice is an error. | none |
| `loop` | Clip name patterns (`*` matches any run) that loop. | clips whose glTF `extras.loop` is true |
| `feet` | Joints that touch the ground, for each clip's ground speed. | joints whose name contains `foot` |
| `hidden` | Parts that start hidden. | parts whose node extras say `visible` or `visivel_inicial` false |
| `sockets` | Name to `{joint, translation, rotation}`. | empty leaf nodes under joints |
| `faces` | Name to `{materials, grid, origin, step, frames, first_cell, start}`; cell `c` is at `origin + (c % columns, c / columns) × step` in UV. | `step` is `[1/columns, 1/rows]`, `start` the first frame |
| `clip_faces` | Clip to face to `[seconds, frame]` pairs; each frame holds until the next. | none |
| `markers` | Clip to marker name to seconds. | none |
| `materials` | Material to `{rim, drawn, additive}`. | the glTF material |
| `textures` | `astc` or `rgba8`. | `astc` |
| `keys` | How clips are stored, below. | below |

`keys`:

| Field | Default | Effect |
|---|---|---|
| `translation_tolerance_m` | 0.0001 | a key its neighbours reproduce within this is dropped |
| `rotation_tolerance_deg` | 0.01 | same, for rotations |
| `scale_tolerance` | 0.0001 | same, for scales |
| `quantize_rotations` | true | rotations in 16 bits, within 0.004° of the source |
| `cubic_resample_fps` | 30 | cubic-spline channels become linear keys at this rate |
| `bounds_fps` | 60 | poses per second sampled for each clip's bounds |
| `ground_speed_fps` | 120 | poses per second sampled for each clip's ground speed |

### What the importer does with a glTF file

- It reads the files gltfpack writes: `EXT_meshopt_compression`,
  `KHR_mesh_quantization` (normalised 16-bit rotations included),
  `KHR_texture_transform` and `KHR_texture_basisu` (below). gltfpack drops
  node names and extras unless given `-kn -ke`: without them parts have no
  names and a part marked hidden in its extras shows.
- One skin per actor. The skinned mesh node's own transform is ignored, as
  the glTF specification asks; the inverse bind matrices place the mesh.
- Joints are reordered parents first. A node between two joints that carries
  a transform is an error. Nodes above the root joint (an `Armature` with a
  scale, a gltfpack dequantisation node) are folded into the root joint when
  their scale is uniform, and refused otherwise.
- A mesh that is not skinned but sits under a joint becomes a rigid part of
  that joint. A mesh that is neither is an error.
- Parts are named by the node name before `__` (`body__MI_body` is part
  `body`), so the game can show or hide `helmet` without knowing its
  material.
- Channels on nodes that are not joints, and morph target weights, are
  skipped with a warning. Two channels on one joint property are an error.
- Each clip's bounds come from skinning the mesh on the CPU at `bounds_fps`,
  so a jump or a fall widens them; the renderer culls with them and physics
  can size a collider from them.
- The ground speed of a clip is the mean horizontal speed of the lowest foot
  while it is the lowest. Game code that moves an actor at that speed keeps
  its feet from sliding.

Errors: `E_ACTOR_NAME` (a clip, joint, material, face, frame or socket the
description names does not exist; the message lists the names that do),
`E_SCHEMA` (the description or the result is invalid), `E_GLTF_EXTENSION`
(the file requires an extension the importer cannot read),
`E_TEXTURE_ENCODE` and `E_IO`.

### Basis Universal textures

Actors and room kits may carry their textures as KTX2 images
(`image/ktx2`) named by `KHR_texture_basisu`, as gltfpack `-tc`/`-tu`,
`toktx` and `basisu` write them:

| KTX2 | Read |
|---|---|
| ETC1S in BasisLZ, with or without its alpha slice | yes |
| UASTC LDR 4×4, raw or in Zstandard | yes |
| a Vulkan format (not Basis data), UASTC HDR | no |
| cube maps, arrays, 3D textures, ETC1S video frames, global selector codebooks | no |

Only level 0 is read; the importer makes its own mips. The image is decoded
to RGBA8 and then stored as the pack's `textures` say (`astc` or `rgba8`),
exactly as a PNG would be, so a pack does not know where its texture came
from. When a texture has both a `source` (PNG or JPEG) and a Basis image,
the `source` wins: it is lossless and the Basis image is not. A KTX2 file
that is broken or of an unread kind fails the build with `E_SCHEMA`
(`unusable glTF: image N: KTX2 …`), naming what it holds.

ETC1S was made to be transcoded, not decoded: re-encoding it to ASTC adds
a second loss over its 4×4 blocks of two-colour lines. UASTC is ASTC-shaped
and survives the trip with little more. A texture meant to look its best
should come from the PNG; the decoder is for files whose PNG is gone or
that a pipeline compressed on its way in.

What was taken:

- **basis_universal** (Binomial, Apache-2.0): the decoder is a port of its
  transcoder's ETC1S/BasisLZ decoding (codebooks, Huffman tables, endpoint
  predictions, the selector history and its runs) and its UASTC unpacking
  (the 19 modes, their partitions, BISE endpoints and ASTC
  unquantisation). It was checked texel for texel against the official
  transcoder on 42 production textures (31 ETC1S up to 2048×1024, 11 UASTC
  in Zstandard up to 1024×1024): every texel equal.
- **three.js** `KTX2Loader` transcodes at load, in a worker, to the best
  format the device samples (UASTC to ASTC, then BC7; ETC1S to ETC2 or
  ETC1, then BC7, never to ASTC). Not taken: the engine's packs already carry ASTC and
  RGBA8 variants (see `docs/web.md`), so the decoding happens once, at
  build, and the runtime holds no transcoder.
- **The `basis-universal` crate** (used by Bevy) binds the C++ library and
  exposes only UASTC's low-level transcoding; ETC1S would need its own
  build. A Rust port keeps the importer free of a C++ build. Zstandard
  comes from the `zstd` crate.

## Animation (`sariema-anim`)

CPU only, no GPU types, deterministic: the same commands give bit-identical
palettes, and nothing allocates after setup (a counting allocator test steps
2000 times without one allocation).

- `Rig::new(&pack)` reads the pack once: names, parents, rest pose, clips.
  `sample` writes a clip's local pose at a time, clamped to the clip;
  rotations interpolate along the shortest arc.
- `Animator::new(&rig, AnimSettings)` holds the layers and the poses.
  `play(rig, layer, clip, Play)` fades the layer to a clip; playing the clip
  already playing changes nothing unless `restart`. `step(rig)` advances one
  fixed step; `evaluate(rig, alpha)` blends the last two steps for drawing;
  `palette()` gives the joint matrices; `socket(rig, id)` a socket in model
  space; `faces()` the atlas cell of each face; `events()` what the last step
  passed.

`AnimSettings` defaults:

| Field | Default |
|---|---|
| `step_s` | 1/60 s |
| `fade_s` | 0.13 s (the game's blend time) |
| `easing` | smoothstep |
| `layers` | 2 |
| `event_tolerance_s` | 0.00001 s |

`Play`: `mode` (`Loop`, `Once` fading the layer out at the end, `HoldLast`;
by default the pack's loop flag picks `Loop` or `HoldLast`), `speed` (at least
0), `start_s`, `fade_s`, `easing`, `restart`.

- **Crossfades** blend from the pose the layer showed when `play` was called.
  A play that interrupts a fade starts from a frozen copy of the blended pose,
  so the joints never jump; the tests bound each joint's turn per step.
- **Layers** override or add (`LayerBlend::Additive` adds the difference
  between the clip and its first frame), with a weight and an optional
  `JointMask` (a wave on the arm's subtree over a walk).
- **Events**: `Finished`, `Looped` and `Marker(i)`, each once in the step it
  happens in, with any speed and across wraps. Clip time is kept in `f64`.
- **Faces**: a clip's face track sets the cell while it plays; `set_face`
  sets it otherwise.
- **Fixed step and interpolation**: simulate with `step` at 60 Hz and draw
  with `evaluate(alpha)` at any rate; at 120 Hz every other frame lies
  between two steps.
- `ground_speed(rig, clip, feet, samples_per_s)` measures a clip's stride
  speed for game code.

## Drawing actors (`sariema-render`)

```rust
let model = renderer.load_actor(&pack)?;        // pipelines for its materials
let hero = renderer.spawn_actor(model)?;          // hidden until set
renderer.set_actor(hero, &ActorState {
    model: world_from_actor,
    palette: animator.palette(),
    faces: animator.faces(),
    visible: true,
})?;
renderer.set_actor_part_visible(hero, "helmet", true);
renderer.set_actor_disc(hero, Some(DiscShadow::for_height(1.5)))?;
```

A loaded pack is an `ActorModel`; each `ActorId` shows one, so 200 actors of
one pack share its meshes, materials and textures. A prop is an actor too:
its model matrix is the holder's model matrix times the holder's socket.

- **Skinning** happens in the vertex shader (`shaders/skin.wgsl`). Every
  actor's frame data lives in one buffer, in rows of four floats: the model
  matrix, its normal matrix, the disc shadow, the face offsets and then four
  rows per joint. On the full path it is a storage buffer and the size comes
  from the pack, so there is no joint limit beyond the device's storage
  binding size, which `load_actor` checks; on the downlevel path
  ([below](#without-storage-buffers-webgl2-opengl-es)) each actor's rows are
  bound as a uniform buffer. The buffer is written once a frame, in one
  call, and only when an actor changed. Normals use the cofactor of the skin
  matrix, which is correct under the scale tracks.
- **Materials** are the room's: the same 12 pipeline kinds (lit or unlit,
  opaque, blended or additive, single or double sided), the same fragment
  shader, so rim light and the "drawn" mask behave as on room draws. A face
  material takes its UV offset from the actor's face cell instead of its
  own.
- **Lights** are the room's clustered lights. Actors receive every shadow
  map, and the room's character light besides, which only actors receive
  ([lighting.md](lighting.md)). A material's metallic and roughness are
  its glTF factors times the mean of its `metallicRoughnessTexture` under
  its vertices.
- **Shadows**: `Quality::actor_shadow_slots` of the frame's shadow slots,
  the sun's first, also hold actors; such a slot is redrawn in the frames an
  actor within its reach moves, appears or vanishes. An actor whose bounds
  lie outside the view of every such slot's map (beyond the box the sun's
  map is fitted to, outside a spot's cone) gets a **disc shadow** on the
  plane through its origin instead.
- **Culling**: with `Quality::frustum_culling`, actors whose moved pack
  bounds lie outside the view are not drawn.
- **Order**: room opaque draws, actors' opaque parts, discs, room blended
  and additive draws, actors' translucent parts.
- **Pipelines** are created by `load_actor` for the kinds its parts use; the
  first pack also creates the shadow and disc pipelines. Drawing creates no
  pipeline and no buffer; `spawn_actor` creates a buffer only when the
  actors outgrow the one they share (it doubles).

Settings:

| Setting | low | medium | high |
|---|---|---|---|
| `Quality::actor_shadow_slots` | 0 (discs) | 1 | 2 |
| `Quality::frustum_culling` | on | on | on |

`DiscShadow::for_height(h)`, the default from the bind pose's height:
radius 0.22 × h, opacity 0.5, softness 0.6 (the outer 60 % of the radius
fades), lift 0.01 m.

### Without storage buffers (WebGL2, OpenGL ES)

WebGPU, Metal, Vulkan and DirectX 12 read storage buffers in vertex shaders.
WebGL2 has no storage buffers at all, and many OpenGL ES 3.1 drivers have
none in the vertex stage (`GL_MAX_VERTEX_SHADER_STORAGE_BLOCKS` is 0). On
such a device the renderer takes its downlevel path (`RenderPath::of`;
[web.md](web.md#backends-and-render-paths) has every WebGL2 gap): each
actor's rows are bound from the one buffer as a uniform buffer,
`Quality::actor_uniform_rows` rows at a time (1024 rows, 16 KiB, WebGL2's
minimum uniform block: about 250 joints), as Bevy and Filament bind their
joint palettes where storage buffers are missing. `load_actor` refuses a
pack whose joints and faces need more rows with `RenderError::Actor`
(`E_SCHEMA`), naming the limit. The room's lights, cluster lists and shadow
slots go into data textures on that path. The downlevel path also runs
natively, so `cargo test -p sariema-render` covers skinning through it.

### Limits known

- A mirroring model matrix shows the inside of the meshes.
- Actors' translucent parts are drawn in actor order, not sorted by distance.
- The sun's map is fitted to the room's casters; an actor outside that box
  casts nothing into it and gets a disc instead.
- Whether a map holds an actor is decided from the actor's bounds over all
  its clips. An actor whose box only grazes the edge of a map counts as held
  and gets no disc, even when its mesh casts nothing into the map.
- There is no way to remove an actor yet; hide it.
- The disc lies on the plane through the actor's origin; a jumping actor's
  disc rises with it until physics gives the ground height.

## Placing actors with the CLI

```sh
sariema build examples/demo --actor arm -o arm.srmk
sariema inspect arm.srmk --json
sariema build examples/demo --room demo -o demo.srma
sariema run demo.srma --headless --frames 30 --screenshot bent.png \
  --actor arm.srmk@0.6,0,1.2,30 --clip bend --time 0.5
```

`run` options for actors (headless and window):

| Option | Meaning |
|---|---|
| `--actor PACK@X,Y,Z,YAW` | place an actor, yaw in degrees about +Y; repeatable |
| `--clip NAME` | the clip every actor plays from time 0 |
| `--then CLIP@SECONDS` | fade to another clip at a time; repeatable |
| `--fade SECONDS` | how long each `--then` fade takes; default 0.13 |
| `--face FACE=FRAME[@SECONDS]` | set a face by frame name or cell; a clip's face track wins |
| `--prop PACK@ACTOR:SOCKET` | hold a pack at a socket of the n-th actor |
| `--time SECONDS` | the moment drawn; earlier headless frames lead up to it a step each |
| `--filmstrip N`, `--span S`, `--columns C` | N frames over S seconds tiled into one image |
| `--focus X,Y,Z`, `--distance M`, `--pitch DEG` | override the room camera |

The JSON report adds `load_actors_ms`, `actors_ms_per_frame` (animating and
posing every actor), `actors`, `filmstrip` with the frame times, and
`actors_drawn` and `actors_culled` in `frame_stats`. Without `--headless` the
window player animates the actors in real time from `--time`.

A filmstrip is how to judge motion without video: an idle, a walk and an
idle again, or a crossfade, frame by frame:

```sh
sariema run demo.srma --headless --frames 1 --screenshot strip.png \
  --width 320 --height 180 --actor arm.srmk@0.6,0,1.2,30 \
  --clip bend --then wave@0.4 --fade 0.2 --filmstrip 6 --columns 3
```

## Renderer CPU work

Profiling `s5` and `s11` (macOS `sample`) showed where a
frame's CPU went before this work: 56–66 % in wgpu's
`CommandEncoder::finish`, which replays each of the frame's 20–25 render
passes into a new Metal encoder; 6–13 % staging the per-frame
`write_buffer` calls; 16–34 % in `Queue::submit`; and a few percent in the
renderer's own code (culling, binning, sorting, recording draws). Changes:

- room draws are culled against the view frustum with their pack bounds
  (`Quality::frustum_culling`); lights already were;
- frame uniforms, lights and cluster lists are written only when their bytes
  change, and the post chain's depth uniforms only when the projection does;
- light culling, binning, shadow slot assignment and draw ordering reuse
  their buffers, and sort without allocating.

Images do not change: the golden tests pass unchanged and `s5` and
`s11` render identically (largest channel difference 0) before
and after.

## Numbers

Mac M5, 1920×1080, `sariema run --headless --frames 300` as in the reference
game's benchmark (the packs `s5.srma`, a yard, and `s11.srma`, a house of
nine rooms). The machine was
busy for every run, with a load average of 18–39 from an Android emulator
and other builds. On a quiet machine the same room measured 0.92–1.20 ms;
here it measured 1.7–3.9 ms, so absolute figures run about 2–3× high.

### Rooms, before and after

`cpu_ms_per_frame`, minimum / median of 7 interleaved runs. "Before" is
`920ace5`, the last commit without culling. "After" adds culling, the
skipped uploads and the actors.

| room, quality | before | after | draws culled |
|---|---|---|---|
| s5, medium | 2.25 / 3.87 | 2.53 / 3.36 | 30 of 32 |
| s5, high | 2.22 / 2.95 | 2.50 / 3.16 | 30 of 32 |
| s11, medium | 1.77 / 2.31 | 2.45 / 3.15 | 106 of 114 |
| s11, high | 1.78 / 2.06 | 2.13 / 2.86 | 106 of 114 |

These minimums do not show a CPU gain: every "after" minimum is 0.3 to
0.7 ms above "before", and the medians go both ways. Fifteen more
interleaved pairs on `s11` high gave a minimum of 2.06 ms before
against 2.16 ms after, and a median of 3.30 against 2.92 ms, with "after"
faster in 11 of the 15 pairs. At this load the runs cannot tell a gain from
a small loss. The absolute before and after waits for a rerun on a quiet
machine with `sariema bench` ([performance.md](performance.md)).

What the sampled profiles do show is less work in the parts this change
touched. They compare parts of the same frame, so they hold up better under
load. Each figure is a share of `Renderer::render`, from two 5 s samples per
build:

| room, quality | encoder finish | renderer's `prepare` | `write_buffer` | post |
|---|---|---|---|---|
| s5, medium | 66 → 74 % | 12.9 → 6.0 % | 12.7 → 4.2 % | 3.1 → 1.1 % |
| s5, high | 60 → 67 % | 16.4 → 12.2 % | 12.9 → 7.6 % | 2.7 → 0.9 % |
| s11, medium | 56 → 72 % | 6.3 → 2.1 % | 6.2 → 0 % | 1.9 → 1.4 % |
| s11, high | 65 → 82 % | 13.1 → 3.8 % | 11.1 → 0 % | 3.0 → 1.7 % |

On s11 no buffer is written in a settled frame. s5 still writes its light
data every frame because its lights flicker.

The table's shares do not give the frame time. If wgpu's encoder finish
cost the same in both builds, these reductions would come to about a tenth
of the frame on s5 and a fifth on s11. That is a hypothesis: a larger
finish share fits just as well with finish itself costing more, and the
minimums above lean that way.

In both builds most of the frame (56–82 %) is wgpu replaying the frame's
20–25 render passes into Metal encoders. Fewer render passes (the post
chain alone has 18–22) is therefore a larger lever than cheaper draws.
Against three.js (0.33 ms on s5, 0.14 ms on s11, when the engine measured
0.92–1.20 ms before this work), whether this work narrows the gap is for
the same quiet rerun to show.

### Actors

Actors walking (`--clip walk --time 5`) in `s5`, alternating a
companion (23 joints) and a hero (25 joints) in a grid in view. `cpu_ms_per_frame` is
minimum / median of 5 runs. Posing is `actors_ms_per_frame`, the time to
sample, blend and skin every actor's palette and hand it to the renderer
(minimum / median across the three qualities).

| actors | low | medium | high | posing |
|---|---|---|---|---|
| 0 | 1.74 / 1.84 | 2.01 / 2.36 | 2.80 / 4.83 | 0 |
| 2 | 1.70 / 2.06 | 2.31 / 2.51 | 3.23 / 3.55 | 0.010 / 0.015 |
| 20 | 2.02 / 3.08 | 2.63 / 5.51 | 4.34 / 4.52 | 0.039 / 0.066 |
| 200 | 3.36 / 3.75 | 3.78 / 4.48 | 4.60 / 5.16 | 0.415 / 0.449 |

Posing costs about 2 µs per actor. For 200 actors a sampled profile splits
the frame as follows:

- posing: 6 %;
- recording the actors' draws: 3.5 %;
- `set_actor`: 1.7 %;
- one upload of their 360 KB of frame data: 10 %;
- drawing their shadows (high only): 2.4 %;
- the rest is wgpu replaying the extra draws.

The reference three.js game draws its five or six skinned meshes inside its 0.33 ms
on s5. Here two actors add 0.01 ms of posing and a frame cost within the
noise. Twenty add 0.3–1.5 ms and two hundred add 1.6–1.8 ms (the
difference of the minimums, at this load).

### Reproducing

Interleave builds on the same pack and quality, and read the minimum and
the median:

```sh
for i in $(seq 1 7); do for bin in ./before ./after; do
  $bin run s5.srma --headless --frames 300 --screenshot x.png \
    --width 1920 --height 1080 --quality high --json
done; done
```

Actors: the same command with `--actor companion.srmk@X,0,Z,YAW` repeated (the
grid around 18.45, 0, −25.5 with 0.9 m spacing) and `--clip walk --time 5`.
Profile a 6000-frame run with `sample <pid> 5 -file out.txt` from 1.5 s
after start. Then compare the inclusive samples of `Renderer::render` with
those of its children `CommandEncoder::finish`, `Queue::submit`,
`Scene::prepare`, `Scene::draw` and `Post::draw`.

## What was taken from where

- **Godot 4**: `AnimationPlayer`'s blend time on `play` and
  `AnimationNodeTransition`'s cross-fade (one fade length per play, overridable
  per call); `BoneAttachment3D`, a node that follows a bone, for sockets; the
  Compatibility renderer's bones in a texture, for the OpenGL ES fallback.
  <https://docs.godotengine.org/en/stable/classes/class_animationplayer.html>,
  <https://docs.godotengine.org/en/stable/classes/class_boneattachment3d.html>
- **Bevy**: `SkinnedMesh`, joints plus inverse bind poses with the palette
  computed on the CPU and uploaded once a frame for all meshes; normals
  skinned with the inverse transpose of the joint matrix's 3×3 part.
  <https://docs.rs/bevy/latest/bevy/render/mesh/skinning/struct.SkinnedMesh.html>
- **ozz-animation**: the split of sampling, blending, local-to-model and
  skinning into separate steps over flat arrays; key reduction driven by a
  tolerance per property, which the importer applies with
  Ramer–Douglas–Peucker per channel. <https://guillaumeblanc.github.io/ozz-animation/>
- **ACL** (animation compression library): quantised rotations, here a fixed
  16 bits per component, chosen because 0.004° is below what a 30 fps clip
  shows. <https://github.com/nfrechette/acl>
- **Unreal Engine**: blending from a saved pose snapshot when a blend is
  interrupted, so a new play starts from what is on screen.
- **three.js**: `AnimationAction.crossFadeTo` and the game's 0.13 s blend, the
  `map.offset` face atlas switch the game uses, and its bone texture.
- **Filament** keeps a fixed bone count per renderable in a uniform buffer;
  the rule against fixed limits is why the palette here is a storage buffer
  sized from the pack.
