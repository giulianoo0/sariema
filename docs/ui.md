# UI

`crates/sariema-ui` is the engine's in-game UI: a retained tree of boxes,
text, images, rings and strokes laid out with flexbox, animated on the
fixed simulation step, driven by multi-touch, and turned into one list of
quads that `sariema_render::ui` draws in as few draw calls as textures
allow. A screen is a JSON description (`schemas/ui.schema.json`) or the
same operations through the API, and `sariema ui render` draws it headless,
so an agent can see a HUD without a device.

## The model

```
input (pointers, focus) ──▶ Ui::step()  ──▶ events
                            animations, widgets, layout when needed
render frame            ──▶ Ui::draw(alpha) ──▶ DrawList { quads, batches }
                                                     │
                            sariema_render::ui::UiPass ◀┘ one draw per batch
```

- **`step`** runs once per simulation step (`step_s`, 1/60 by default):
  it hands back the events since the last step, advances every animation by
  exactly one step, fires widget holds, and lays out again only if something
  that affects layout changed (a node created or removed, a text, a size, the
  screen).
- **`draw(alpha)`** builds the draw list between the last two steps:
  positions, scales, rotations, opacities, colours, sizes, ring progress and
  typewriter reveal are all interpolated, so motion is smooth at any frame
  rate while the simulation stays on its fixed step.

Neither allocates on a screen that does not lay out again: a test arms a
counting allocator around 120 frames of a static screen and 90 frames of 50
nodes moving and fading plus a typewriter, and counts zero.

**Units and scaling.** Lengths are UI units of the `reference` size (844 ×
390 by default: a phone held sideways). The reference, turned to the
screen's orientation, is scaled to fit and the longer side gets the extra
room, as Godot's `expand` stretch aspect: a 16:9 screen is 844 units wide
and taller, a 21:9 screen is 390 units tall and wider, a portrait screen is
390 units wide. The draw list and pointer positions are physical pixels.

**Safe area.** `Ui::set_screen(width, height, insets)` takes the system's
insets in pixels. A node with `"safe": true` covers the safe area; the HUD
goes inside it, full-screen fades and letterboxes outside it.

## Nodes

Every node has a layout style, a look and a transform.

