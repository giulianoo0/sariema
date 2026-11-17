# Lighting at run time

How a room's light reaches the actors, how a game changes the light and
the look while it plays, and how walls between the camera and the hero
step aside.

## The character light

An actor lit only by the room reads badly wherever the room is dark, lit
from the side or coloured: its back turns to the camera, a warm lamp turns
skin orange. Each room pack carries a `CharacterLight` that only actors
receive, in the actor pipelines alone, on every path WebGL2 included; it
casts no shadow and draws nothing extra. On an actor's fragment it costs a
directional term, a rim term (one `pow`) and the least brightness's test,
and for every light reaching the fragment a luminance and a mix that pull
the light's colour towards grey. Both are behind tests on the frame's
values, the same for every fragment, so a room that keeps the defaults
skips them without diverging.

On an actor's lit colour, in this order:

1. **The room's light colours** (ambient, sun, local lights) reach the actor
   mixed towards their luminance as far as `room_tint` is below 1. At 0 a
   warm lamp still lights the actor as brightly, in the albedo's own hue.
2. **The fill**: a directional light fixed to the camera, given in its frame
   (x right, y up, z towards the viewer), so it lights the side the camera
   sees in every shot. It wraps past the side it faces by `wrap`:
   `saturate((n·l + wrap) / (1 + wrap))`. No shadow, no highlight.
3. **The rim**: `rim × (1 − n·v)^rim_power`, tinted by the albedo and the
   light's colour. A thin facet seen edge-on brightens in its own colour,
   never white.
4. **The least brightness**: whatever is still below `min_brightness × the
   albedo's luminance` is made up with light in the albedo's own hue.

| `character_light` field | Default | What |
|---|---|---|
| `direction` | [0.3, 0.6, 1] | towards the fill, in the camera's frame |
| `color` | [1, 1, 1] | linear RGB of the fill and the rim |
| `intensity` | 0 | the fill, in the sun's units |
| `wrap` | 0.5 | 0 is Lambert's cosine, 1 lights all but the far side |
| `rim` | 0 | the rim at the silhouette, a fraction of the albedo |
| `rim_power` | 3 | higher hugs the silhouette tighter |
| `min_brightness` | 0 | the least an actor reflects, a fraction of its albedo's luminance |
| `room_tint` | 1 | how much of the room's light colour tints actors |

The defaults change nothing: an actor is lit as the room is. A room turns it
on in its description (`schemas/room.schema.json`):

```json
"character_light": {
  "direction": [0.3, 0.5, 1.0],
  "color": [1.0, 0.72, 0.46],
  "intensity": 1.2,
  "rim": 0.3,
  "min_brightness": 0.3,
  "room_tint": 0.5
}
```

For a dim interior lit by warm lamps those values keep a character's hair,
skin and clothes in their own colours with the back turned to the camera; a
bright exterior wants about half the intensity. While a room switch lasts
the two rooms' character lights blend like their suns.

`crates/sariema-render/tests/character.rs` holds it to its promises: in a
dim room lit warm from the side, the test actor reads at least as bright as
under a neutral light from the front; under a green light of the same
luminance as a grey one, its hue moves less than half a degree with
`room_tint` 0 and more with 0.5 and 1; its rim leaves no pixel paler than
its albedo, where the white material rim of the same strength does; and on
a ball of flat facets, each drawn with its face's normal, the rim keeps the
step from one facet to the next within half again of what it is without a
rim, where the white rim multiplies it by five.

Measured with `sariema bench --actor` ([performance.md](performance.md)) on
an Apple M5 at 1920×1080, medium quality, in three furnished rooms with one
or two actors and with an actor across a third of the frame: the draws are
the same with the light on and off, the scene pass (0.8 to 1.8 ms) differs
between them by less than between two runs of the same, and the frame's
uniform and each room pack grow by 48 bytes; no texture, buffer or
pipeline is added.

### Black hair, grey clothes, white lines

A character that came out of its exporter with a `metallicRoughnessTexture`
and no factors is metal in glTF's defaults (metallic 1, roughness 1) unless
the texture says otherwise. A material here holds one metallic and one
roughness value, so the importer scales each factor by the texture's mean
under the material's vertices (blue for metal, green for roughness); before
it did, such a character drew as rough metal: black hair, grey shirt,
saturated skin and white edges where Fresnel reached 1.

The material `rim` of an actor description is another thing: white light
added after shading, which on faceted hair, whose shells all turn edge-on
somewhere, draws a white line along every facet. Leave it at 0 and use the
character light's rim.

## Changing the light and the look while playing

