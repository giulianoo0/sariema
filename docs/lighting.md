# Lighting at run time

How a room's light reaches the actors, how a game changes the light and
the look while it plays, and how walls between the camera and the hero
step aside.

## The character light

An actor lit only by the room reads badly wherever the room is dark, lit
from the side or coloured: its back turns to the camera, a warm lamp turns
skin orange. Each room pack carries a `CharacterLight` that only actors
receive. It costs one directional term, one rim term and one comparison in
the actor's fragment shader, on every path WebGL2 included; it casts no
shadow and draws nothing extra.

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
`room_tint` 0 and more with 0.5 and 1; and its rim leaves no pixel paler
than its albedo, where the white material rim of the same strength does.

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

## What was taken from where

- **Godot 4** (`scene_forward_lights_inc.glsl`): its rim is added per light,
  times the light's colour and attenuation, and tinted towards the albedo by
  `rim_tint`; the character light's rim is that with `rim_tint` 1 on the one
  light only actors receive. The fill's wrap follows its
  `DIFFUSE_LAMBERT_WRAP`, with `wrap` in place of the roughness and one
  normalising factor of `1 + wrap`.
- **Link's Awakening** (study `04-vfx-detalhes`, measured only): the rim
  (`fresnel_effect`) is on characters alone, power 5.2, intensity 1; scenery
  has it off. Its characters are smooth and their rim white; the facets of
  stylised hair want it tinted.
- **Light masks** (Godot's `light_cull_mask`, Filament's light channels,
  Unity's light layers) are how those engines keep a light on characters
  alone: a real light more in the clustered loop, shading or testing every
  fragment it covers. Here the character light rides the actor pipelines,
  which the room's draws never pay for, and a room needs no light placed by
  hand that a moving camera would leave behind.
