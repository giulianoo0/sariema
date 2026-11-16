// SPDX-License-Identifier: Apache-2.0

use glam::{Affine2, Vec2};
use serde::Deserialize;

use crate::color::Color;
use crate::text::{Align, Line, Shaped};
use crate::widget::Widget;

/// A node of a [`crate::Ui`]. Stays valid until the node is removed; a
/// removed node's id is never confused with a later node's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl NodeId {
    /// The node's slot, stable for its whole life.
    pub fn index(self) -> u32 {
        self.index
    }
}

/// A property animations and [`crate::Ui::set`] change. Positions and sizes
/// are UI units, `Rotation` degrees clockwise, colour channels [0, 1].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prop {
    /// Offset from where layout put the node, like a CSS translate.
    X,
    Y,
    ScaleX,
    ScaleY,
    Rotation,
    /// Multiplies down to every descendant.
    Opacity,
    /// The node's size; NaN leaves it to layout. Changing it lays out again.
    Width,
    Height,
    /// How much of a ring's arc or a stroke's path is drawn, [0, 1].
    Progress,
    /// Characters of a text shown; the next one fades in over its fraction.
    Reveal,
    Red,
    Green,
    Blue,
    Alpha,
}

pub(crate) const PROPS: usize = 14;

impl Prop {
    pub(crate) const ALL: [Prop; PROPS] = [
        Prop::X,
        Prop::Y,
        Prop::ScaleX,
        Prop::ScaleY,
        Prop::Rotation,
        Prop::Opacity,
        Prop::Width,
        Prop::Height,
        Prop::Progress,
        Prop::Reveal,
        Prop::Red,
        Prop::Green,
        Prop::Blue,
        Prop::Alpha,
    ];

    /// Moves or reshapes the node rather than fading or tinting it; reduced
    /// motion sets these at once.
    pub(crate) fn moves(self) -> bool {
        matches!(
            self,
            Prop::X
                | Prop::Y
                | Prop::ScaleX
                | Prop::ScaleY
                | Prop::Rotation
                | Prop::Width
                | Prop::Height
        )
    }

    pub(crate) fn lays_out(self) -> bool {
        matches!(self, Prop::Width | Prop::Height)
    }
}

pub(crate) fn initial_values(fill: Color) -> [f32; PROPS] {
    let [r, g, b, a] = fill.0;
    [
        0.0,
        0.0,
        1.0,
        1.0,
        0.0,
        1.0,
        f32::NAN,
        f32::NAN,
        1.0,
        f32::INFINITY,
        r,
        g,
        b,
        a,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

/// A box shadow or text shadow: offset and blur in UI units.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub blur: f32,
    pub color: Color,
}

pub(crate) struct Text {
    pub text: String,
    pub font: u32,
    pub size: f32,
    pub line_height: f32,
    pub align: Align,
    pub max_lines: u32,
    pub outline: Option<Border>,
    pub shadow: Option<Shadow>,
    pub glyphs: Vec<Shaped>,
    pub lines: Vec<Line>,
    pub chars: u32,
}

pub(crate) struct Image {
    pub image: u32,
    /// Pixels of the image: x, y, width, height.
    pub region: [f32; 4],
    /// Nine-slice insets in image pixels: left, top, right, bottom.
    pub slice: Option<[f32; 4]>,
    /// UI units per image pixel for the slice borders.
    pub slice_scale: f32,
}

pub(crate) struct Ring {
    pub width: f32,
    /// Degrees clockwise from the top where the arc starts.
    pub start: f32,
    pub track: Color,
}

pub(crate) struct Stroke {
    /// Points in `viewbox` coordinates, mapped onto the node's box.
    pub points: Vec<[f32; 2]>,
    pub viewbox: Option<[f32; 2]>,
    pub width: f32,
}

pub(crate) enum Kind {
    Box,
    Text(Box<Text>),
    Image(Image),
    Ring(Ring),
    Stroke(Stroke),
}

pub(crate) struct Node {
    pub generation: u32,
    pub live: bool,
    /// Not laid out yet: its first layout box is also its previous one.
    pub fresh: bool,
    pub taffy: taffy::NodeId,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub name: Option<String>,
    pub kind: Kind,
    pub border: Option<Border>,
    /// Per corner (top-left, top-right, bottom-right, bottom-left): x and y radius.
    pub radius: [[f32; 2]; 4],
    pub shadow: Option<Shadow>,
    /// Pivot of scale and rotation, as a fraction of the node's size.
    pub origin: [f32; 2],
    pub clip: bool,
    pub visible: bool,
    pub interactive: bool,
    pub focus: Option<u32>,
    pub safe: bool,
    pub cur: [f32; PROPS],
    pub prev: [f32; PROPS],
    /// Layout box relative to the parent: x, y, width, height.
    pub rect: [f32; 4],
    pub rect_prev: [f32; 4],
    /// Content box inside the layout box: left, top, width, height.
    pub content: [f32; 4],
    pub widget: Option<Widget>,
}

impl Node {
    pub fn get(&self, p: Prop) -> f32 {
        self.cur[p as usize]
    }

    /// The value between the last two steps; `alpha` 1 is the last.
    pub fn at(&self, p: Prop, alpha: f32) -> f32 {
        lerp(self.prev[p as usize], self.cur[p as usize], alpha)
    }

    /// The transform from this node's units to its parent's, and its size.
    pub fn local(&self, alpha: f32) -> (Affine2, [f32; 2]) {
        let r: [f32; 4] = std::array::from_fn(|k| lerp(self.rect_prev[k], self.rect[k], alpha));
        let v = |p| self.at(p, alpha);
        let origin = Vec2::from(self.origin) * Vec2::new(r[2], r[3]);
        let xf =
            Affine2::from_translation(Vec2::new(r[0] + v(Prop::X), r[1] + v(Prop::Y)) + origin)
                * Affine2::from_angle(v(Prop::Rotation).to_radians())
                * Affine2::from_scale(Vec2::new(v(Prop::ScaleX), v(Prop::ScaleY)))
                * Affine2::from_translation(-origin);
        (xf, [r[2], r[3]])
    }
}

/// Exact at both ends, so infinite values stay put.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    if a == b || t >= 1.0 {
        b
    } else {
        a + (b - a) * t
    }
}
