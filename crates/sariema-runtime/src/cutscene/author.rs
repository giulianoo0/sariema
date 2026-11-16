// SPDX-License-Identifier: Apache-2.0

//! Cutscene JSON (`schemas/cutscene.schema.json`) and its compilation into
//! the pack's [`Cutscene`]. Names become ids; roles become indices.

use std::collections::BTreeMap;

use sariema_format::script::cutscene::{
    Await, Block, Cover, Cutscene, Place, Point, Role, Shot, Toward,
};
use sariema_format::script::{Advance, Anchor, Bus, ClipMode, Ease, name_id};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub name: String,
    #[serde(default = "yes")]
    pub skippable: bool,
    pub roles: BTreeMap<String, RoleSource>,
    pub steps: Vec<Step>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSource {
    #[serde(default)]
    pub start: Option<PlaceSource>,
    /// Actor asset a `spawn` of this role creates; defaults to the role name.
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceSource {
    pub position: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Sequence(Vec<Step>),
    Parallel(Vec<Vec<Step>>),
    Wait(f32),
    WaitFor(WaitFor),
    Branch(Branch),
    Shot(ShotSource),
    Walk(Walk),
    Clip(Clip),
    Face(Face),
    LookAt(LookAt),
    Line(Line),
    Sound(Sound),
    Music(Music),
    Fade(Fade),
    Letterbox(Letterbox),
    SetFlag(String),
    ClearFlag(String),
    Spawn(Spawn),
    Despawn(String),
    Show(String),
    Hide(String),
    Teleport(Teleport),
    Call(String),
    TimeScale(TimeScale),
    CameraShake(CameraShake),
    Hinge(HingeSource),
}

/// A hinged node turned with an actor's clip; see [`Block::Hinge`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HingeSource {
    /// The node, as the room names it: `<room>.<piece>_<hinge>`.
    pub node: String,
    pub role: String,
    pub clip: String,
    /// Seconds of the clip the turn starts and ends at.
    pub span: [f32; 2],
    /// Degrees from closed.
    pub to: f32,
    #[serde(default = "HingeSource::ease")]
    pub ease: String,
}

