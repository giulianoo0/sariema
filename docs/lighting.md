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
fades in fog instead of turning the fog's colour, and the ambient
occlusion fades with the surface it darkens, judged at the pixel's view
depth. The background is not fogged: a room that wants its sky hazed sets
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

## What was taken from where

- **Godot 4** (`scene_forward_lights_inc.glsl`): its rim is added per light,
  times the light's colour and attenuation, and tinted towards the albedo by
  `rim_tint`; the character light's rim is that with `rim_tint` 1 on the one
  light only actors receive. The fill's wrap follows its
  `DIFFUSE_LAMBERT_WRAP`, with `wrap` in place of the roughness and one
  normalising factor of `1 + wrap`.
- **A stylised action game's character rim** (technique and numbers
  only): the rim is on characters alone, power 5.2, intensity 1; scenery
  has it off. Its characters are smooth and their rim white; the facets
  of stylised hair want it tinted.
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
