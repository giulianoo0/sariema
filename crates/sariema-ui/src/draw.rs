// SPDX-License-Identifier: Apache-2.0

//! The draw list: quads in painter's order, in batches that share a texture
//! and a scissor. Shapes need no texture, so they join any batch; a batch
//! breaks only where the texture or the scissor changes.

use bytemuck::{Pod, Zeroable};
use glam::{Affine2, Vec2};

use crate::input::bounds;
use crate::node::{Gradient, GradientKind, Kind, Node, Prop, Text};
use crate::text::Align;
use crate::ui::Ui;

/// What a quad draws; the shader branches on it.
pub mod kind {
    /// A box with per-corner elliptical radii, a border and a blur.
    pub const RECT: u32 = 0;
    /// A glyph from the distance-field atlas, with outline and softness.
    pub const GLYPH: u32 = 1;
    pub const IMAGE: u32 = 2;
    /// A ring's track and its arc, with round caps.
    pub const RING: u32 = 3;
    /// One segment of a stroke, with round caps.
    pub const SEGMENT: u32 = 4;
    /// A [`RECT`] filled from `fill` at `params[2..4]` to `uv` at
    /// `gradient[0..2]`.
    pub const LINEAR: u32 = 5;
    /// A [`RECT`] filled from `fill` at the centre `params[2..4]` to `uv`
    /// at the radii `gradient[0..2]`, starting at the fraction `gradient[2]`
    /// of them.
    pub const RADIAL: u32 = 6;
}

/// Texture of the glyph atlas. An image's texture is `1 +` its index in
/// [`Ui::images`].
pub const ATLAS_TEXTURE: u32 = 0;
/// A batch of shapes only.
pub const NO_TEXTURE: u32 = u32::MAX;

/// One instance of the UI shader. Its layout is the renderer's
/// `UiInstance`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Quad {
    /// The shape's box in its own units: x, y, width, height.
    pub rect: [f32; 4],
    /// Own units to pixels, `[a, b, c, d, tx, ty]`:
    /// x' = a·x + c·y + tx, y' = b·x + d·y + ty.
    pub xform: [f32; 6],
    /// Glyphs and images: texture coordinates u0, v0, u1, v1. Segments:
    /// the two ends in own units. Gradients: the second colour.
    pub uv: [f32; 4],
    /// Straight-alpha sRGB, opacity applied.
    pub fill: [f32; 4],
    /// Border, outline or ring track colour.
    pub stroke: [f32; 4],
    /// Corner radii along x: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
    pub radii_y: [f32; 4],
    /// By kind. Rect: border width, blur; gradients add their start point
    /// or centre. Glyph: outline and softness in field units, field units
    /// per texture coordinate. Ring: width, start angle in radians,
    /// progress. Segment: half width.
    pub params: [f32; 4],
    pub kind: u32,
    /// Gradients only, see [`kind::LINEAR`] and [`kind::RADIAL`].
    pub gradient: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    /// [`ATLAS_TEXTURE`], `1 +` an image index, or [`NO_TEXTURE`].
    pub texture: u32,
    /// Pixels: x, y, width, height.
    pub scissor: [u32; 4],
    pub start: u32,
    pub count: u32,
}

#[derive(Default)]
pub struct DrawList {
    pub quads: Vec<Quad>,
    pub batches: Vec<Batch>,
}

impl DrawList {
    fn push(&mut self, quad: Quad, texture: u32, scissor: [u32; 4]) {
        match self.batches.last_mut() {
            Some(b)
                if b.scissor == scissor
                    && (texture == NO_TEXTURE
                        || b.texture == NO_TEXTURE
                        || b.texture == texture) =>
            {
                if b.texture == NO_TEXTURE {
                    b.texture = texture;
                }
                b.count += 1;
            }
            _ => self.batches.push(Batch {
                texture,
                scissor,
                start: self.quads.len() as u32,
                count: 1,
            }),
        }
        self.quads.push(quad);
    }
}

