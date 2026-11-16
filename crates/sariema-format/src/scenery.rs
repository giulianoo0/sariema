// SPDX-License-Identifier: Apache-2.0

//! What a game can change in a room while it plays, indexed by name: its
//! groups, nodes, pieces and lights, and the scene parameters a tween can
//! drive. A game pack carries one [`RoomIndex`] per room, so the step can
//! check and address them without decoding the room itself.

use serde::{Deserialize, Serialize};

use crate::{Hinge, RoomPack};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoomIndex {
    /// Each group's name and whether it is on when the room loads.
    pub groups: Vec<(String, bool)>,
    pub nodes: Vec<NodeIndex>,
    pub pieces: Vec<PieceIndex>,
    pub lights: Vec<LightIndex>,
    /// Every scene parameter of the room with its value when it loads.
    pub params: Vec<Param>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeIndex {
    pub name: String,
    /// Column-major.
    pub pivot: [f32; 16],
    pub visible: bool,
    pub hinge: Option<Hinge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightIndex {
    pub name: String,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PieceIndex {
    pub name: String,
    /// The first is the look before any state, named `""`.
    pub states: Vec<String>,
    pub state: u32,
}

/// A number of the room a tween, a spring or a clip binding drives, named
/// by a path such as `look.exposure` or `light.lamp.intensity`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub value: f32,
    pub target: Target,
}

/// RGB channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    R,
    G,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightField {
    /// Candela.
    Intensity,
    Color(Channel),
    /// Metres.
    Range,
}

/// What a [`Param`] changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Exposure,
    BloomThreshold,
    BloomStrength,
    Tint(Channel),
    Real,
    Night,
    Tilt,
    Focus,
    AoStrength,
    SunIntensity,
    SunColor(Channel),
    /// Degrees from −Z towards +X of where the sun is.
    SunAzimuth,
    /// Degrees above the horizon of where the sun is.
    SunElevation,
    /// Multiplies the ambient light.
    Ambient,
    /// Multiplies one channel of the ambient light.
    AmbientColor(Channel),
    BackgroundTop(Channel),
    BackgroundBottom(Channel),
    /// Fraction of light lost per metre beyond `FogStart`; 0 is no fog.
    FogDensity,
    FogColor(Channel),
    /// Metres from the camera where the fog begins.
    FogStart,
    /// Index into [`RoomIndex::lights`].
    Light(u32, LightField),
    /// Degrees a hinged node is turned from its rest pose; index into
    /// [`RoomIndex::nodes`].
    Hinge(u32),
}

const CHANNELS: [(Channel, &str); 3] = [(Channel::R, "r"), (Channel::G, "g"), (Channel::B, "b")];

/// The azimuth (degrees from −Z towards +X) and elevation (degrees above
/// the horizon) of a direction towards the sun.
pub fn sun_angles(direction: [f32; 3]) -> (f32, f32) {
    let [x, y, z] = direction;
    let azimuth = x.atan2(-z).to_degrees();
    let elevation = y.atan2((x * x + z * z).sqrt()).to_degrees();
    (azimuth, elevation)
}

/// The unit direction towards a sun at `azimuth` and `elevation` degrees;
/// inverts [`sun_angles`].
pub fn sun_direction(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [a.sin() * e.cos(), e.sin(), -a.cos() * e.cos()]
}

impl RoomIndex {
    pub fn of(room: &RoomPack) -> RoomIndex {
        let mut params = Vec::new();
        let mut push = |name: String, value: f32, target: Target| {
            params.push(Param {
                name,
                value,
                target,
            })
        };
        let look = &room.look;
        push("look.exposure".into(), look.exposure, Target::Exposure);
        push(
            "look.bloom_threshold".into(),
            look.bloom_threshold,
            Target::BloomThreshold,
        );
        push(
            "look.bloom_strength".into(),
            look.bloom_strength,
            Target::BloomStrength,
        );
        for (k, (c, n)) in CHANNELS.iter().enumerate() {
            push(format!("look.tint.{n}"), look.tint[k], Target::Tint(*c));
        }
        push("look.real".into(), look.real, Target::Real);
        push("look.night".into(), look.night, Target::Night);
        push("look.tilt".into(), look.tilt, Target::Tilt);
        push("look.focus".into(), look.focus, Target::Focus);
        push(
            "look.ao_strength".into(),
            look.ao_strength,
            Target::AoStrength,
        );
        if let Some(sun) = &room.sun {
            push("sun.intensity".into(), sun.intensity, Target::SunIntensity);
            for (k, (c, n)) in CHANNELS.iter().enumerate() {
                push(format!("sun.color.{n}"), sun.color[k], Target::SunColor(*c));
            }
            let (azimuth, elevation) = sun_angles(sun.direction);
            push("sun.azimuth".into(), azimuth, Target::SunAzimuth);
            push("sun.elevation".into(), elevation, Target::SunElevation);
        }
        push("ambient.intensity".into(), 1.0, Target::Ambient);
        for (c, n) in CHANNELS {
            push(format!("ambient.color.{n}"), 1.0, Target::AmbientColor(c));
        }
        for (k, (c, n)) in CHANNELS.iter().enumerate() {
            let (top, bottom) = (room.background.top[k], room.background.bottom[k]);
            push(
                format!("background.top.{n}"),
                top,
                Target::BackgroundTop(*c),
            );
            push(
                format!("background.bottom.{n}"),
                bottom,
                Target::BackgroundBottom(*c),
            );
        }
        push("fog.density".into(), 0.0, Target::FogDensity);
        for (k, (c, n)) in CHANNELS.iter().enumerate() {
            let color = room.background.bottom[k];
            push(format!("fog.color.{n}"), color, Target::FogColor(*c));
        }
        push("fog.start".into(), 0.0, Target::FogStart);
        for (i, light) in room.lights.iter().enumerate() {
            let i = i as u32;
            let name = &light.name;
            push(
                format!("light.{name}.intensity"),
                light.intensity,
                Target::Light(i, LightField::Intensity),
            );
            for (k, (c, n)) in CHANNELS.iter().enumerate() {
                push(
                    format!("light.{name}.color.{n}"),
                    light.color[k],
                    Target::Light(i, LightField::Color(*c)),
                );
            }
            push(
                format!("light.{name}.range"),
                light.range,
                Target::Light(i, LightField::Range),
            );
        }
        for (i, node) in room.nodes.iter().enumerate() {
            if let Some(hinge) = &node.hinge {
                push(
                    format!("hinge.{}", node.name),
                    hinge.angle_deg,
                    Target::Hinge(i as u32),
                );
            }
        }
        RoomIndex {
            groups: room.groups.iter().map(|g| (g.name.clone(), g.on)).collect(),
            nodes: (room.nodes.iter())
                .map(|n| NodeIndex {
                    name: n.name.clone(),
                    pivot: n.pivot,
                    visible: n.visible,
                    hinge: n.hinge.clone(),
                })
                .collect(),
            pieces: (room.pieces.iter())
                .map(|p| PieceIndex {
                    name: p.name.clone(),
                    states: p.states.iter().map(|s| s.name.clone()).collect(),
                    state: p.state,
                })
                .collect(),
            lights: (room.lights.iter())
                .map(|l| LightIndex {
                    name: l.name.clone(),
                    position: l.position,
                })
                .collect(),
            params,
        }
    }
}
