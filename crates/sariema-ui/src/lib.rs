// SPDX-License-Identifier: Apache-2.0

//! The Sariema UI: a retained tree of boxes, text, images, rings and strokes
//! laid out with flexbox, animated on the fixed simulation step, driven by
//! multi-touch, and turned into a batched list of quads for
//! `sariema_render::ui`.
//!
//! A frame is [`Ui::step`] once per simulation step (input and animation),
//! then [`Ui::draw`] with the render's interpolation factor. A screen that
//! does not change allocates nothing in either.

mod anim;
mod atlas;
mod color;
mod desc;
mod draw;
mod input;
mod node;
mod settings;
mod text;
mod ui;
mod widget;

use std::fmt;

pub use anim::{Ease, Motion, Spring};
pub use color::Color;
pub use desc::{
    Cross, Direction, FontDesc, Gap, Justify, Length, NodeDesc, Radius, RingDesc, Scale, Sides,
    StrokeDesc, Target, Track, Typewriter, UiDesc, Value,
};
pub use draw::{ATLAS_TEXTURE, Batch, DrawList, NO_TEXTURE, Quad, kind};
pub use input::Event;
pub use node::{Border, Gradient, GradientKind, NodeId, Prop, Shadow};
pub use settings::{SafeArea, UiSettings};
pub use text::{Align, Line};
pub use ui::{Animation, ImageData, Screen, Ui};
pub use widget::{ButtonDesc, JoystickDesc, WidgetDesc, stick_curve};

#[derive(Clone, Debug, PartialEq)]
pub enum UiError {
    Io(String),
    /// A description, node or setting that does not hold together.
    Schema(String),
    Font(String),
    Image(String),
    /// A node, font, image or animation that does not exist.
    Unknown(String),
    /// The glyph atlas has no room for a glyph; raise `atlas_size`.
    AtlasFull,
    /// More animations than `UiSettings::animations`.
    Animations(usize),
}

impl UiError {
    pub fn code(&self) -> &'static str {
        match self {
            UiError::Io(_) => "E_IO",
            UiError::Schema(_) => "E_SCHEMA",
            UiError::Font(_) => "E_UI_FONT",
            UiError::Image(_) => "E_UI_IMAGE",
            UiError::Unknown(_) => "E_UI_UNKNOWN",
            UiError::AtlasFull => "E_UI_ATLAS_FULL",
            UiError::Animations(_) => "E_UI_ANIMATIONS",
        }
    }
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::Io(why) | UiError::Schema(why) => f.write_str(why),
            UiError::Font(why) => write!(f, "font {why}"),
            UiError::Image(why) => write!(f, "image {why}"),
            UiError::Unknown(what) => write!(f, "no {what}"),
            UiError::AtlasFull => write!(f, "the glyph atlas is full; raise atlas_size"),
            UiError::Animations(n) => {
                write!(f, "more than {n} animations at once; raise animations")
            }
        }
    }
}

impl std::error::Error for UiError {}