Every number of a room that a game may change while it plays is a scene
parameter, named by a path. A room's index lists them with the value they
load with:

| Parameters | What |
|---|---|
| `look.exposure`, `look.bloom_threshold`, `look.bloom_strength`, `look.tint.r/g/b`, `look.real`, `look.night`, `look.tilt`, `look.focus`, `look.ao_strength` | the look |
| `sun.intensity`, `sun.color.r/g/b`, `sun.azimuth`, `sun.elevation` | the sun, when the room has one; degrees from −Z towards +X and above the horizon |
| `ambient.intensity`, `ambient.color.r/g/b` | multiply the whole ambient light, 1 at load |
| `hemisphere.intensity`, `hemisphere.sky.r/g/b`, `hemisphere.ground.r/g/b` | the ambient light straight up and straight down, read from the room's own; the intensity multiplies both and is 1 at load |
| `background.top.r/g/b`, `background.bottom.r/g/b` | the gradient drawn where nothing is |
| `fog.density`, `fog.color.r/g/b`, `fog.start`, `fog.end` | the fog, as the room describes it |
| `light.<name>.intensity`, `light.<name>.color.r/g/b`, `light.<name>.range` | each light |
| `hinge.<node>` | degrees a hinged node is turned |

A game moves them with commands, each one a tween from the value on
screen, so nothing jumps:

- `tween` with `property: scene` and `var` the parameter's name id tweens one.
- `scene_tween { tween, values, duration, ease }` tweens several together,
  `values` holding a name id and a target `f32` for each
  (`ctx.scene_tween(tween, &[(id("look.exposure"), 0.6), (id("fog.density"), 0.05)], 2.0, Ease::InOutSine)`
  in the SDK). Each replaces whatever drove its own parameter; `tween_done`
  reports `tween` once, when the last of them arrives. A parameter the
  active room lacks refuses the whole command with `E_SCENE_UNKNOWN`, and
  values torn short or not finite with `E_SCENE_VALUES`.