impl Ui {
    /// Builds the draw list at `alpha` between the last two steps (1 is
    /// the last step). Lays out first if anything changed since.
    pub fn draw(&mut self, alpha: f32) -> &DrawList {
        if self.layout_dirty {
            self.layout();
        }
        let mut list = std::mem::take(&mut self.draw);
        list.quads.clear();
        list.batches.clear();
        let s = self.screen;
        let full = [0, 0, s.width.ceil() as u32, s.height.ceil() as u32];
        let mut painter = Painter {
            ui: self,
            alpha: alpha.clamp(0.0, 1.0),
            list: &mut list,
            screen: full,
        };
        painter.node(
            self.root,
            Affine2::from_scale(Vec2::splat(s.scale)),
            1.0,
            full,
        );
        self.draw = list;
        &self.draw
    }
}

struct Painter<'a> {
    ui: &'a Ui,
    alpha: f32,
    list: &'a mut DrawList,
    screen: [u32; 4],
}

impl<'a> Painter<'a> {
    fn node(&mut self, index: u32, parent: Affine2, parent_opacity: f32, scissor: [u32; 4]) {
        let ui = self.ui;
        let n = &ui.nodes[index as usize];
        if !n.visible {
            return;
        }
        let opacity = parent_opacity * n.at(Prop::Opacity, self.alpha).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let (local, [w, h]) = n.local(self.alpha);
        let xf = parent * local;
        let channel = |p| n.at(p, self.alpha).clamp(0.0, 1.0);
        let fill = [
            channel(Prop::Red),
            channel(Prop::Green),
            channel(Prop::Blue),
            channel(Prop::Alpha) * opacity,
        ];
        if let Some(s) = n.shadow
            && matches!(n.kind, Kind::Box | Kind::Image(_))
        {
            let mut q = self.rect(xf, [s.x, s.y, w, h], n, faded(s.color.0, opacity));
            q.params = [0.0, s.blur.max(0.0), 0.0, 0.0];
            self.list.push(q, NO_TEXTURE, scissor);
        }
        match &n.kind {
            Kind::Box => {
                let border = n.border.filter(|b| b.width > 0.0 && b.color.0[3] > 0.0);
                if fill[3] > 0.0 || border.is_some() || n.gradient.is_some() {
                    let mut q = self.rect(xf, [0.0, 0.0, w, h], n, fill);
                    if let Some(b) = border {
                        q.stroke = faded(b.color.0, opacity);
                        q.params[0] = b.width;
                    }
                    if let Some(g) = &n.gradient {
                        gradient(&mut q, g, w, h, opacity);
                    }
                    self.list.push(q, NO_TEXTURE, scissor);
                }
            }
            Kind::Text(t) => self.text(xf, n, t, fill, opacity, scissor),
            Kind::Image(image) => self.image(xf, w, h, image, fill, scissor),
            Kind::Ring(ring) => {
                let q = Quad {
                    rect: [0.0, 0.0, w, h],
                    xform: xform(xf),
                    fill,
                    stroke: faded(ring.track.0, opacity),
                    params: [
                        ring.width,
                        ring.start.to_radians(),
                        n.at(Prop::Progress, self.alpha).clamp(0.0, 1.0),
                        0.0,
                    ],
                    kind: kind::RING,
                    ..Quad::default()
                };
                self.list.push(q, NO_TEXTURE, scissor);
            }
            Kind::Stroke(stroke) => {
                let map = |p: [f32; 2]| match stroke.viewbox {
                    Some([vw, vh]) => Vec2::new(p[0] * w / vw, p[1] * h / vh),
                    None => Vec2::from(p),
                };
                let total: f32 = stroke
                    .points
                    .windows(2)
                    .map(|s| map(s[0]).distance(map(s[1])))
                    .sum();
                let drawn = n.at(Prop::Progress, self.alpha).clamp(0.0, 1.0) * total;
                let half = stroke.width * 0.5;
                let mut done = 0.0f32;
                for s in stroke.points.windows(2) {
                    if done >= drawn {
                        break;
                    }
                    let (a, mut b) = (map(s[0]), map(s[1]));
                    let length = a.distance(b);
                    if done + length > drawn && length > 0.0 {
                        b = a + (b - a) * ((drawn - done) / length);
                    }
                    done += length;
                    let (lo, hi) = (a.min(b) - half, a.max(b) + half);
                    let q = Quad {
                        rect: [lo.x, lo.y, hi.x - lo.x, hi.y - lo.y],
                        xform: xform(xf),
                        uv: [a.x, a.y, b.x, b.y],
                        fill,
                        params: [half, 0.0, 0.0, 0.0],
                        kind: kind::SEGMENT,
                        ..Quad::default()
                    };
                    self.list.push(q, NO_TEXTURE, scissor);
                }
            }
        }
        let inner = if n.clip {
            intersect(scissor, self.pixels(bounds(xf, w, h)))
        } else {
            scissor
        };
        if inner[2] == 0 || inner[3] == 0 {
            return;
        }
        for &child in &n.children {
            self.node(child, xf, opacity, inner);
        }
    }

