// SPDX-License-Identifier: Apache-2.0

//! Fonts, shaping and line breaking.
//!
//! Shaping is rustybuzz (a port of HarfBuzz): kerning, ligatures and marks
//! come from the font's own tables, for any script the font covers. Lines
//! break at spaces and at `\n`; a word wider than the line breaks between
//! glyphs. Lines are laid out left to right.

use ab_glyph::{Font as _, FontVec, GlyphId, PxScale, VariableFont, point};
use rustybuzz::ttf_parser::Tag;
use rustybuzz::{Face, UnicodeBuffer, Variation};
use serde::Deserialize;

use crate::atlas::{Atlas, Cell};

pub(crate) struct Font {
    data: Vec<u8>,
    outlines: FontVec,
    variations: Vec<Variation>,
    units_per_em: f32,
    /// Over the em; the descender is negative.
    pub ascender: f32,
    pub descender: f32,
    height_unscaled: f32,
}

impl Font {
    /// `weight` sets the `wght` axis of a variable font.
    pub fn new(data: Vec<u8>, weight: Option<f32>) -> Result<Font, String> {
        let mut face = Face::from_slice(&data, 0).ok_or("not a TrueType or OpenType font")?;
        let mut outlines = FontVec::try_from_vec(data.clone()).map_err(|e| e.to_string())?;
        let mut variations = Vec::new();
        if let Some(value) = weight {
            if !outlines.set_variation(b"wght", value) {
                return Err("has no weight axis; give one font file per weight".into());
            }
            variations.push(Variation {
                tag: Tag::from_bytes(b"wght"),
                value,
            });
            face.set_variations(&variations);
        }
        let units_per_em = f32::from(face.units_per_em() as u16);
        Ok(Font {
            ascender: f32::from(face.ascender()) / units_per_em,
            descender: f32::from(face.descender()) / units_per_em,
            height_unscaled: outlines.height_unscaled(),
            data,
            outlines,
            variations,
            units_per_em,
        })
    }

    /// Shapes `text` into `out`, rasterising into `atlas` the glyphs it has
    /// not seen. Returns `false` when the atlas is full.
    pub fn shape(
        &self,
        font: u32,
        text: &str,
        cache: &mut std::collections::HashMap<(u32, u16), Option<Cell>>,
        atlas: &mut Atlas,
        sdf_px: f32,
        out: &mut Vec<Shaped>,
    ) -> bool {
        out.clear();
        let mut face = Face::from_slice(&self.data, 0).expect("checked in Font::new");
        face.set_variations(&self.variations);
        let mut chars_before = 0u32;
        for (n, paragraph) in text.split('\n').enumerate() {
            if n > 0 {
                out.push(Shaped {
                    glyph: 0,
                    advance: 0.0,
                    offset: [0.0; 2],
                    char_index: chars_before,
                    kind: GlyphKind::Newline,
                    cell: None,
                });
                chars_before += 1;
            }
            if paragraph.is_empty() {
                continue;
            }
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(paragraph);
            let shaped = rustybuzz::shape(&face, &[], buffer);
            let char_at: Vec<(usize, char)> = paragraph.char_indices().collect();
            for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                let byte = info.cluster as usize;
                let index = char_at.partition_point(|&(b, _)| b < byte);
                let c = char_at.get(index).map_or(' ', |&(_, c)| c);
                let glyph = info.glyph_id as u16;
                let kind = if c.is_whitespace() {
                    GlyphKind::Space
                } else {
                    GlyphKind::Ink
                };
                let cell = match kind {
                    GlyphKind::Ink => match cache.get(&(font, glyph)) {
                        Some(cell) => *cell,
                        None => {
                            let Some(cell) = self.rasterise(glyph, atlas, sdf_px) else {
                                return false;
                            };
                            cache.insert((font, glyph), cell);
                            cell
                        }
                    },
                    _ => None,
                };
                let em = self.units_per_em;
                out.push(Shaped {
                    glyph,
                    advance: pos.x_advance as f32 / em,
                    offset: [pos.x_offset as f32 / em, -pos.y_offset as f32 / em],
                    char_index: chars_before + index as u32,
                    kind,
                    cell,
                });
            }
            chars_before += char_at.len() as u32;
        }
        true
    }

    /// `Some(None)` for a glyph without an outline; `None` when the atlas is full.
    fn rasterise(&self, glyph: u16, atlas: &mut Atlas, sdf_px: f32) -> Option<Option<Cell>> {
        let scale = PxScale::from(sdf_px * self.height_unscaled / self.units_per_em);
        let positioned = GlyphId(glyph).with_scale_and_position(scale, point(0.0, 0.0));
        let Some(outlined) = self.outlines.outline_glyph(positioned) else {
            return Some(None);
        };
        let bounds = outlined.px_bounds();
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        let mut coverage = vec![0.0f32; (w * h) as usize];
        // Quantised to bytes, as TinySDF's canvas is: the rasteriser's
        // accumulation leaves traces of coverage that would read as edges.
        outlined.draw(|x, y, c| {
            if x < w && y < h {
                coverage[(y * w + x) as usize] = (c.clamp(0.0, 1.0) * 255.0).round() / 255.0;
            }
        });
        atlas
            .insert(&coverage, w, h, [bounds.min.x, bounds.min.y])
            .map(Some)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlyphKind {
    Ink,
    Space,
    Newline,
}

/// A shaped glyph; lengths are over the em.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shaped {
    /// 0 when the font has no glyph for the character.
    pub glyph: u16,
    pub advance: f32,
    pub offset: [f32; 2],
    /// Characters of the text before this glyph's cluster.
    pub char_index: u32,
    pub kind: GlyphKind,
    pub cell: Option<Cell>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A laid-out line: glyphs `start..end`, trailing spaces excluded, and its
/// width in UI units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub start: u32,
    pub end: u32,
    pub width: f32,
}

