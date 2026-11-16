// SPDX-License-Identifier: Apache-2.0

//! The JSON description of a screen (`schemas/ui.schema.json`), also the
//! argument of [`crate::Ui::create`].

use std::collections::BTreeMap;

use serde::Deserialize;
use taffy::prelude::{
    AlignContent, AlignItems, Dimension, FlexDirection, FlexWrap, LengthPercentage,
    LengthPercentageAuto, Position, Rect, Size, Style,
};

use crate::anim::{Ease, Spring};
use crate::color::Color;
use crate::node::{Border, Gradient, Prop, Shadow};
use crate::settings::UiSettings;
use crate::text::Align;
use crate::widget::WidgetDesc;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiDesc {
    #[serde(default)]
    pub settings: UiSettings,
    /// Name → font file, relative to the description.
    #[serde(default)]
    pub fonts: BTreeMap<String, FontDesc>,
    /// Name → PNG or JPEG file, relative to the description.
    #[serde(default)]
    pub images: BTreeMap<String, String>,
    /// Children of the screen's root, which covers the whole screen.
    #[serde(default)]
    pub nodes: Vec<NodeDesc>,
    #[serde(default)]
    pub animations: BTreeMap<String, Vec<Track>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontDesc {
    pub file: String,
    /// The `wght` axis of a variable font.
    #[serde(default)]
    pub weight: Option<f32>,
}

/// A length in UI units, a percentage of the parent, or left to layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Units(f32),
    Percent(f32),
    Auto,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LengthRepr {
    Number(f32),
    Text(String),
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Length, D::Error> {
        match LengthRepr::deserialize(d)? {
            LengthRepr::Number(v) => Ok(Length::Units(v)),
            LengthRepr::Text(s) if s == "auto" => Ok(Length::Auto),
            LengthRepr::Text(s) => s
                .strip_suffix('%')
                .and_then(|p| p.parse::<f32>().ok())
                .map(|p| Length::Percent(p / 100.0))
                .ok_or_else(|| {
                    serde::de::Error::custom(format!("{s:?} is not a number, \"N%\" or \"auto\""))
                }),
        }
    }
}

impl Length {
    fn dimension(self) -> Dimension {
        match self {
            Length::Units(v) => Dimension::length(v),
            Length::Percent(p) => Dimension::percent(p),
            Length::Auto => Dimension::auto(),
        }
    }

    fn lpa(self) -> LengthPercentageAuto {
        match self {
            Length::Units(v) => LengthPercentageAuto::length(v),
            Length::Percent(p) => LengthPercentageAuto::percent(p),
            Length::Auto => LengthPercentageAuto::auto(),
        }
    }

    fn lp(self) -> LengthPercentage {
        match self {
            Length::Units(v) => LengthPercentage::length(v),
            Length::Percent(p) => LengthPercentage::percent(p),
            Length::Auto => LengthPercentage::length(0.0),
        }
    }
}

/// One value for every side, `[vertical, horizontal]`, or
/// `[top, right, bottom, left]`, as in CSS.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Sides {
    All(Length),
    Two([Length; 2]),
    Four([Length; 4]),
}

impl Sides {
    /// Top, right, bottom, left.
    fn trbl(self) -> [Length; 4] {
        match self {
            Sides::All(v) => [v; 4],
            Sides::Two([v, h]) => [v, h, v, h],
            Sides::Four(s) => s,
        }
    }
}

/// One gap for both axes, or `[row, column]`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Gap {
    Both(f32),
    Each([f32; 2]),
}

/// One radius for every corner, one per corner (top-left, top-right,
/// bottom-right, bottom-left), or elliptical corners as CSS's `x / y`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Radius {
    All(f32),
    Corners([f32; 4]),
    Elliptical { x: [f32; 4], y: [f32; 4] },
}

