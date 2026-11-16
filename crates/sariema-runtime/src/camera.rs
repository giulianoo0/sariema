// SPDX-License-Identifier: Apache-2.0

//! Camera rigs and the switch between them. A rig computes a pose each step
//! (follow a subject, frame two, or stay fixed); switching blends from the
//! last pose shown to the new rig's live pose, so position and angle stay
//! continuous. A blend of 0 is a cut, the only discontinuity.

use std::collections::BTreeMap;
use std::f32::consts::TAU;

use glam::{EulerRot, Quat, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Anchor, Ease};
use serde::{Deserialize, Serialize};

use crate::ease::ease;
use crate::motion::MotionSettings;
use crate::rng::Pcg32;
use crate::spring::Spring;
use crate::tween::steps_of;
use crate::world::{Entity, World};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub position: Vec3,
    pub rotation: Quat,
    /// Vertical field of view in degrees.
    pub fov: f32,
}

impl CameraPose {
    /// Rotation from yaw (about +Y, 0 looks along −Z) and pitch (positive looks up), degrees.
    pub fn rotation_of(yaw: f32, pitch: f32) -> Quat {
        Quat::from_euler(EulerRot::YXZ, yaw.to_radians(), pitch.to_radians(), 0.0)
    }

    pub fn looking(position: Vec3, at: Vec3, fov: f32) -> CameraPose {
        let f = (at - position).normalize_or(Vec3::NEG_Z);
        let yaw = (-f.x).atan2(-f.z).to_degrees();
        let pitch = f.y.clamp(-1.0, 1.0).asin().to_degrees();
        CameraPose {
            position,
            rotation: CameraPose::rotation_of(yaw, pitch),
            fov,
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    /// Degrees about +Y of the view direction.
    pub fn yaw(&self) -> f32 {
        let f = self.forward();
        (-f.x).atan2(-f.z).to_degrees()
    }

    pub fn lerp(&self, other: &CameraPose, t: f32) -> CameraPose {
        CameraPose {
            position: self.position.lerp(other.position, t),
            rotation: self.rotation.slerp(other.rotation, t),
            fov: self.fov + (other.fov - self.fov) * t,
        }
    }
}

/// Follow tuning a rig gets until `rig_tuning` changes it, and how the
/// camera blends when the rig in use is reframed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CameraSettings {
    /// Metres the subject moves before the focus starts to follow.
    pub dead_zone: f32,
    /// Hz of the spring that carries the focus.
    pub frequency: f32,
    pub damping: f32,
    /// Seconds of subject velocity the focus leads by.
    pub look_ahead: f32,
    pub reframe_blend: f32,
    #[serde(
        serialize_with = "crate::settings::ease_out",
        deserialize_with = "crate::settings::ease_in"
    )]
    pub reframe_ease: Ease,
}