| Kind | Made by | Draws |
|---|---|---|
| box | default | a rounded rectangle: `fill` or `gradient`, `border`, per-corner `radius` (elliptical corners as CSS's `x / y`), `shadow` with offset and Gaussian blur |
| text | `text` | shaped glyphs from the distance-field atlas, with `outline` and `text_shadow` |
| image | `image` | a `region` of an image, tinted by `fill`, or a nine-slice with `slice` insets |
| ring | `ring` | a full circle `track` and an arc of `progress`, round caps, from `start` degrees |
| stroke | `stroke` | a polyline with round caps drawn up to `progress` (check marks, strike-throughs, icons) |

Transforms (`x`, `y`, `scale`, `rotation` around `origin`) apply after
layout, like CSS transforms; `opacity` multiplies down the tree; `clip`
clips the children to the node's axis-aligned screen box. Fades and
letterboxes are plain boxes.

**Gradients** replace a box's `fill` with CSS's two-stop gradients:

```json
"gradient": { "type": "linear", "angle": 160, "from": "#FFF8EC", "to": "#FFE2A8" }
"gradient": { "type": "radial", "center": [0.5, 0.32], "from": "#FF8A3D", "to": "#E0650F", "stops": [0, 0.7] }
"gradient": { "type": "radial", "center": [0.5, 0.46], "radius": [0.7, 0.62],
              "from": "#080C1E00", "to": "#080C1E8C", "stops": [0.55, 1] }
```

`angle` is degrees clockwise from "to the top" (default 180, downwards) and
the line spans the box corner to corner, as in CSS. A radial gradient has a
`center` as a fraction of the box and either `radius` as fractions of its
width and height (CSS's `ellipse 70% 62%`) or none, a circle reaching the
farthest corner (CSS's `circle`). `stops` (default `[0, 1]`) are where
`from` ends and `to` starts. Colours blend premultiplied, in sRGB, as a
browser blends them, so a fade from a transparent colour does not darken.
`opacity` fades both colours; the `fill` animation does not drive a
gradient. `Ui::set_gradient` changes or removes one at run time. A
gradient box is a shape like any other: it joins its neighbours' batch.

**Layout** is CSS flexbox through taffy 0.15 (what Bevy's UI uses):
`direction`, `wrap`, `justify`, `align`, `align_self`, `grow`, `shrink`,
`basis`, `gap`, `padding`, `margin`, min and max sizes, `aspect`, and
`absolute` placement with `left`, `top`, `right`, `bottom`. Lengths are
units, `"N%"` or `"auto"`. Two choices differ from a browser's defaults:
every node is a containing block for its absolute children (as in Bevy and
React Native), and the root is a row filling the screen.

**Groups** (`"groups": ["cutscene"]`) name sets of nodes for the host, as
Godot's node groups do; the screen does not read them. The engine keeps a
node of the root in `cutscene` on screen while a cutscene plays.

## Text

- **Shaping** is rustybuzz (a port of HarfBuzz): kerning, ligatures and
  marks come from the font's own tables.
- **Glyphs** are rasterised once, with ab_glyph, at `sdf_px` per em into a
  one-channel signed distance field (`sdf_spread` pixels each side), packed
  in a shelf atlas of `atlas_size`². One rasterisation serves every size,
  outline and soft shadow; the shader antialiases from the field's
  screen-space rate of change.
- **Lines** break greedily at spaces and at `\n`; a word longer than the
  line breaks between glyphs. `max_lines` cuts the rest. A text node is as
  wide as its widest line, up to what layout gives it, and as tall as its
  lines × `size` × `line_height`. `text_align` places each line.
- **Fonts** come from the game: a description names font files, each with
  an optional `weight` on a variable font's `wght` axis. The engine commits
  one, Nunito (SIL Open Font License, `crates/sariema-ui/fonts/OFL.txt`),
  which covers Latin-1 and Latin Extended-A except the Dutch `ĳ` and the
  long `ſ`, so all of Portuguese.
- **Missing glyphs** are counted: `Ui::missing_glyphs(node)` and the
  `missing_glyphs` field of `sariema ui render --json`.

**Other scripts.** rustybuzz already shapes every script HarfBuzz does
(Arabic joining, Indic reordering, marks). What a script beyond Latin,
Greek and Cyrillic still needs: a font that covers it (add it to the
description; a fallback chain per node is not there yet), bidirectional
reordering of mixed lines (the `unicode-bidi` crate, applied per line before
placement), and line breaking by UAX #14 instead of at spaces
(`unicode-linebreak`), which CJK needs. Glyph quads are placed from shaped
advances and offsets, so none of the drawing changes.

## Animation

Any property animates: `x`, `y`, `scale_x`, `scale_y`, `rotation`,
`opacity`, `width`, `height`, `progress`, `reveal` and the four channels of
`fill` (descriptions also take `position`, `scale`, `size` and `fill` as
groups).

- **Tweens** take a duration and a cubic Bézier: `out` is
  `cubic-bezier(.22, 1, .36, 1)` for what enters, leaves or answers a touch,
  `in_out` is `(.65, 0, .35, 1)` for what changes shape on screen, `linear`,
  or any four numbers. A tween ends exactly on its target, on the step its
  duration reaches.
- **Springs** are set like iOS's: `response` seconds and `damping` (1 does
  not overshoot), integrated at `spring_hz` fixed substeps so they are
  deterministic; they rest within `spring_rest_delta` and `spring_rest_speed`.
- **Typewriter** reveals a text's characters at `rate` per second, pausing
  `pause` seconds after `.`, `!`, `?` and `…`. The text is laid out whole
  from the start, so the box does not grow while it types.
- **Interrupting**: an animation that starts on a property takes it over
  from the value it has then, and a spring keeps the velocity it had, so
  nothing jumps. The one taken over stops before moving in that step.
- **Delays, staggers and sequences**: a track's `delay`; `nodes` with a
  `stagger` start one after the other; `after` starts a track when the
  previous track's last tween ends. An explicit `from` holds through the
  delay, like CSS's `animation-fill-mode: backwards`, so staggered words do
  not show at their end place first.
- **Reduced motion** (`reduced_motion`): moves, resizes and typewriters
  jump to their end; fades, colours, rings and strokes still animate.

## Input and widgets

`pointer_down`, `pointer_move`, `pointer_up` and `pointer_cancel` take a
pointer id and pixels. The topmost interactive node under a pointer when it
goes down captures it: moves and the release go to that node wherever the
pointer is, so a thumb on the joystick and another on a button never mix.
Hit testing goes through every transform, respects clipping, and skips
invisible and fully transparent nodes. Events: `Press`, `Drag`, `Release`,
`Click` (released over the node it pressed), `Stick`, `Tap`, `Held`,
`Focus`. Nodes with `focus` form a focus order for `focus_next`,
`focus_prev` and `activate`, for gamepads and keyboards.

Widgets are a node (the touch zone) that moves named descendants, so their
look is ordinary nodes:

- **Joystick**: the base appears under the thumb (opacity and scale tween
  in over `appear_s`); the pin follows the thumb 1:1 up to `radius`; the
  output is 0 inside `dead_zone` and grows linearly to 1 at `saturation`,
  keeping the direction (defaults 40 units, 0.15 and 0.7, from the movement
  research). On release the pin and the base spring back (0.18 s and 0.35 s,
  damping 1). `Ui::stick(node)` reads the output.
- **Action button**: the face scales to `press_scale` (0.92 over 0.12 s) on
  press; with a `ring` (one name, or a list that fills together, such as an
  ink outline behind the arc), holding fills it linearly over `hold_s`
  (1.2 s) and a full ring fires `Held`; an earlier release fires `Tap` and
  empties the ring over `empty_s` (0.2 s).

## The draw list and the renderer

`DrawList` holds `Quad`s (152 bytes: box, 2×3 transform, texture
coordinates, fill, stroke, radii, parameters, kind, gradient geometry) and
`Batch`es (texture,
scissor in pixels, range). Quads keep painter's order; a batch breaks only
where the texture or the scissor changes, and shapes need no texture, so
boxes, rings, strokes and the text over them share one batch: a HUD of
boxes and text is one draw call. Images break batches where they alternate
with text (pack icons into one image and use `region`).

`sariema_render::ui::UiPass` draws a list over any target: one instanced
draw of 4 vertices per batch, the batch's texture bound and its scissor set.
Its WGSL evaluates each shape's distance field per pixel (rounded boxes with
elliptical corners after Quilez, borders, CSS-like Gaussian shadows,
gradients, arcs, capsules) and samples the glyph atlas. A gradient's second
colour rides in the texture coordinates a box does not use and its geometry
in spare parameters, so the quad stays 152 bytes; the shader passes 11
vectors between stages and reads 11 attributes, inside WebGL2's 15 and 16
(a test builds the pipeline on a device held to WebGL2's limits). Its pipeline, sampler and buffers are
created in `UiPass::new`; the instance buffer grows only when a frame has
more quads than ever. Colours are sRGB with straight alpha and blend
premultiplied; on an sRGB target they are turned to linear first, so there
blending happens in linear light, where a browser blends in sRGB.
`UiHeadless` draws on its own device into an image. A `Renderer` holds one,
made with it (`Quality::ui_quads`, 1024, sets where its instance buffer
starts): `Renderer::set_ui_texture` and `Renderer::render_ui` draw the HUD
over the frame `render` drew, and its pass, draw calls and uploaded bytes
count in that frame's `FrameStats`.

## Using it from code

```rust
let mut ui = Ui::load(Path::new("hud.json"))?;     // or Ui::from_desc(&desc, loader)
ui.set_screen(w as f32, h as f32, SafeArea { left: 47.0 * s, bottom: 21.0 * s, ..Default::default() });
let objective = ui.node("objective").unwrap();
ui.play("enter")?;
ui.animate(objective, Prop::Y, Animation::tween(0.0, 0.26, Ease::OUT).from(-14.0))?;
// each simulation step:
for event in ui.step() { /* Stick, Tap, Held, Click… */ }
// a node set to a step's value, drawn there until the next step:
ui.snap(bar)?;
// each rendered frame, alpha between the last two steps:
let list = ui.draw(alpha);
if ui.take_atlas_changed() { let (side, texels) = ui.atlas(); pass.set_texture(0, side, side, UiTextureFormat::R8, texels); }
pass.draw(&mut encoder, target, bytemuck::cast_slice(&list.quads), &batches, None);
```

Images are textures `1 + index` of `Ui::images()` (premultiplied RGBA8).
`sariema-ui` uses no threads and passes `cargo check` for
`wasm32-unknown-unknown`, for a web build.
`sariema_ui::Quad` and `sariema_render::ui::UiInstance` have the same
layout, checked at compile time in the CLI, so the quads cast without a copy.

## Using it from the CLI

```sh
sariema ui render <ui.json> --width W --height H [--safe L,T,R,B] \
    [--state k=v]... [--time S] [--filmstrip N] [--background img|#colour] -o out.png [--json]
```

`--state`, applied in order before the first step: `node.prop=value`
(`x`, `y`, `scale`, `scale_x`, `scale_y`, `rotation`, `opacity`, `width`,
`height`, `progress`, `reveal`, `fill`, `text`, `visible`), `play=animation`,
or `down=`, `move=`, `up=pointer:x,y` in pixels. `--time` runs that many
seconds of steps; `--filmstrip N` draws N frames evenly from 0 to `--time`
side by side. The reply has the quads, batches and draw calls of the last
frame, the nodes, the steps run, every event with its step, and text nodes
missing glyphs.

New error codes: `E_UI_FONT` (a font file that does not load, or a weight
on a font without a `wght` axis), `E_UI_IMAGE`, `E_UI_UNKNOWN` (a node, font,
image or animation name), `E_UI_ATLAS_FULL` (raise `atlas_size`),
`E_UI_ANIMATIONS` (raise `animations`). Descriptions that break the schema
give `E_SCHEMA` with the JSON path; a bad `--state` gives `E_USAGE`.

## Settings

`UiSettings` (the description's `settings`; every field has a default):

| Field | Default | Meaning |
|---|---|---|
| `reference` | [844, 390] | design size in UI units, given in landscape |
| `step_s` | 1/60 | seconds per step |
| `atlas_size` | 1024 | side of the glyph atlas |
| `sdf_px` | 48 | pixels per em glyphs are rasterised at |
| `sdf_spread` | 6 | field reach in rasterised pixels; bounds outlines and text-shadow blur |
| `animations` | 512 | animations running or waiting at once |
| `spring_hz` | 240 | spring substeps per second |
| `spring_rest_delta`, `spring_rest_speed` | 0.001, 0.01 | when a spring is at rest |
| `pointers` | 10 | pointers down at once |
| `reduced_motion` | false | see Animation |

`UiPass::new` takes the quads its instance buffer starts with.

## Numbers

`cargo run --release -p sariema-ui --example ui_bench`, Apple M5, one core.
A 202-node screen (50 cards, each a bordered box with a shadow, a disc, a
13-unit text and a ring, in a wrapping row): 690 quads, **1 batch, 1 draw
call**.

| Frame (step + draw list) | µs |
|---|---|
| first layout + draw list | 244 |
| static | 15.4 |
| 50 nodes moving and fading (tweens and springs) | 23.1 |
| 50 nodes changing width (a layout every step) | 144.9 |

## Known limits

- Clipping is the node's axis-aligned box on screen; rounded clip masks and
  clipping under rotation are not there.
- One shadow per box: a second shadow is a sibling box with only a
  `shadow`. Gradients have two stops; more need stacked boxes.
- No per-range text colour or style inside one text node (use two nodes);
  no italic synthesis; no ellipsis when `max_lines` cuts.
- Text nodes shrink to their widest line; CSS would make a wrapping box as
  wide as its container.
- Lines break at spaces only, and lay out left to right (see Other scripts).
- A size animation lays out every step; animate scale where a size does not
  have to change the layout.
- Text input fields, scrolling and an announcement queue are not part of
  this layer.

## What we took from other engines

- **Bevy UI**: taffy for flexbox layout and a retained node tree that a
  render pass turns into batched instanced quads.
- **Godot**: the `expand` stretch aspect for one reference size across
  aspect ratios and orientations, and safe-area insets read from the system.
- **Mapbox TinySDF**: the distance field from antialiased coverage through
  a Felzenszwalb–Huttenlocher Euclidean distance transform, and coverage
  quantised to bytes before it (the rasteriser's tiny residues otherwise
  read as edges).
- **Valve's SDF text** (Green, 2007): one small field atlas for text at any
  size, with outlines and soft shadows from thresholds.
- **Inigo Quilez's distance functions**: rounded boxes with a radius per
  corner and the elliptical-corner estimate, capsules and arcs.
- **iOS / SwiftUI**: springs set by response and damping.
- **CSS**: the box model, radius scaling when corners overlap, box-shadow
  blur as a Gaussian of half its radius, fill-mode backwards for delays, and
  the geometry of `linear-gradient` (angle, corner-to-corner line) and
  `radial-gradient` (`circle` to the farthest corner, `ellipse` with
  percentage radii), interpolated premultiplied.