impl HingeSource {
    fn ease() -> String {
        "in_out_sine".into()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitFor {
    pub signal: Option<String>,
    pub arrived: Option<String>,
    pub clip_finished: Option<String>,
    pub clip_marker: Option<ClipMarker>,
    pub timeout: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipMarker {
    pub role: String,
    pub clip: String,
    pub marker: String,
}

/// A bone or socket name per subject: one for `follow`, a pair (each may
/// be null) for `frame`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Points {
    One(String),
    Two([Option<String>; 2]),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    pub flag: String,
    pub then: Vec<Step>,
    #[serde(rename = "else", default)]
    pub otherwise: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShotSource {
    pub follow: Option<String>,
    pub frame: Option<[String; 2]>,
    pub fixed: Option<Fixed>,
    pub bone: Option<Points>,
    pub socket: Option<Points>,
    #[serde(default = "ShotSource::distance")]
    pub distance: f32,
    #[serde(default = "ShotSource::pitch")]
    pub pitch: f32,
    #[serde(default)]
    pub yaw: f32,
    #[serde(default = "ShotSource::height")]
    pub height: f32,
    #[serde(default = "ShotSource::fov")]
    pub fov: f32,
    #[serde(default = "ShotSource::margin")]
    pub margin: f32,
    /// Seconds; 0 is a cut. Required, so every cut is written down.
    pub blend: f32,
    #[serde(default = "ShotSource::ease")]
    pub ease: String,
}

impl ShotSource {
    fn distance() -> f32 {
        5.0
    }
    fn pitch() -> f32 {
        30.0
    }
    fn height() -> f32 {
        1.0
    }
    fn fov() -> f32 {
        45.0
    }
    fn margin() -> f32 {
        1.0
    }
    fn ease() -> String {
        "in_out_cubic".into()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixed {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Walk {
    pub role: String,
    pub to: [f32; 3],
    #[serde(default = "Walk::speed")]
    pub speed: f32,
    pub face: Option<f32>,
}

impl Walk {
    fn speed() -> f32 {
        1.4
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub role: String,
    pub clip: String,
    #[serde(default = "Clip::mode")]
    pub mode: String,
    #[serde(default = "Clip::blend")]
    pub blend: f32,
    #[serde(default = "Clip::speed")]
    pub speed: f32,
}

impl Clip {
    fn mode() -> String {
        "once".into()
    }
    fn blend() -> f32 {
        0.2
    }
    fn speed() -> f32 {
        1.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeScale {
    pub scale: f32,
    #[serde(default)]
    pub blend: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraShake {
    pub amplitude: f32,
    #[serde(default = "CameraShake::frequency")]
    pub frequency: f32,
    pub duration: f32,
}

impl CameraShake {
    fn frequency() -> f32 {
        12.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Face {
    pub role: String,
    pub toward: Option<String>,
    pub yaw: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookAt {
    pub role: String,
    pub target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub speaker: String,
    pub text: String,
    pub voice: Option<String>,
    #[serde(default = "Line::advance")]
    pub advance: String,
    /// Seconds an `auto` line stays; required for `auto`.
    pub duration: Option<f32>,
}

impl Line {
    fn advance() -> String {
        "tap".into()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    pub sound: String,
    #[serde(default = "Sound::bus")]
    pub bus: String,
    pub at: Option<String>,
    #[serde(default = "Sound::volume")]
    pub volume: f32,
}

impl Sound {
    fn bus() -> String {
        "effects".into()
    }
    fn volume() -> f32 {
        1.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Music {
    /// Absent stops the music.
    pub music: Option<String>,
    #[serde(default = "Music::fade")]
    pub fade: f32,
}

impl Music {
    fn fade() -> f32 {
        1.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fade {
    pub to: f32,
    pub duration: f32,
    #[serde(default)]
    pub color: [f32; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Letterbox {
    pub to: f32,
    pub duration: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub role: String,
    pub position: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Teleport {
    pub role: String,
    pub position: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    pub cover: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorError {
    pub code: &'static str,
    pub cutscene: String,
    /// Where in the steps, like `steps[2].parallel[1][0]`.
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for AuthorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cutscene {}, {}: {}",
            self.cutscene, self.at, self.message
        )
    }
}

struct Compiler<'a> {
    cutscene: &'a str,
    roles: Vec<&'a str>,
    names: Vec<String>,
}

impl Compiler<'_> {
    fn error(&self, code: &'static str, at: &str, message: String) -> AuthorError {
        AuthorError {
            code,
            cutscene: self.cutscene.into(),
            at: at.into(),
            message,
        }
    }

    fn name(&mut self, name: &str) -> u32 {
        if !self.names.iter().any(|n| n == name) {
            self.names.push(name.into());
        }
        name_id(name)
    }

    fn role(&self, name: &str, at: &str) -> Result<u16, AuthorError> {
        self.roles
            .iter()
            .position(|r| *r == name)
            .map(|i| i as u16)
            .ok_or_else(|| {
                self.error(
                    "E_CUTSCENE_ROLE",
                    at,
                    format!(
                        "unknown role {name:?}; the roles are {}",
                        self.roles.join(", ")
                    ),
                )
            })
    }

    fn pick<T>(
        &self,
        at: &str,
        what: &str,
        parse: impl Fn(&str) -> Option<T>,
        name: &str,
    ) -> Result<T, AuthorError> {
        parse(name).ok_or_else(|| self.error("E_SCHEMA", at, format!("unknown {what} {name:?}")))
    }

    fn list(&mut self, steps: &[Step], at: &str) -> Result<Vec<Block>, AuthorError> {
        steps
            .iter()
            .enumerate()
            .map(|(i, s)| self.step(s, &format!("{at}[{i}]")))
            .collect()
    }

    fn step(&mut self, step: &Step, at: &str) -> Result<Block, AuthorError> {
        let place = |p: [f32; 3], yaw: f32| Place { position: p, yaw };
        Ok(match step {
            Step::Sequence(steps) => Block::Sequence(self.list(steps, &format!("{at}.sequence"))?),
            Step::Parallel(branches) => Block::Parallel(
                branches
                    .iter()
                    .enumerate()
                    .map(|(b, steps)| self.list(steps, &format!("{at}.parallel[{b}]")))
                    .collect::<Result<_, _>>()?,
            ),
            Step::Wait(t) => Block::Wait(*t),
            Step::WaitFor(w) => {
                let at = format!("{at}.wait_for");
                let on = match (&w.signal, &w.arrived, &w.clip_finished, &w.clip_marker) {
                    (Some(s), None, None, None) => Await::Signal(self.name(s)),
                    (None, Some(r), None, None) => Await::Arrived(self.role(r, &at)?),
                    (None, None, Some(r), None) => Await::ClipFinished(self.role(r, &at)?),
                    (None, None, None, Some(m)) => Await::ClipMarker {
                        role: self.role(&m.role, &at)?,
                        clip: self.name(&m.clip),
                        marker: self.name(&m.marker),
                    },
                    _ => {
                        return Err(self.error(
                            "E_SCHEMA",
                            &at,
                            "name exactly one of signal, arrived, clip_finished and clip_marker"
                                .into(),
                        ));
                    }
                };
                Block::WaitFor {
                    on,
                    timeout: w.timeout,
                }
            }
            Step::Branch(b) => Block::Branch {
                flag: self.name(&b.flag),
                then: self.list(&b.then, &format!("{at}.branch.then"))?,
                otherwise: self.list(&b.otherwise, &format!("{at}.branch.else"))?,
            },
            Step::Shot(s) => {
                let at = format!("{at}.shot");
                let shot = match (&s.follow, &s.frame, &s.fixed) {
                    (Some(r), None, None) => Shot::Follow {
                        subject: self.role(r, &at)?,
                        distance: s.distance,
                        pitch: s.pitch,
                        yaw: s.yaw,
                        height: s.height,
                        fov: s.fov,
                        point: self.points(s, 1, &at)?[0],
                    },
                    (None, Some([a, b]), None) => Shot::Frame {
                        a: self.role(a, &at)?,
                        b: self.role(b, &at)?,
                        pitch: s.pitch,
                        yaw: s.yaw,
                        height: s.height,
                        fov: s.fov,
                        margin: s.margin,
                        points: self.points(s, 2, &at)?,
                    },
                    (None, None, Some(_)) if s.bone.is_some() || s.socket.is_some() => {
                        return Err(self.error(
                            "E_SCHEMA",
                            &at,
                            "a fixed shot has no subject to take a bone or socket of".into(),
                        ));
                    }
                    (None, None, Some(f)) => Shot::Fixed {
                        position: f.position,
                        look_at: f.look_at,
                        fov: s.fov,
                    },
                    _ => {
                        return Err(self.error(
                            "E_SCHEMA",
                            &at,
                            "name exactly one of follow, frame and fixed".into(),
                        ));
                    }
                };
                Block::Shot {
                    shot,
                    blend: s.blend,
                    ease: self.pick(&at, "ease", Ease::from_name, &s.ease)? as u8,
                }
            }
            Step::Walk(w) => Block::Walk {
                role: self.role(&w.role, &format!("{at}.walk"))?,
                to: w.to,
                speed: w.speed,
                face: w.face,
            },
            Step::Clip(c) => {
                let at = format!("{at}.clip");
                Block::Clip {
                    role: self.role(&c.role, &at)?,
                    clip: self.name(&c.clip),
                    mode: self.pick(&at, "clip mode", ClipMode::from_name, &c.mode)? as u8,
                    blend: c.blend,
                    speed: c.speed,
                }
            }
            Step::Face(f) => {
                let at = format!("{at}.face");
                let toward = match (&f.toward, f.yaw) {
                    (Some(r), None) => Toward::Role(self.role(r, &at)?),
                    (None, Some(y)) => Toward::Yaw(y),
                    _ => {
                        return Err(self.error(
                            "E_SCHEMA",
                            &at,
                            "name exactly one of toward and yaw".into(),
                        ));
                    }
                };
                Block::Face {
                    role: self.role(&f.role, &at)?,
                    toward,
                }
            }
            Step::LookAt(l) => {
                let at = format!("{at}.look_at");
                Block::LookAt {
                    role: self.role(&l.role, &at)?,
                    target: l.target.as_deref().map(|t| self.role(t, &at)).transpose()?,
                }
            }
            Step::Line(l) => {
                let at = format!("{at}.line");
                let advance = self.pick(&at, "advance", Advance::from_name, &l.advance)?;
                let duration = match (advance, l.duration) {
                    (Advance::Auto, Some(d)) => d,
                    (Advance::Auto, None) => {
                        return Err(self.error(
                            "E_SCHEMA",
                            &at,
                            "an auto line needs a duration".into(),
                        ));
                    }
                    (Advance::Tap, d) => d.unwrap_or(0.0),
                };
                Block::Line {
                    speaker: self.role(&l.speaker, &at)?,
                    text: l.text.clone(),
                    voice: l.voice.as_deref().map_or(0, |v| self.name(v)),
                    advance: advance as u8,
                    duration,
                }
            }
            Step::Sound(s) => {
                let at = format!("{at}.sound");
                Block::Sound {
                    sound: self.name(&s.sound),
                    bus: self.pick(&at, "bus", Bus::from_name, &s.bus)? as u8,
                    at: s.at.as_deref().map(|r| self.role(r, &at)).transpose()?,
                    volume: s.volume,
                }
            }
            Step::Music(m) => Block::Music {
                music: m.music.as_deref().map(|n| self.name(n)),
                fade: m.fade,
            },
            Step::Fade(f) => Block::Fade {
                to: f.to,
                duration: f.duration,
                color: f.color,
            },
            Step::Letterbox(l) => Block::Letterbox {
                to: l.to,
                duration: l.duration,
            },
            Step::SetFlag(f) | Step::ClearFlag(f) => Block::SetFlag {
                flag: self.name(f),
                on: matches!(step, Step::SetFlag(_)),
            },
            Step::Spawn(s) => Block::Spawn {
                role: self.role(&s.role, &format!("{at}.spawn"))?,
                at: place(s.position, s.yaw),
            },
            Step::Despawn(r) => Block::Despawn {
                role: self.role(r, &format!("{at}.despawn"))?,
            },
            Step::Show(r) => Block::Show {
                role: self.role(r, &format!("{at}.show"))?,
            },
            Step::Hide(r) => Block::Hide {
                role: self.role(r, &format!("{at}.hide"))?,
            },
            Step::Teleport(t) => {
                let at = format!("{at}.teleport");
                let cover = match t.cover.as_deref() {
                    None => None,
                    Some("cut") => Some(Cover::Cut),
                    Some("fade") => Some(Cover::Fade),
                    Some(other) => {
                        return Err(self.error(
                            "E_SCHEMA",
                            &at,
                            format!("unknown cover {other:?}"),
                        ));
                    }
                };
                Block::Teleport {
                    role: self.role(&t.role, &at)?,
                    to: place(t.position, t.yaw),
                    cover,
                }
            }
            Step::Call(c) => Block::Call { call: self.name(c) },
            Step::TimeScale(t) => Block::TimeScale {
                scale: t.scale,
                blend: t.blend,
            },
            Step::CameraShake(c) => Block::CameraShake {
                amplitude: c.amplitude,
                frequency: c.frequency,
                duration: c.duration,
            },
            Step::Hinge(h) => {
                let at = format!("{at}.hinge");
                let [start, end] = h.span;
                if !(start >= 0.0 && end > start && end.is_finite()) {
                    return Err(self.error(
                        "E_SCHEMA",
                        &at,
                        format!(
                            "span {:?} must start at 0 s or later and end after it starts",
                            h.span
                        ),
                    ));
                }
                Block::Hinge {
                    hinge: self.name(&format!("hinge.{}", h.node)),
                    role: self.role(&h.role, &at)?,
                    clip: self.name(&h.clip),
                    span: h.span,
                    to: h.to,
                    ease: self.pick(&at, "ease", Ease::from_name, &h.ease)? as u8,
                }
            }
        })
    }

    /// The bone or socket of each of a shot's `subjects`.
    fn points(
        &mut self,
        s: &ShotSource,
        subjects: usize,
        at: &str,
    ) -> Result<[Point; 2], AuthorError> {
        let mut points = [Point::ORIGIN; 2];
        for (anchor, names) in [(Anchor::Bone, &s.bone), (Anchor::Socket, &s.socket)] {
            let names: [Option<&str>; 2] = match (names, subjects) {
                (None, _) => continue,
                (Some(Points::One(n)), 1) => [Some(n), None],
                (Some(Points::Two([a, b])), 2) => [a.as_deref(), b.as_deref()],
                _ => {
                    return Err(self.error(
                        "E_SCHEMA",
                        at,
                        format!(
                            "{} takes one name per subject: a name for follow, a pair for frame",
                            anchor.name()
                        ),
                    ));
                }
            };
            for (point, name) in points.iter_mut().zip(names) {
                let Some(name) = name else { continue };
                if *point != Point::ORIGIN {
                    return Err(self.error(
                        "E_SCHEMA",
                        at,
                        "a subject takes a bone or a socket, not both".into(),
                    ));
                }
                *point = Point {
                    anchor: anchor as u8,
                    name: self.name(name),
                };
            }
        }
        Ok(points)
    }
}

/// Compiles one cutscene; also returns every name it uses, for the pack.
pub fn compile(source: &Source) -> Result<(Cutscene, Vec<String>), AuthorError> {
    let mut c = Compiler {
        cutscene: &source.name,
        roles: source.roles.keys().map(String::as_str).collect(),
        names: Vec::new(),
    };
    c.name(&source.name);
    let mut roles = Vec::new();
    for (name, role) in &source.roles {
        c.name(name);
        let actor = role.actor.as_deref().unwrap_or(name);
        roles.push(Role {
            name: name.clone(),
            start: role.start.map(|p| Place {
                position: p.position,
                yaw: p.yaw,
            }),
            actor: c.name(actor),
        });
    }
    let body = c.list(&source.steps, "steps")?;
    Ok((
        Cutscene {
            name: source.name.clone(),
            skippable: source.skippable,
            roles,
            body,
        },
        c.names,
    ))
}

/// Parses and compiles cutscene JSON.
pub fn compile_json(text: &str) -> Result<(Cutscene, Vec<String>), AuthorError> {
    let source: Source = serde_json::from_str(text).map_err(|e| AuthorError {
        code: "E_SCHEMA",
        cutscene: String::new(),
        at: format!("line {} column {}", e.line(), e.column()),
        message: e.to_string(),
    })?;
    compile(&source)
}