impl Radius {
    pub(crate) fn corners(self) -> [[f32; 2]; 4] {
        match self {
            Radius::All(r) => [[r, r]; 4],
            Radius::Corners(c) => c.map(|r| [r, r]),
            Radius::Elliptical { x, y } => std::array::from_fn(|i| [x[i], y[i]]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scale {
    Uniform(f32),
    Each([f32; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    Start,
    End,
    Center,
    Between,
    Around,
    Evenly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cross {
    Start,
    End,
    Center,
    Stretch,
    Baseline,
}

impl Cross {
    fn items(self) -> AlignItems {
        match self {
            Cross::Start => AlignItems::FLEX_START,
            Cross::End => AlignItems::FLEX_END,
            Cross::Center => AlignItems::CENTER,
            Cross::Stretch => AlignItems::STRETCH,
            Cross::Baseline => AlignItems::BASELINE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RingDesc {
    /// Stroke width in UI units.
    pub width: f32,
    /// Degrees clockwise from the top where the arc starts. Default 0.
    #[serde(default)]
    pub start: f32,
    /// Colour of the full circle under the arc. Default transparent.
    #[serde(default = "transparent")]
    pub track: Color,
}

fn transparent() -> Color {
    Color::TRANSPARENT
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrokeDesc {
    /// A polyline drawn with round caps and joins.
    pub points: Vec<[f32; 2]>,
    /// Width and height the points are given in, stretched onto the node's
    /// box. Default: the points are UI units in the node.
    #[serde(default)]
    pub viewbox: Option<[f32; 2]>,
    pub width: f32,
}

/// A node and its children. Layout follows CSS flexbox through taffy;
/// every node is a containing block for its absolutely placed children.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDesc {
    pub name: Option<String>,

    pub width: Option<Length>,
    pub height: Option<Length>,
    pub min_width: Option<Length>,
    pub max_width: Option<Length>,
    pub min_height: Option<Length>,
    pub max_height: Option<Length>,
    /// Main axis of the children. Default row.
    pub direction: Option<Direction>,
    #[serde(default)]
    pub wrap: bool,
    pub justify: Option<Justify>,
    /// Cross-axis placement of the children.
    pub align: Option<Cross>,
    /// Cross-axis placement of this node, over its parent's `align`.
    pub align_self: Option<Cross>,
    #[serde(default)]
    pub grow: f32,
    pub shrink: Option<f32>,
    pub basis: Option<Length>,
    pub gap: Option<Gap>,
    pub padding: Option<Sides>,
    pub margin: Option<Sides>,
    /// Taken out of the flow and placed by `left`, `top`, `right`, `bottom`
    /// inside the parent.
    #[serde(default)]
    pub absolute: bool,
    pub left: Option<Length>,
    pub top: Option<Length>,
    pub right: Option<Length>,
    pub bottom: Option<Length>,
    /// Width over height.
    pub aspect: Option<f32>,
    /// Placed over the screen's safe area: absolute, its insets the
    /// system's. Meant for a child of the root holding the HUD.
    #[serde(default)]
    pub safe: bool,

    pub fill: Option<Color>,
    /// Replaces `fill` on a box.
    pub gradient: Option<Gradient>,
    pub border: Option<Border>,
    pub radius: Option<Radius>,
    pub shadow: Option<Shadow>,

    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    pub scale: Option<Scale>,
    /// Degrees clockwise.
    #[serde(default)]
    pub rotation: f32,
    pub opacity: Option<f32>,
    /// Pivot of scale and rotation as a fraction of the size. Default centre.
    pub origin: Option<[f32; 2]>,
    /// Clips the children to this node's box, axis-aligned on screen.
    #[serde(default)]
    pub clip: bool,
    pub visible: Option<bool>,
    /// Receives presses; the topmost interactive node under a pointer does.
    #[serde(default)]
    pub interactive: bool,
    /// Place in the focus order; lower first.
    pub focus: Option<u32>,

    pub text: Option<String>,
    pub font: Option<String>,
    /// Font size in UI units. Default 16.
    pub size: Option<f32>,
    /// Line box over the font size. Default 1.2.
    pub line_height: Option<f32>,
    pub text_align: Option<Align>,
    pub max_lines: Option<u32>,
    pub outline: Option<Border>,
    pub text_shadow: Option<Shadow>,
    /// Characters shown, for typewriter text. Default all.
    pub reveal: Option<f32>,

    pub image: Option<String>,
    /// Pixels of the image to show: x, y, width, height. Default all.
    pub region: Option<[f32; 4]>,
    /// Nine-slice insets in image pixels: left, top, right, bottom.
    pub slice: Option<[f32; 4]>,
    /// UI units per image pixel of the slice borders. Default 1.
    pub slice_scale: Option<f32>,

    pub ring: Option<RingDesc>,
    pub stroke: Option<StrokeDesc>,
    /// Drawn fraction of a ring or stroke. Default 1.
    pub progress: Option<f32>,

    pub widget: Option<WidgetDesc>,

    #[serde(default)]
    pub children: Vec<NodeDesc>,
}

impl NodeDesc {
    pub(crate) fn style(&self) -> Style {
        let dim = |l: Option<Length>| l.map_or(Dimension::auto(), Length::dimension);
        let lpa = |l: Option<Length>| l.map_or(LengthPercentageAuto::auto(), Length::lpa);
        let [pt, pr, pb, pl] = self
            .padding
            .map_or([Length::Units(0.0); 4], Sides::trbl)
            .map(Length::lp);
        let [mt, mr, mb, ml] = self
            .margin
            .map_or([Length::Units(0.0); 4], Sides::trbl)
            .map(Length::lpa);
        let (row_gap, column_gap) = match self.gap {
            None => (0.0, 0.0),
            Some(Gap::Both(g)) => (g, g),
            Some(Gap::Each([r, c])) => (r, c),
        };
        Style {
            position: if self.absolute || self.safe {
                Position::Absolute
            } else {
                Position::Relative
            },
            size: Size {
                width: dim(self.width),
                height: dim(self.height),
            },
            min_size: Size {
                width: lpa(self.min_width),
                height: lpa(self.min_height),
            },
            max_size: Size {
                width: lpa(self.max_width),
                height: lpa(self.max_height),
            },
            aspect_ratio: self.aspect,
            margin: Rect {
                left: ml,
                right: mr,
                top: mt,
                bottom: mb,
            },
            padding: Rect {
                left: pl,
                right: pr,
                top: pt,
                bottom: pb,
            },
            inset: Rect {
                left: lpa(self.left),
                right: lpa(self.right),
                top: lpa(self.top),
                bottom: lpa(self.bottom),
            },
            flex_direction: match self.direction.unwrap_or(Direction::Row) {
                Direction::Row => FlexDirection::Row,
                Direction::Column => FlexDirection::Column,
                Direction::RowReverse => FlexDirection::RowReverse,
                Direction::ColumnReverse => FlexDirection::ColumnReverse,
            },
            flex_wrap: if self.wrap {
                FlexWrap::Wrap
            } else {
                FlexWrap::NoWrap
            },
            justify_content: match self.justify.unwrap_or(Justify::Start) {
                Justify::Start => AlignContent::FLEX_START,
                Justify::End => AlignContent::FLEX_END,
                Justify::Center => AlignContent::CENTER,
                Justify::Between => AlignContent::SPACE_BETWEEN,
                Justify::Around => AlignContent::SPACE_AROUND,
                Justify::Evenly => AlignContent::SPACE_EVENLY,
            },
            align_items: self.align.map_or(AlignItems::STRETCH, Cross::items),
            align_self: self.align_self.map(Cross::items),
            flex_grow: self.grow,
            flex_shrink: self.shrink.unwrap_or(1.0),
            flex_basis: dim(self.basis),
            gap: Size {
                width: LengthPercentage::length(column_gap),
                height: LengthPercentage::length(row_gap),
            },
            ..Style::default()
        }
    }
}

/// What a track animates: one property or a group set together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    X,
    Y,
    /// X and Y.
    Position,
    ScaleX,
    ScaleY,
    /// Both scales.
    Scale,
    Rotation,
    Opacity,
    Width,
    Height,
    /// Width and height.
    Size,
    Progress,
    Reveal,
    /// The four channels of the fill.
    Fill,
}

impl Target {
    pub(crate) fn props(self) -> &'static [Prop] {
        match self {
            Target::X => &[Prop::X],
            Target::Y => &[Prop::Y],
            Target::Position => &[Prop::X, Prop::Y],
            Target::ScaleX => &[Prop::ScaleX],
            Target::ScaleY => &[Prop::ScaleY],
            Target::Scale => &[Prop::ScaleX, Prop::ScaleY],
            Target::Rotation => &[Prop::Rotation],
            Target::Opacity => &[Prop::Opacity],
            Target::Width => &[Prop::Width],
            Target::Height => &[Prop::Height],
            Target::Size => &[Prop::Width, Prop::Height],
            Target::Progress => &[Prop::Progress],
            Target::Reveal => &[Prop::Reveal],
            Target::Fill => &[Prop::Red, Prop::Green, Prop::Blue, Prop::Alpha],
        }
    }
}

/// A value of a track: a number for every property of the target, one per
/// property, or a colour for `fill`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Number(f32),
    Pair([f32; 2]),
    Color(Color),
}

impl Value {
    pub(crate) fn get(self, i: usize) -> Option<f32> {
        match self {
            Value::Number(v) => Some(v),
            Value::Pair(p) => p.get(i).copied(),
            Value::Color(c) => c.0.get(i).copied(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Typewriter {
    /// Characters per second. Default 30.
    #[serde(default = "thirty")]
    pub rate: f32,
    /// Seconds of pause after `.`, `!`, `?` and `…`. Default 0.07.
    #[serde(default = "pause")]
    pub pause: f32,
}

fn thirty() -> f32 {
    30.0
}

fn pause() -> f32 {
    0.07
}

/// One property change of a named animation, on one node or staggered over
/// several.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub node: Option<String>,
    #[serde(default)]
    pub nodes: Vec<String>,
    /// Seconds between the starts of consecutive `nodes`.
    #[serde(default)]
    pub stagger: f32,
    pub prop: Target,
    /// Start value. Default: the value when the track starts, so an
    /// animation interrupting another continues from where it is.
    pub from: Option<Value>,
    pub to: Option<Value>,
    /// Seconds after the animation starts.
    #[serde(default)]
    pub delay: f32,
    /// Starts when the previous track's last tween ends, plus `delay`.
    #[serde(default)]
    pub after: bool,
    /// Tween length in seconds.
    pub duration: Option<f32>,
    /// Default `out`.
    pub ease: Option<Ease>,
    pub spring: Option<Spring>,
    pub typewriter: Option<Typewriter>,
}