    fn rect(&self, xf: Affine2, rect: [f32; 4], n: &Node, fill: [f32; 4]) -> Quad {
        let [w, h] = [rect[2], rect[3]];
        let r = n.radius;
        let fit = |side: f32, a: f32, b: f32| if a + b > side { side / (a + b) } else { 1.0 };
        let k = fit(w, r[0][0], r[1][0])
            .min(fit(w, r[3][0], r[2][0]))
            .min(fit(h, r[0][1], r[3][1]))
            .min(fit(h, r[1][1], r[2][1]));
        Quad {
            rect,
            xform: xform(xf),
            fill,
            radii: r.map(|c| c[0] * k),
            radii_y: r.map(|c| c[1] * k),
            kind: kind::RECT,
            ..Quad::default()
        }
    }

    fn text(
        &mut self,
        xf: Affine2,
        n: &Node,
        t: &Text,
        fill: [f32; 4],
        opacity: f32,
        scissor: [u32; 4],
    ) {
        let s = &self.ui.settings;
        let font = self.ui.font(t.font);
        let size = t.size;
        let k = size / s.sdf_px;
        let to_field = s.sdf_px / size / (2.0 * s.sdf_spread);
        let field_scale = s.atlas_size as f32 / (2.0 * s.sdf_spread);
        let line_px = size * t.line_height;
        let ascent = font.ascender * size;
        let half_leading = (line_px - (font.ascender - font.descender) * size) * 0.5;
        let reveal = n.at(Prop::Reveal, self.alpha);
        let [cx, cy, cw, _] = n.content;
        let atlas = s.atlas_size as f32;
        let outline = t
            .outline
            .filter(|o| o.width > 0.0)
            .map(|o| ((o.width * to_field).min(0.49), faded(o.color.0, opacity)));
        let outline_v = outline.map_or(0.0, |o| o.0);
        let passes = [
            t.shadow.map(|sh| {
                let c = faded(sh.color.0, opacity);
                ([sh.x, sh.y], c, c, (sh.blur * to_field).max(0.0))
            }),
            Some(([0.0, 0.0], fill, outline.map_or(fill, |o| o.1), 0.0)),
        ];
        for (offset, color, stroke, soft) in passes.into_iter().flatten() {
            for (li, line) in t.lines.iter().enumerate() {
                let x0 = cx
                    + match t.align {
                        Align::Left => 0.0,
                        Align::Center => (cw - line.width) * 0.5,
                        Align::Right => cw - line.width,
                    };
                let baseline = cy + li as f32 * line_px + half_leading + ascent;
                let mut pen = x0;
                for g in &t.glyphs[line.start as usize..line.end as usize] {
                    let shown = (reveal - g.char_index as f32).clamp(0.0, 1.0);
                    if let (Some(cell), true) = (g.cell, shown > 0.0) {
                        let [u, v, cw_, ch] = cell.rect.map(|x| x as f32);
                        let q = Quad {
                            rect: [
                                pen + g.offset[0] * size + cell.bearing[0] * k + offset[0],
                                baseline + g.offset[1] * size + cell.bearing[1] * k + offset[1],
                                cw_ * k,
                                ch * k,
                            ],
                            xform: xform(xf),
                            uv: [u / atlas, v / atlas, (u + cw_) / atlas, (v + ch) / atlas],
                            fill: faded(color, shown),
                            stroke: faded(stroke, shown),
                            params: [outline_v, soft, field_scale, 0.0],
                            kind: kind::GLYPH,
                            ..Quad::default()
                        };
                        self.list.push(q, crate::draw::ATLAS_TEXTURE, scissor);
                    }
                    pen += g.advance * size;
                }
            }
        }
    }

