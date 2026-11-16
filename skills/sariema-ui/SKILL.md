---
name: sariema-ui
description: Use when a Sariema game needs a HUD, dialogue box, objective, menu or touch controls (joystick, action button), when a screen must be checked without a device, when text overflows, wraps badly or lacks glyphs, when HUD pieces collide with the notch or each other, or when UI motion must enter, leave or be interrupted smoothly.
---

# Sariema UI

## Overview

A screen is a JSON description (`schemas/ui.schema.json`): fonts, images,
a tree of nodes laid out with flexbox, and named animations. You cannot
watch a phone, so you **render moments headless and look at the PNG**:
`sariema ui render` lays the screen out at a real size and safe area,
plays animations and touches for a given time, and reports quads, draw
calls, events and missing glyphs.

Core rule: **lay out, don't place.** Put the HUD in one `"safe": true` node
and anchor each piece with `absolute` + `left/top/right/bottom` or flex
rows; never compute pixel positions for one screen size. Then check three
aspect ratios and portrait.

## When to use

- Building or changing any HUD piece, dialogue, menu or control.
- "The text is cut / wraps in two / shows boxes": render it, read
  `missing_glyphs` and look at the lines.
- "The HUD hits the notch or the thumbs": render with `--safe` at 844×390,
  667×375 and 932×430.
- "The animation jumps / feels stiff": render a `--filmstrip`.
- Not for: drawing the art itself (icons, nine-slice frames are image files).

## Workflow

1. Write the description. Units are the reference size (844 × 390, a phone
   sideways); fonts and images are files next to it.
2. Render the resting state at the phone's pixels and insets:
   `sariema ui render hud.json --width 2532 --height 1170 --safe 141,0,141,63 -o rest.png --json`
3. Render each moment: `--state play=enter --time 0.15` for mid-animation,
   `--filmstrip 6 --time 0.5` for the whole motion, `--state down=1:x,y`
   for touches. Look at every PNG.
4. Read the reply: `draw_calls` (a HUD of boxes and text is 1),
   `missing_glyphs` (must be empty), `events` (taps, holds, stick values).
5. Check 667×375, 1024×768 and portrait before calling it done.

## Worked example

`examples/ui/objective.json` in the engine repository: an objective chip
whose `done` animation turns the disc green, draws a check, swaps the label
and strikes the phrase.

```sh
sariema ui render examples/ui/objective.json --width 844 --height 390 \
    --background "#5B7A99" --state play=done --time 0.3 -o mid.png
# drew 1 frame of 844 × 390 into mid.png: 34 quads in 1 draw call
sariema ui render examples/ui/controls.json --width 844 --height 390 \
    --state down=2:788,334 --time 1.3 --filmstrip 4 -o hold.png --json
# … "events":[{"event":"press","node":"action",…,"step":1},
#             {"event":"held","node":"action","step":72}] …
```

The track that strikes the phrase, from the description:

```json
{ "node": "strike", "prop": "progress", "to": 1, "duration": 0.34, "ease": "in_out", "delay": 0.14 }
```

## In a game

`"hud": "hud.json"` under `game` in `sariema.json` puts the description,
its fonts and its images in the game pack. The logic drives it by name:
`ui_visible`, `ui_text`, `ui_value` (a node's progress) and `ui_play` (an
animation). A joystick widget drives the move axis that takes the touch
stick; a button node named like an input button (`action`) presses that
button. Over the HUD the engine draws the cinema bars, the dialogue line
riding above the bottom one (with the HUD's first font) and the fade.
While a cutscene holds control the engine hides the HUD's root nodes and
brings them back once its cinema bars are gone; put `"groups": ["cutscene"]` on one that must stay.
A `ui_text` or `say` line over 4096 bytes (`hud.max_text_bytes`) is
`rejected` with `E_UI_TEXT_TOO_LONG`, and one whose glyphs no longer fit the
atlas with `E_UI_ATLAS_FULL`; the line is hidden, the node left empty.
Hide nothing yourself on `cutscene_started`. Check the frames in the game,
not only the description:

```sh
sariema run --headless --game climb.srmg --input examples/climb/input/play.json \
    --steps 900 --screenshot strip.png --filmstrip 8 --width 844 --height 390 --json
```

`examples/climb/hud.json` is a HUD with an objective that animates in on
the first move and checks itself off when the cutscene ends.

## Quick reference

| Need | Write |
|---|---|
| HUD inside notch and home bar | parent `{ "safe": true, "children": [...] }`, screen with `--safe L,T,R,B` |
| Corner piece | `"absolute": true, "right": 16, "top": 12` |
| Gradient face or vignette | `"gradient": {"type": "radial", "center": [0.5, 0.32], "from", "to", "stops": [0, 0.7]}`; `"type": "linear", "angle": 160` for a sheen |
| Two shadows | a sibling box under it with only `radius` and the second `shadow` |
| Paper card, hand-cut corners | `"fill", "border": {"width","color"}, "radius": {"x": [16, 14, 17, 13], "y": [14, 17, 13, 16]}, "shadow": {"y": 3, "color"}` |
| Text that wraps at a width | `"text", "font", "size", "line_height", "max_width"` (+ `"max_lines"`) |
| Dialogue that types | `"reveal": 0` on the text, track `{ "prop": "reveal", "typewriter": {} }` (30/s, 70 ms after `.!?`) |
| Enter | tracks `y` from −14 and `opacity` from 0, `"duration": 0.26`, ease `out` |
| Words one by one | `"nodes": [...], "stagger": 0.045` with `from` values |
| Something after something | `"after": true` on the later track |
| Bounce of celebration | `"spring": { "response": 0.5, "damping": 0.62 }` |
| Check mark, strike-through | `"stroke": { "points": [...], "viewbox": [w, h], "width": 4 }`, animate `progress` |
| Hold-to-act ring | button widget with `"ring"`; ring node `{ "ring": { "width": 5 }, "progress": 0 }` |
| Outlined arc | `"ring": ["outline", "arc"]`: a wider ink ring behind the arc fills with it |
| Joystick | zone with `"widget": { "joystick": { "base": "b", "pin": "p" } }` |
| Fade or letterbox | a box with `fill`, animate `opacity` or `scale_y` |

Props: `x`, `y`, `position`, `scale`, `scale_x`, `scale_y`, `rotation`,
`opacity`, `width`, `height`, `size`, `progress`, `reveal`, `fill`.

## Common mistakes

| Mistake | What reports it | Fix |
|---|---|---|
| Animating `width`/`height` to grow a card that has children | `µs` cost; children reflow every step | animate `scale`, or a childless background box |
| A text that names no font with two fonts loaded | `E_SCHEMA` "names no font" | add `"font"` |
| A name animations or `--state` use that does not exist | `E_UI_UNKNOWN` naming it | check `name` spelling |
| A misspelt key (`"colour"`) | `E_SCHEMA` with the JSON path | follow the schema |
| Text in a script the font lacks | `missing_glyphs` in the reply, boxes in the PNG | add a font that covers it |
| A label that grows into the thumb area | the PNG; `max_width` missing | give `max_width` and `max_lines` |
| Images between texts | `draw_calls` > 1 | pack icons into one image with `region`, or draw icons as `stroke`/`ring` |
| Expecting time 0 to show an animation | the PNG shows the start | add `--time` |
| `ease-in` for an entrance | it feels late | entrances and touches use `out` |