- A cutscene's `scene` step is the same `scene_tween`
  ([cutscenes](cutscenes.md#steps)): it ends at once, and a skip lands it,
  and every scene step still under way, under the skip's fade.

The tween runs in the step, so a parameter's value at every step is the
same on every machine, and the world hash covers only the parameters that
differ from the room's load: it changes when the game asks and only then.
A frame between two steps draws each parameter between its values at those
steps, at the frame's `alpha`, as it draws every pose, so a slow change
stays smooth at any refresh rate. A parameter that did not change between
the two steps is drawn exactly, so a room nobody touches is drawn as its
pack has it to the bit.

**The hemisphere.** A room's ambient light is nine spherical-harmonic
terms, baked from the room's hemisphere light and whatever the importer
added. The hemisphere parameters are its first term and the vertical part
of its second, read as the light straight up (`sky`) and straight down
(`ground`). Setting them rewrites those two terms and keeps the rest, so a
night sky turned on over a room keeps the sideways light it had.

**The fog.** What is drawn goes towards the fog's colour by the share
`1 − e^(−density × (d − start))` past `start`, or by the ramp from `start`
to `end`, whichever is thicker, where `d` is the distance from the camera.
At `density` 0 and `end` at or before `start` the room has none, and the
frame pays one uniform test per fragment. A glow (an additive material)
fades in fog instead of turning the fog's colour, a highlight
([fx](fx.md#highlights)) is added before the fog and fades with the surface
it lights, and the ambient occlusion fades with the surface it darkens,
judged at the pixel's view depth. The background is not fogged: a room that wants its sky hazed sets
the background to the fog's colour. A room describes its fog in its
description (`schemas/room.schema.json`):

```json
"fog": { "color": [0.55, 0.6, 0.7], "density": 0.03, "start": 4.0 }
```

The colour is the background's bottom when left out, so a game that only
tweens `fog.density` gets a haze of the sky's colour.

`examples/dusk` does all of it in a room of the importer's test kit: its
logic tweens the sun, the hemisphere, the background and the exposure
towards dusk with one `scene_tween`, and when `tween_done` comes it plays
`nightfall`, whose `scene` step brings the fog in and turns the lamps up.

```sh
sariema build examples/dusk -o dusk.srmg
sariema run --headless --game dusk.srmg --steps 600 --screenshot dusk.png --filmstrip 6
```

Measured with `sariema bench` on an Apple M5 at 1920×1080, medium quality,
in three furnished rooms with one or two actors, four runs of each case in
alternating order: the draws and the bytes sent each frame are the same
with fog and without. Fog adds 0.01 to 0.02 ms to the composite pass
(0.40 to 0.46 ms); the scene pass (0.8 to 2 ms) differs between fog and
none, and between a room without fog and the same room drawn before these
parameters existed, by less than it differs between two runs of the same
(up to 0.09 ms). The frame's uniform grows by 32 bytes and the composite's
by 16, each sent only when it changes; each room pack grows by 24; no
texture, buffer or pipeline is added, the fog being a test on the frame's
values in the pipelines that exist. A room's parameters (75 to 172 in
those rooms) are copied once a step for the frames between steps, four
bytes each.

## Walls that step aside

A room's cutaways cut its walls down where they would hide the hero. Each
one names pieces of the kit; above a height, on the camera's side of a line
that follows the hero, they are not drawn, and the inside of what is cut
shows a flat cap, lit like the room. Everything else draws as before. A
room describes them in its description (`schemas/room.schema.json`):

```json
"cutaways": [
  {
    "name": "walls",
    "pieces": ["shell_*", "front_door"],
    "height": 0.4,
    "areas": [
      { "min": [0.0, -9.0], "max": [12.0, 0.0] },
      { "min": [0.0, -19.5], "max": [12.0, -9.0] }
    ],
    "offset": -0.08,
    "follow": 0.1,
    "cap": [0.042, 0.047, 0.055]
  }
]
```

- **`pieces`**: kit names, `*` matching any run of characters. A pattern
  that no placed piece matches, or a piece that two cutaways match, stops
  the build. Merged pieces of a cutaway merge apart from the rest, a draw
  per room and material; a loose piece brings every draw of its copy, its
  states and its hinges.
- **What belongs to a wall is cut with it.** A piece placed within
  `attach` metres (0.5 by default) of an edge of its room (the room's
  `n_celulas` in the placement) belongs to that edge's wall: a door, its
  frame and leaf, a window, a picture, a lamp on the wall. Each part of it
  is judged at the point of that wall behind it, so it is cut exactly
  where the wall is, by the same plane and with the same cap, whichever
  way the line or the radius crosses the wall. None of it is left standing
  past the line, or floating over a wall that came down. The place a piece
  is put decides, not its bounds: a door's open leaf swings well away from
  its wall, and still goes with it. Such a piece draws on its own, not
  merged. A piece placed further from every edge, such as a room's shell,
  is cut where each part of it lies. Without `n_celulas` every piece is.
- **What hangs on a wall goes with it unlisted.** A piece that no cutaway
  lists, placed within `attach` of an edge of a room that holds pieces of
  a cutaway, goes in that cutaway when it lies wholly above the part kept
  whole (`whole_below`) and nothing left standing holds it up: a lamp, a
  picture, a cabinet on the wall, and what stands on a shelf of the
  cutaway. A piece holds up another when its top is within `rest` metres
  (0.02 by default) of the other's bottom, under it, so a lamp on a table
  left standing stays with the table. `sariema inspect` lists every draw
  of each cutaway by name.
- **The line** runs across the floor, square to the camera's view. What
  lies past it, towards the camera, is cut. With `areas` (rectangles of
  floor in x and z, such as each room of a house) it sits at the near edge,
  seen from the camera, of the smallest area the hero stands in. It stays
  where it was while the hero is in none, so a doorway between two rooms
  moves nothing. Without areas it sits at the hero. `offset` moves it
  towards the camera; a negative one moves it away. A shell's wall that
  stands on the edge is cut where its parts lie, so the offset must reach
  past the wall's thickness, or a sliver of the wall stays whole on the
  far side of the line.
- **`follow`** is the time, in seconds, the line takes to cover about two
  thirds of the way to where it goes next: 0.1 by default, 0 to jump. A
  camera cut moves it at once. The line moves in the step, like the hero,
  so a replay cuts the same walls. Frames between two steps draw it
  between them.
- **`height`** is the scene parameter `cutaway.<name>.height`, which a game
  tweens like any other (`scene_tween`, a cutscene's `scene` step). Raised
  past the walls, it puts them back. Set below the floor, it takes the
  pieces on the camera's side away whole.
- **`radius`** limits the cut to that many metres around the hero; 0, the
  default, does not limit it. **`soft`** fades the cut in over that many
  metres, past the line and inside the radius: the cut's height rises
  steeply towards the edge, so a wall comes down as the hero nears it
  instead of all at once.
- **`cap`** is the colour, linear RGB, of the inside of what is cut, lit
  as a lid facing the camera. The back of every face seen through the top
  of a cut shows it, on either side of the line: a corner post the line
  runs through shows one flat lid, the inside of its standing part
  included. A single-sided piece is drawn with both sides for it, and its
  back stays hidden elsewhere. The part kept apart under the cut
  (`whole_below`) draws its back faces again in the cap while the cut
  reaches it, nudged towards the camera past a floor it lies on
  (`Quality::cut_lid_bias`), so the bottom of a cut wall is capped too. Two pieces that overlap inside one
  another still show the faces of one inside the other's lid. Without a
  cap the cut is hollow.
- **What an actor carries is never cut.** A room node held with `hold`,
  or hanging from something held, such as a node under a held light,
  leaves every cutaway while it is held, a piece that went in unlisted
  included. Let go, it is cut or kept as before.
- **`cut_shadows`**, true by default, casts the shadow the pieces are drawn
  with. A shadow map drawn for one cut is redrawn when the cut moves. False
  keeps the shadow of the whole wall.
- **`style`**: `cut`, the default, or `fade`. A fading cutaway dissolves
  every piece it reaches whole, in the same stable dither as a room's
  dissolve and without sorting, over `soft`, and keeps its shadow; its
  height and cap are not used. The line's `follow` sets how long it takes
  to go and to come back as the hero walks from room to room.
- **`whole_below`**: triangles of merged pieces that lie wholly below it are
  drawn apart from the rest of the cutaway, a draw per room, wall and
  material. They stay in it, so a height the game lowers under them cuts
  them too; until it does, they take the ordinary pipelines, so floors and
  the foot of walls keep the GPU's early depth test. It is the height by
  default; below the floor, every triangle is drawn with the rest.

Without a camera rig the line is placed for the room's camera, which looks
down −Z. In `sariema run` and `sariema bench` with a room pack, the first
`--actor` is the hero, its line settled; `sariema inspect` lists a room's
cutaways and their height parameters.

### The cut of a house seen from the south

A house whose walls come down to 0.4 m between the camera and the hero,
with a dark cap: the line at the near edge of the hero's room, moved past
the thickness of its walls (0.3 m for walls up to 0.3 m thick). That is
the JSON above with one area per room, and the doors and windows listed
with the shells; the pictures and lamps hung on those walls go in by
themselves:

```json
"pieces": ["shell_*", "facade", "back_door", "window*", "door_frame", "door_leaf*"]
```

### A door frame over the hero's head

Listed with the walls, a door frame is cut at the walls' height, jambs,
lintel and leaf together, once the hero is in the room behind its wall.
The lintel hides the hero's head only from there: the camera looks down
from the hero's front, and while the hero is still in front of the
doorway the frame stands behind the hero. In a doorway between two areas
the line stays where it was, and the frame with it. Frames that should
vanish instead go in a cutaway of their own with `style` `fade`, the same
areas, an `offset` of about −0.5 m, so the line passes them fully, a
`soft` of 0.4 and a `follow` of 0.25 s.

`examples/cutaway` is a house of two rooms seen from the south, its walls
crates of the importer's test kit, the doorway between them in the wall
of the middle. The hero walks from the back room into the front one; the
line moves from one room's near edge to the other's, and the middle wall
comes back up behind the hero.

```sh
sariema build examples/cutaway -o cutaway.srmg
sariema run --headless --game cutaway.srmg --steps 300 --screenshot cutaway.png --filmstrip 6 --columns 3
```

A draw in a cutaway takes pipelines of its own, made with the first room
that has some, or when the game loads, if any room of its pack has some.
Their fragment shader can discard, which turns the GPU's early depth test
off for those draws. A draw whose bounds the cut cannot reach this frame
takes the ordinary pipelines instead, with the same pixels. Measured with
`sariema bench` on an Apple M5 at 1920×1080, medium quality, in two
furnished rooms of a house with the hero inside (two runs of each case,
the machine otherwise busy), with the shells, the façade and the pieces on
the walls in one cutaway:

- With the hero standing, the scene pass costs 0.12 to 0.18 ms more than
  without the cutaway (0.71 to 0.80 ms without), and the shadows the same.
- With the hero walking across the line and back (`--walk`), the scene
  pass costs 0.08 to 0.14 ms more. With areas the cut moves only as the
  hero crosses into another one, and redrawing the shadow maps it reaches
  then costs under 0.003 ms a frame over the walk.
- With a `radius` of 2 m, the cut moves on every step the hero takes, and
  every shadow map its draws reach is redrawn each frame: the shadow
  passes cost 0.045 ms more in one of the rooms (0.056 ms without; twice
  the shadow maps drawn), and nothing more in the other, whose cut walls
  cast into no shadow map along the walk. A line without areas, which
  follows the hero, moves the cut as often.
- There are 85 to 89 more draws a frame (98 to 100 without). The shells
  are merged apart, their part under the cut drawn apart again by wall and
  material, and each piece on a wall is drawn on its own.
- The CPU's frame time stays within what two runs of the same case vary
  (0.90 to 1.16 ms).

Keeping the floors with the rest of the cutaway (`whole_below` under the
floor) made the scene pass 0.52 ms more. A room with cutaways holds one 64-byte uniform per cutaway and
one that is off, each at the GPU's uniform offset alignment, written when
a line, height or hero moves it. The fourteen scene pipelines and the one
shadow pipeline of cutaways are made once.

## What was taken from where

- **Godot 4** (`scene_forward_lights_inc.glsl`): its rim is added per light,
  times the light's colour and attenuation, and tinted towards the albedo by
  `rim_tint`; the character light's rim is that with `rim_tint` 1 on the one
  light only actors receive. The fill's wrap follows its
  `DIFFUSE_LAMBERT_WRAP`, with `wrap` in place of the roughness and one
  normalising factor of `1 + wrap`.
- **A rim on characters alone** is common in stylised games: scenery has
  it off. Smooth characters take a white rim; the facets of stylised hair
  want it tinted.
- **Light masks** (Godot's `light_cull_mask`, Filament's light channels,
  Unity's light layers) are how those engines keep a light on characters
  alone: a real light more in the clustered loop, shading or testing every
  fragment it covers. Here the character light rides the actor pipelines,
  which the room's draws never pay for, and a room needs no light placed by
  hand that a moving camera would leave behind.
- **Fog**: Filament's `surface_fog.fs` begins its exponential fog at a
  start distance (`max(d − fogStart, 0)` metres of optical path) and Bevy's
  `FogFalloff` offers a linear ramp from `start` to `end` and an
  exponential one by `density`, both by the distance from the camera;
  here a room has both and the thicker wins, as Godot 4 takes the thicker
  of its distance and height fog (`fog_amount = max(vfog_amount,
  fog_amount)`). Godot adds the fog's colour to additive materials too;
  here a glow fades instead.
- **Tweens of the environment**: Godot animates any environment property
  with a `Tween`, and tweeners set parallel run together with one
  `finished` signal when all have; `scene_tween` is that, with
  `tween_done`. Godot and PixiJS step tweens on the display's frames
  (PixiJS by a callback on its ticker); here they step with the
  deterministic world step and the frames draw between steps.
- **The hemisphere** is three.js's `HemisphereLight` (a sky colour, a
  ground colour, an intensity), which the importer already turns into the
  ambient's terms; the parameters read it back from them.
- **Cutaways**: Godot 4's `BaseMaterial3D` fades by distance with a
  dither and a discard (its interleaved gradient noise, the same as the
  dissolve here). Bevy has no clip planes, and a cut there is an extended
  material that discards past a plane (`dot(world, n) − d`). Filament has
  no user clip planes either: a masked material discards below its
  threshold. three.js clips in world space with `clippingPlanes` and caps
  the cut with stencil passes (back faces count up, front faces count
  down, a plane drawn where the count is not 0). PixiJS masks in 2D, a
  shape into the stencil buffer or a sprite's alpha, and inverts one with
  `setMask({ inverse: true })`. The cut here is a discard past the line
  and above the height, as in Bevy. The cap is the back faces drawn in one
  colour, which needs no stencil pass, no extra pass and no extra target,
  and works the same on WebGL2.
- **Hiding what hides the hero**: Godot 4 and Bevy have no built-in
  occlusion fade. A game there casts a ray from the camera to the hero
  and changes the material of what it hits: alpha or a dithered fade in
  Godot's `BaseMaterial3D`, a material's alpha or a discard in Bevy.
  Top-down games mostly do one of two things. Some fade whatever stands on
  the ray from the camera to the hero. Others cut the walls down on the
  camera's side. Here walls and everything on them are cut, as the second
  does, because the cut follows the rooms. A ray test turns pieces on and off as the hero's head
  brushes their edges, and asks the CPU for a raycast against every piece
  each frame. The dither of `fade` is the stable screen-door pattern of
  Godot's distance fade, and needs no sorting.
- **The house cut**: top-down games cut the walls on the camera's side
  low, about 0.4 m, with a dark inside, at the near edge of the hero's
  room less a little, eased as it moves; some lower those walls in their
  geometry instead. A cutaway does either: a fixed height is a piece cut
  whatever the hero does.