/// Greedy line breaking of `glyphs` at `size` into lines no wider than
/// `max_width`, at most `max_lines` of them, given to `out` in order.
pub(crate) fn break_lines(
    glyphs: &[Shaped],
    size: f32,
    max_width: f32,
    max_lines: u32,
    mut out: impl FnMut(Line),
) {
    if glyphs.is_empty() || max_lines == 0 {
        return;
    }
    // A little slack so a line measured at exactly its own width fits.
    let limit = max_width + 1e-3;
    let mut start = 0usize;
    let mut pen = 0.0f32;
    let mut ink_end = 0usize;
    let mut ink_width = 0.0f32;
    let mut brk: Option<(usize, f32, usize)> = None;
    let mut i = 0usize;
    let mut emitted = 0u32;
    macro_rules! emit {
        ($end:expr, $width:expr) => {
            out(Line {
                start: start as u32,
                end: $end as u32,
                width: $width,
            });
            emitted += 1;
            if emitted == max_lines {
                return;
            }
        };
    }
    while i < glyphs.len() {
        let g = &glyphs[i];
        let advance = g.advance * size;
        match g.kind {
            GlyphKind::Newline => {
                emit!(ink_end, ink_width);
                start = i + 1;
                (pen, ink_end, ink_width, brk) = (0.0, start, 0.0, None);
            }
            GlyphKind::Space => {
                if ink_end > start {
                    brk = Some((ink_end, ink_width, i + 1));
                }
                pen += advance;
            }
            GlyphKind::Ink => {
                if pen + advance > limit && ink_end > start {
                    if let Some((end, width, next)) = brk.take() {
                        emit!(end, width);
                        start = next;
                        pen = glyphs[next..i].iter().map(|g| g.advance * size).sum();
                        (ink_end, ink_width) = (i, pen);
                    } else {
                        emit!(ink_end, ink_width);
                        start = i;
                        (pen, ink_end, ink_width) = (0.0, i, 0.0);
                    }
                    continue;
                }
                pen += advance;
                ink_end = i + 1;
                ink_width = pen;
            }
        }
        i += 1;
    }
    emit!(ink_end, ink_width);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(n: usize) -> impl Iterator<Item = Shaped> {
        (0..n).map(|_| Shaped {
            glyph: 1,
            advance: 1.0,
            offset: [0.0; 2],
            char_index: 0,
            kind: GlyphKind::Ink,
            cell: None,
        })
    }

    fn space() -> Shaped {
        Shaped {
            kind: GlyphKind::Space,
            ..word(1).next().unwrap()
        }
    }

    #[test]
    fn breaks_at_spaces_and_inside_long_words() {
        let mut g: Vec<Shaped> = word(3).collect();
        g.push(space());
        g.extend(word(3));
        g.push(space());
        g.extend(word(9));
        let lines = |max_width: f32, max_lines: u32| {
            let mut lines = Vec::new();
            break_lines(&g, 1.0, max_width, max_lines, |l| lines.push(l));
            lines
        };
        let spans: Vec<(u32, u32, f32)> = lines(7.0, u32::MAX)
            .iter()
            .map(|l| (l.start, l.end, l.width))
            .collect();
        assert_eq!(spans, [(0, 7, 7.0), (8, 15, 7.0), (15, 17, 2.0)]);
        assert_eq!(lines(7.0, 2).len(), 2);
        let one = lines(f32::INFINITY, u32::MAX);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].width, 17.0);
    }
}