impl Default for CameraSettings {
    fn default() -> CameraSettings {
        CameraSettings {
            dead_zone: 0.25,
            frequency: 1.5,
            damping: 1.0,
            look_ahead: 0.0,
            reframe_blend: 0.5,
            reframe_ease: Ease::InOutCubic,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Framing {
    Follow {
        subject: Entity,
        distance: f32,
        pitch: f32,
        yaw: f32,
        height: f32,
        fov: f32,
    },
    Frame {
        a: Entity,
        b: Entity,
        pitch: f32,
        yaw: f32,
        height: f32,
        fov: f32,
        margin: f32,
    },
    Fixed(CameraPose),
}

#[derive(Clone, Copy, Debug)]
struct Rig {
    framing: Framing,
    tuning: CameraSettings,
    spring: Spring,
    bounds: Option<(Vec3, Vec3)>,
    /// Where the dead zone lets the focus go.
    goal: Vec3,
    focus: Vec3,
    velocity: Vec3,
    distance: f32,
    distance_velocity: f32,
    last_subject: Vec3,
    ready: bool,
}

impl Rig {
    fn new(framing: Framing, tuning: CameraSettings) -> Rig {
        Rig {
            framing,
            tuning,
            spring: Spring::new(tuning.frequency, tuning.damping, STEP_SECONDS),
            bounds: None,
            goal: Vec3::ZERO,
            focus: Vec3::ZERO,
            velocity: Vec3::ZERO,
            distance: 0.0,
            distance_velocity: 0.0,
            last_subject: Vec3::ZERO,
            ready: false,
        }
    }

    fn clamp(&self, p: Vec3) -> Vec3 {
        self.bounds.map_or(p, |(lo, hi)| p.clamp(lo, hi))
    }

    /// The subject point and the distance the framing wants, or `None` when
    /// a subject is gone.
    fn wanted(&self, world: &World, points: &[(Entity, Vec3)]) -> Option<(Vec3, f32)> {
        let at = |e: Entity| {
            let pose = world.pose(e)?;
            let local = points.iter().find(|(p, _)| *p == e).map(|&(_, at)| at);
            Some(pose.position + local.map_or(Vec3::ZERO, |at| pose.rotation * (at * pose.scale)))
        };
        match self.framing {
            Framing::Follow {
                subject,
                distance,
                height,
                ..
            } => Some((at(subject)? + Vec3::Y * height, distance)),
            Framing::Frame {
                a,
                b,
                height,
                fov,
                margin,
                ..
            } => {
                let (pa, pb) = (at(a)?, at(b)?);
                let half = pa.distance(pb) / 2.0 + margin;
                let distance = half / (fov.to_radians() / 2.0).tan().max(1e-3);
                Some(((pa + pb) / 2.0 + Vec3::Y * height, distance))
            }
            Framing::Fixed(_) => None,
        }
    }

    fn step(&mut self, world: &World, points: &[(Entity, Vec3)]) -> Option<CameraPose> {
        let (yaw, pitch, fov) = match self.framing {
            Framing::Fixed(pose) => return Some(pose),
            Framing::Follow {
                yaw, pitch, fov, ..
            }
            | Framing::Frame {
                yaw, pitch, fov, ..
            } => (yaw, pitch, fov),
        };
        let (subject, distance) = self.wanted(world, points)?;
        if !self.ready {
            self.last_subject = subject;
            self.goal = self.clamp(subject);
            self.focus = self.goal;
            self.velocity = Vec3::ZERO;
            self.distance = distance;
            self.distance_velocity = 0.0;
            self.ready = true;
        }
        let lead = (subject - self.last_subject) / STEP_SECONDS * self.tuning.look_ahead;
        self.last_subject = subject;
        let desired = subject + lead;
        let mut gap = desired - self.goal;
        gap.y = 0.0;
        let len = gap.length();
        if len > self.tuning.dead_zone {
            self.goal += gap * (1.0 - self.tuning.dead_zone / len);
        }
        self.goal.y = desired.y;
        self.goal = self.clamp(self.goal);
        let before = self.focus;
        for k in 0..3 {
            (self.focus[k], self.velocity[k]) =
                self.spring
                    .step(self.focus[k], self.velocity[k], self.goal[k]);
        }
        // Stops the focus at a bound it reaches. A focus already outside
        // (the bounds just changed) springs back in instead of jumping.
        if let Some((lo, hi)) = self.bounds {
            for k in 0..3 {
                let was_inside = (lo[k]..=hi[k]).contains(&before[k]);
                if was_inside && !(lo[k]..=hi[k]).contains(&self.focus[k]) {
                    self.focus[k] = self.focus[k].clamp(lo[k], hi[k]);
                    self.velocity[k] = 0.0;
                }
            }
        }
        (self.distance, self.distance_velocity) =
            self.spring
                .step(self.distance, self.distance_velocity, distance);
        let rotation = CameraPose::rotation_of(yaw, -pitch);
        Some(CameraPose {
            position: self.focus - rotation * Vec3::NEG_Z * self.distance,
            rotation,
            fov,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Blend {
    from: CameraPose,
    steps: u32,
    elapsed: u32,
    ease: Ease,
}

#[derive(Clone, Copy, Debug)]
struct Shake {
    amplitude: f32,
    frequency: f32,
    steps: u32,
    elapsed: u32,
    phase: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraError {
    UnknownRig,
}

impl CameraError {
    pub const fn code(self) -> &'static str {
        "E_RIG_UNKNOWN"
    }
}

pub struct Camera {
    settings: CameraSettings,
    /// How far and how much a reframing may move the view in one step and
    /// still be shown as is: `motion.max_visible_speed` and `turn_rate`.
    step_move: f32,
    step_turn: f32,
    rigs: BTreeMap<u32, Rig>,
    active: Option<u32>,
    blend: Option<Blend>,
    shake: Option<Shake>,
    pose: Option<CameraPose>,
    previous: Option<CameraPose>,
    cut: bool,
    /// `(rig, subject, anchor, name)`: where in its subject a rig aims.
    anchors: Vec<(u32, Entity, Anchor, u32)>,
    /// This step's resolved anchors: a subject and the point aimed at, in
    /// its own frame (model space, before its placement and scale).
    points: Vec<(Entity, Vec3)>,
}

impl Camera {
    pub fn new(settings: CameraSettings, motion: &MotionSettings) -> Camera {
        Camera {
            settings,
            step_move: motion.max_visible_speed * STEP_SECONDS,
            step_turn: motion.turn_rate * STEP_SECONDS,
            rigs: BTreeMap::new(),
            active: None,
            blend: None,
            shake: None,
            pose: None,
            previous: None,
            cut: false,
            anchors: Vec::new(),
            points: Vec::new(),
        }
    }

    /// Where an anchor of [`Camera::anchors`] is this step, in `subject`'s
    /// own frame; the rig aims there instead of at its origin. Given each
    /// step by the animation module, before the camera steps.
    pub fn set_anchor_point(&mut self, subject: Entity, at: Vec3) {
        match self.points.iter_mut().find(|(e, _)| *e == subject) {
            Some(point) => point.1 = at,
            None => self.points.push((subject, at)),
        }
    }

    /// Creates or reframes a rig, keeping its tuning and bounds. A sudden
    /// reframing of the rig in use blends from the pose shown; a gradual
    /// one, like a camera turned by a stick, is shown as is.
    pub fn set_framing(&mut self, rig: u32, framing: Framing) {
        let tuning = self.settings;
        let r = self
            .rigs
            .entry(rig)
            .or_insert_with(|| Rig::new(framing, tuning));
        let was = r.framing;
        r.framing = framing;
        if self.active == Some(rig) && !self.gradual(&was, &framing) {
            self.transition(tuning.reframe_blend, tuning.reframe_ease);
        }
    }

    /// Whether going from framing `a` to `b` in one step keeps the subjects
    /// and moves and turns no faster than a visible actor may.
    fn gradual(&self, a: &Framing, b: &Framing) -> bool {
        let turn =
            |x: f32, y: f32| ((x - y + 180.0).rem_euclid(360.0) - 180.0).abs() <= self.step_turn;
        let near = |x: f32, y: f32| (x - y).abs() <= self.step_move;
        match (*a, *b) {
            (
                Framing::Follow {
                    subject: s1,
                    distance: d1,
                    pitch: p1,
                    yaw: y1,
                    height: h1,
                    fov: f1,
                },
                Framing::Follow {
                    subject: s2,
                    distance: d2,
                    pitch: p2,
                    yaw: y2,
                    height: h2,
                    fov: f2,
                },
            ) => {
                s1 == s2
                    && near(d1, d2)
                    && near(h1, h2)
                    && turn(p1, p2)
                    && turn(y1, y2)
                    && turn(f1, f2)
            }
            (
                Framing::Frame {
                    a: a1,
                    b: b1,
                    pitch: p1,
                    yaw: y1,
                    height: h1,
                    fov: f1,
                    margin: m1,
                },
                Framing::Frame {
                    a: a2,
                    b: b2,
                    pitch: p2,
                    yaw: y2,
                    height: h2,
                    fov: f2,
                    margin: m2,
                },
            ) => {
                (a1, b1) == (a2, b2)
                    && near(h1, h2)
                    && near(m1, m2)
                    && turn(p1, p2)
                    && turn(y1, y2)
                    && turn(f1, f2)
            }
            (Framing::Fixed(p1), Framing::Fixed(p2)) => {
                p1.position.distance(p2.position) <= self.step_move
                    && p1.rotation.angle_between(p2.rotation).to_degrees() <= self.step_turn
                    && turn(p1.fov, p2.fov)
            }
            _ => false,
        }
    }

    pub fn set_tuning(&mut self, rig: u32, tuning: CameraSettings) -> Result<(), CameraError> {
        let r = self.rigs.get_mut(&rig).ok_or(CameraError::UnknownRig)?;
        r.tuning = tuning;
        r.spring = Spring::new(tuning.frequency, tuning.damping, STEP_SECONDS);
        Ok(())
    }

    pub fn set_bounds(&mut self, rig: u32, min: Vec3, max: Vec3) -> Result<(), CameraError> {
        let r = self.rigs.get_mut(&rig).ok_or(CameraError::UnknownRig)?;
        r.bounds = Some((min.min(max), min.max(max)));
        Ok(())
    }

    pub fn active(&self) -> Option<u32> {
        self.active
    }

    pub fn set_anchor(
        &mut self,
        rig: u32,
        subject: Entity,
        anchor: Anchor,
        name: u32,
    ) -> Result<(), CameraError> {
        if !self.rigs.contains_key(&rig) {
            return Err(CameraError::UnknownRig);
        }
        self.anchors.retain(|&(r, e, ..)| (r, e) != (rig, subject));
        if anchor != Anchor::Origin {
            self.anchors.push((rig, subject, anchor, name));
        }
        Ok(())
    }

    /// The bones and sockets the rig in use aims at, for the animation
    /// module to resolve. Until it does, the camera aims at the origins.
    pub fn anchors(&self) -> impl Iterator<Item = (Entity, Anchor, u32)> + '_ {
        let framed = self
            .active
            .and_then(|r| self.rigs.get(&r))
            .map_or([Entity::NONE; 2], |r| match r.framing {
                Framing::Follow { subject, .. } => [subject, Entity::NONE],
                Framing::Frame { a, b, .. } => [a, b],
                Framing::Fixed(_) => [Entity::NONE; 2],
            });
        self.anchors
            .iter()
            .filter(move |&&(r, e, ..)| Some(r) == self.active && framed.contains(&e))
            .map(|&(_, e, anchor, name)| (e, anchor, name))
    }

    pub fn shaking(&self) -> bool {
        self.shake.is_some()
    }

    /// Switches to `rig` over `blend` seconds; 0, or no pose shown yet, cuts.
    pub fn use_rig(&mut self, rig: u32, blend: f32, curve: Ease) -> Result<(), CameraError> {
        let r = self.rigs.get_mut(&rig).ok_or(CameraError::UnknownRig)?;
        r.ready = false;
        self.active = Some(rig);
        self.transition(blend, curve);
        Ok(())
    }

    fn transition(&mut self, blend: f32, curve: Ease) {
        self.blend = match self.pose {
            Some(from) if blend > 0.0 => Some(Blend {
                from,
                steps: steps_of(blend),
                elapsed: 0,
                ease: curve,
            }),
            _ => {
                self.cut = true;
                None
            }
        };
    }

    pub fn shake(&mut self, amplitude: f32, frequency: f32, duration: f32, rng: &mut Pcg32) {
        self.shake = Some(Shake {
            amplitude,
            frequency,
            steps: steps_of(duration),
            elapsed: 0,
            phase: std::array::from_fn(|_| rng.next_f32() * TAU),
        });
    }

    /// Whether this step's pose follows a cut, so it must not be interpolated.
    pub fn cut(&self) -> bool {
        self.cut
    }

    pub fn begin_step(&mut self) {
        self.previous = self.pose;
        self.cut = false;
        self.points.clear();
    }

    pub fn step(&mut self, world: &World) {
        let Some(active) = self.active else {
            return;
        };
        let points = &self.points;
        let Some(live) = self
            .rigs
            .get_mut(&active)
            .and_then(|r| r.step(world, points))
        else {
            return;
        };
        let mut pose = match &mut self.blend {
            Some(b) => {
                b.elapsed += 1;
                let t = ease(b.ease, b.elapsed as f32 / b.steps as f32);
                let pose = b.from.lerp(&live, t);
                if b.elapsed >= b.steps {
                    self.blend = None;
                }
                pose
            }
            None => live,
        };
        if let Some(s) = &mut self.shake {
            s.elapsed += 1;
            let t = s.elapsed as f32 * STEP_SECONDS;
            let fade = (1.0 - s.elapsed as f32 / s.steps as f32).max(0.0);
            let wave = |k: usize, rate: f32| (TAU * s.frequency * rate * t + s.phase[k]).sin();
            pose.position += s.amplitude
                * fade
                * fade
                * Vec3::new(wave(0, 1.0), wave(1, 1.31) * 0.6, wave(2, 0.87) * 0.3);
            if s.elapsed >= s.steps {
                self.shake = None;
            }
        }
        self.pose = Some(pose);
        if self.previous.is_none() {
            self.cut = true;
        }
    }

    pub fn pose(&self) -> Option<CameraPose> {
        self.pose
    }

    pub fn blending(&self) -> bool {
        self.blend.is_some()
    }

    pub fn interpolated(&self, alpha: f32) -> Option<CameraPose> {
        let now = self.pose?;
        match self.previous {
            Some(before) if !self.cut => Some(before.lerp(&now, alpha)),
            _ => Some(now),
        }
    }

    pub fn hash_into(&self, h: &mut crate::hash::StateHasher) {
        h.u32(self.active.unwrap_or(u32::MAX));
        if let Some(p) = self.pose {
            h.vec3(p.position);
            p.rotation.to_array().into_iter().for_each(|c| h.f32(c));
            h.f32(p.fov);
        }
    }
}
