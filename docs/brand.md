# Brand

The mark is a seriema, the long-legged bird of the Brazilian cerrado and
sertão, running across the sun at noon. The colours come from the same
land: clay, dry scrub, cactus, the hard blue sky of the drought and the
warm night after it.

## Files

Everything is in `assets/brand/`:

| File | Use |
|---|---|
| `mark.svg` | The mark in colour, from 32 px up |
| `mark-mono.svg` | The mark in one ink (`currentColor`): the sun in the ink, the bird cut out |
| `favicon.svg`, `favicon-32.png` | The mark drawn for 16–32 px: thicker legs, one crest feather, no eye |
| `wordmark.svg` | "sariema" alone, outlined, in night ink |
| `lockup.svg`, `lockup-dark.svg` | Mark and word side by side, for light and for dark backgrounds |
| `og.png` | The 1200×630 social card |

`mark.svg`, `mark-mono.svg` and `favicon.svg` are drawn by hand. The word,
the lockups and the PNGs are made from them and Geist SemiBold by
`npm --prefix site run brand`.

## Using the mark

- **Clear space.** Keep a quarter of the sun's diameter free around the
  mark, and around the lockup measured on its mark.
- **Size.** The mark from 32 px; under that, the favicon drawing. The
  lockup from 20 px tall.
- **Backgrounds.** The colour mark sits on cream, on night, or on any calm
  colour where the sun disc stays distinct. On a photo or a busy image,
  use `mark-mono.svg` in cream or night.
- **The word** is lowercase Geist SemiBold at −0.035 em tracking, its
  x-height centred on the sun.

Do not:

- recolour the bird or the sun, or swap their colours;
- draw the bird without its sun, or the sun without its bird;
- rotate, stretch, outline or add a shadow to the mark;
- set the word in another face or in capitals;
- use clay as a text colour on cream surfaces (4.2:1, under AA): text
  takes the deeper accent instead.

## Colours

| Name | Hex | For |
|---|---|---|
| barro (clay) | `#B4532A` | the primary action in the light theme, the bird's legs and bill |
| sol a pino (noon sun) | `#E3A23B` | the sun; the primary action and accent in the dark theme |
| mandacaru (cactus) | `#4F6B3A` | "works" and success |
| caatinga (dry scrub) | `#9AA284` | quiet decoration |
| areia (sand) | `#EFE3CC` | light surfaces; text in the dark theme |
| céu de seca (drought sky) | `#4C86B5` | information |
| noite (night) | `#1C1814` | the bird; text in the light theme |

## Theme tokens

The site defines these in `site/app/theme.css` and maps f.UI's and
Fumadocs' roles onto them. The light theme is a warm cream in the spirit
of Solarized light; the dark one is a warm night, never a cold grey.

| Token | Light | Dark |
|---|---|---|
| `--sr-bg` | `#FBF6EC` | `#17130F` |
| `--sr-surface` | `#F4EBDA` | `#1F1A15` |
| `--sr-surface-2` | `#EFE3CC` | `#2A231C` |
| `--sr-card` | `#FFFAF1` | `#211B16` |
| `--sr-line` | `#E3D5BA` | `#352B22` |
| `--sr-text` | `#1C1814` | `#EFE3CC` |
| `--sr-text-muted` | `#5E5446` | `#B3A48A` |
| `--sr-accent` (links, current item) | `#9C4422` | `#E3A23B` |
| `--sr-action` / `--sr-on-action` | `#B4532A` / `#FFFFFF` | `#E3A23B` / `#17130F` |
| `--sr-green` | `#4F6B3A` | `#8FB06E` |
| `--sr-sky` | `#2F6A99` | `#7DA6CC` |

Contrast of the text pairs (WCAG 2; AA is 4.5:1 for body text):

| Pair | Light | Dark |
|---|---|---|
| text on bg | 16.4 | 14.5 |
| text on surface-2 | 13.9 | 12.2 |
| muted on bg | 6.9 | 7.6 |
| muted on surface-2 | 5.8 | 6.3 |
| muted on card | 7.1 | 7.0 |
| accent on bg | 6.0 | 8.4 |
| accent on surface | 5.4 | 7.8 |
| on-action on action | 5.0 | 8.4 |
| green on surface | 5.1 | 7.1 |
| sky on surface | 4.9 | 6.7 |

## Type

Geist for text and Geist Mono for code, as f.UI sets them. Numbers that
change in place use tabular figures.