    fn image(
        &mut self,
        xf: Affine2,
        w: f32,
        h: f32,
        image: &crate::node::Image,
        fill: [f32; 4],
        scissor: [u32; 4],
    ) {
        let data = &self.ui.images[image.image as usize];
        let (iw, ih) = (data.width as f32, data.height as f32);
        let [rx, ry, rw, rh] = image.region;
        let texture = 1 + image.image;
        let quad = |dst: [f32; 4], src: [f32; 4]| Quad {
            rect: dst,
            xform: xform(xf),
            uv: [src[0] / iw, src[1] / ih, src[2] / iw, src[3] / ih],
            fill,
            kind: kind::IMAGE,
            ..Quad::default()
        };
        let Some([l, t, r, b]) = image.slice else {
            self.list.push(
                quad([0.0, 0.0, w, h], [rx, ry, rx + rw, ry + rh]),
                texture,
                scissor,
            );
            return;
        };
        let s = image.slice_scale;
        let xd = [0.0, l * s, w - r * s, w];
        let yd = [0.0, t * s, h - b * s, h];
        let xs = [rx, rx + l, rx + rw - r, rx + rw];
        let ys = [ry, ry + t, ry + rh - b, ry + rh];
        for j in 0..3 {
            for i in 0..3 {
                let dst = [xd[i], yd[j], xd[i + 1] - xd[i], yd[j + 1] - yd[j]];
                if dst[2] > 0.0 && dst[3] > 0.0 {
                    self.list.push(
                        quad(dst, [xs[i], ys[j], xs[i + 1], ys[j + 1]]),
                        texture,
                        scissor,
                    );
                }
            }
        }
    }

    fn pixels(&self, b: [f32; 4]) -> [u32; 4] {
        let [sw, sh] = [self.screen[2] as f32, self.screen[3] as f32];
        let x0 = b[0].floor().clamp(0.0, sw);
        let y0 = b[1].floor().clamp(0.0, sh);
        let x1 = (b[0] + b[2]).ceil().clamp(0.0, sw);
        let y1 = (b[1] + b[3]).ceil().clamp(0.0, sh);
        [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32]
    }
}

fn xform(xf: Affine2) -> [f32; 6] {
    let m = xf.matrix2;
    [
        m.x_axis.x,
        m.x_axis.y,
        m.y_axis.x,
        m.y_axis.y,
        xf.translation.x,
        xf.translation.y,
    ]
}

fn faded(c: [f32; 4], k: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * k]
}

fn intersect(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
}

fn gradient(q: &mut Quad, g: &Gradient, w: f32, h: f32, opacity: f32) {
    q.fill = faded(g.from.0, opacity);
    q.uv = faded(g.to.0, opacity);
    let [s0, s1] = [g.stops[0], g.stops[1].max(g.stops[0])];
    let (a, b, inner) = match g.kind {
        GradientKind::Linear => {
            q.kind = kind::LINEAR;
            let (sin, cos) = g.angle.to_radians().sin_cos();
            let dir = Vec2::new(sin, -cos);
            let length = (w * sin).abs() + (h * cos).abs();
            let mid = Vec2::new(w, h) * 0.5;
            (
                mid + dir * length * (s0 - 0.5),
                mid + dir * length * (s1 - 0.5),
                0.0,
            )
        }
        GradientKind::Radial => {
            q.kind = kind::RADIAL;
            let c = Vec2::new(g.center[0] * w, g.center[1] * h);
            let radii = match g.radius {
                Some([rx, ry]) => Vec2::new(rx * w, ry * h),
                None => {
                    let corners = [
                        Vec2::ZERO,
                        Vec2::new(w, 0.0),
                        Vec2::new(0.0, h),
                        Vec2::new(w, h),
                    ];
                    Vec2::splat(corners.iter().map(|p| p.distance(c)).fold(0.0, f32::max))
                }
            };
            let inner = if s1 > 0.0 { s0 / s1 } else { 0.0 };
            (c, radii * s1, inner)
        }
    };
    q.params[2] = a.x;
    q.params[3] = a.y;
    q.gradient = [b.x, b.y, inner];
}
