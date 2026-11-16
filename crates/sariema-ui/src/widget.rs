// SPDX-License-Identifier: Apache-2.0

//! Built-in touch widgets. Each lives on a node that is its touch zone and
//! moves named descendants, so its look is ordinary nodes.

use serde::{Deserialize, Deserializer};

use crate::anim::Spring;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WidgetDesc {
    Joystick(JoystickDesc),
    Button(ButtonDesc),
}

/// A virtual joystick: the base appears under the thumb where it lands, the
/// pin follows the thumb 1:1 up to `radius`, and both spring back on
/// release. Defaults come from the movement research: dead zone 0.15, full
/// speed at 0.7 of the radius.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JoystickDesc {
    /// Node that moves under the thumb; a descendant of the zone.
    pub base: String,
    /// Node that follows the thumb; a descendant of the base.
    pub pin: String,
    /// UI units the pin travels from the base's centre. Default 40.
    pub radius: f32,
    /// Fraction of `radius` under which the output is 0. Default 0.15.
    pub dead_zone: f32,
    /// Fraction of `radius` at which the output reaches 1. Default 0.7.
    pub saturation: f32,
    /// Base opacity at rest. Default 0.45.
    pub rest_opacity: f32,
    /// Base opacity while held. Default 1.
    pub held_opacity: f32,
    /// Seconds the base takes to fade and grow in under the thumb. Default 0.18.
    pub appear_s: f32,
    /// Scale the base grows from as it appears. Default 0.94.
    pub appear_scale: f32,
    /// Spring of the pin back to the centre. Default 0.18 s, damping 1.
    pub pin_return: Spring,
    /// Spring of the base back to rest. Default 0.35 s, damping 1.
    pub base_return: Spring,
}

impl Default for JoystickDesc {
    fn default() -> JoystickDesc {
        JoystickDesc {
            base: String::new(),
            pin: String::new(),
            radius: 40.0,
            dead_zone: 0.15,
            saturation: 0.7,
            rest_opacity: 0.45,
            held_opacity: 1.0,
            appear_s: 0.18,
            appear_scale: 0.94,
            pin_return: Spring {
                response: 0.18,
                damping: 1.0,
            },
            base_return: Spring {
                response: 0.35,
                damping: 1.0,
            },
        }
    }
}

/// An action button: the face shrinks while pressed; with a ring, holding
/// fills it linearly and a full ring is a hold, an earlier release a tap.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ButtonDesc {
    /// Node scaled while pressed. Default: the button's own node.
    pub face: Option<String>,
    /// Rings whose progress fills while held: one name, or several that
    /// fill together (an outline behind the arc). Without one the button
    /// only taps.
    #[serde(deserialize_with = "one_or_many")]
    pub ring: Vec<String>,
    /// Seconds of holding that fill the ring from empty. Default 1.2.
    pub hold_s: f32,
    /// Seconds a full ring takes to empty after an early release. Default 0.2.
    pub empty_s: f32,
    /// Face scale while pressed. Default 0.92.
    pub press_scale: f32,
    /// Seconds the face takes to reach it and to come back. Default 0.12.
    pub press_s: f32,
}

impl Default for ButtonDesc {
    fn default() -> ButtonDesc {
        ButtonDesc {
            face: None,
            ring: Vec::new(),
            hold_s: 1.2,
            empty_s: 0.2,
            press_scale: 0.92,
            press_s: 0.12,
        }
    }
}

pub(crate) enum Widget {
    Joystick(Joystick),
    Button(Button),
}

pub(crate) struct Joystick {
    pub desc: JoystickDesc,
    pub base: u32,
    pub pin: u32,
    pub pointer: Option<u64>,
    /// Where the base's centre sits, in the zone's units.
    pub center: [f32; 2],
    pub output: [f32; 2],
}

pub(crate) struct Button {
    pub desc: ButtonDesc,
    pub face: u32,
    pub rings: Vec<u32>,
    pub pointer: Option<u64>,
    pub held: bool,
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(name) => vec![name],
        OneOrMany::Many(names) => names,
    })
}

/// The joystick's output for a pin offset `d`: 0 within the dead zone, then
/// growing linearly to length 1 at the saturation radius, in `d`'s direction.
pub fn stick_curve(d: [f32; 2], radius: f32, dead_zone: f32, saturation: f32) -> [f32; 2] {
    let length = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let m = (length / radius).min(1.0);
    if m <= dead_zone || length == 0.0 {
        return [0.0, 0.0];
    }
    let k = ((m - dead_zone) / (saturation - dead_zone).max(f32::EPSILON)).min(1.0);
    [d[0] / length * k, d[1] / length * k]
}
