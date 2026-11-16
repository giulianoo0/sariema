// SPDX-License-Identifier: Apache-2.0

//! Actors placed in a room by `run`: each plays a sequence of clips on the
//! animator's fixed step and is drawn at any time between two steps.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::Args;
use glam::{Mat4, Quat, Vec3};
use sariema_anim::{AnimSettings, Animator, ClipId, FaceId, Play, Rig, SocketId};
use sariema_core::Camera;
use sariema_format::{RoomCamera, RoomPack};
use sariema_render::{ActorId, ActorModel, ActorState, CutState, Renderer};
use serde_json::{Value, json};

use crate::actor;
use crate::output::{Failure, list};

#[derive(Args, Clone, Debug, Default)]
pub struct StageArgs {
    /// An actor pack placed at X,Y,Z with YAW degrees about +Y; repeatable.
    #[arg(long = "actor", value_name = "PACK@X,Y,Z,YAW")]
    pub actors: Vec<Placement>,
    /// The clip every actor plays from time 0; without one they stand at rest.
    #[arg(long, requires = "actors")]
    pub clip: Option<String>,
    /// A clip every actor fades to at SECONDS; repeatable, in order.
    #[arg(long = "then", value_name = "CLIP@SECONDS", requires = "clip")]
    pub then: Vec<Switch>,
    /// Seconds each fade to a `--then` clip takes; the animator's default without it.
    #[arg(long, requires = "then")]
    pub fade: Option<f32>,
    /// A face set to a frame, by name or atlas cell, at SECONDS (0 without
    /// it); repeatable. A clip with a track for the face overrides it.
    #[arg(
        long = "face",
        value_name = "FACE=FRAME[@SECONDS]",
        requires = "actors"
    )]
    pub faces: Vec<FaceChange>,
    /// A pack held at a socket of the actor given by its 0-based `--actor`
    /// order; repeatable.
    #[arg(long = "prop", value_name = "PACK@ACTOR:SOCKET", requires = "actors")]
    pub props: Vec<PropArg>,
    /// The time drawn: the screenshot's in headless runs, where the earlier
    /// frames lead up to it one step each; the start of a window's.
    #[arg(long, default_value_t = 0.0)]
    pub time: f32,
    /// Saves N frames from `--time` over `--span` seconds as one image instead
    /// of the last frame.
    #[arg(long, requires = "headless", value_parser = clap::value_parser!(u32).range(1..))]
    pub filmstrip: Option<u32>,
    /// Seconds the filmstrip covers; the duration of `--clip` without it.
    #[arg(long, requires = "filmstrip")]
    pub span: Option<f32>,
    /// Frames per filmstrip row.
    #[arg(long, default_value_t = 4, requires = "filmstrip", value_parser = clap::value_parser!(u32).range(1..))]
    pub columns: u32,
    /// Replaces the room camera's focus.
    #[arg(long, value_name = "X,Y,Z")]
    pub focus: Option<Triple>,
    /// Replaces the room camera's distance from its focus, in metres.
    #[arg(long)]
    pub distance: Option<f32>,
    /// Replaces the room camera's pitch, in degrees.
    #[arg(long)]
    pub pitch: Option<f32>,
}

impl StageArgs {
    /// The room's camera with the overrides given.
    pub fn camera(&self, room: &RoomCamera) -> RoomCamera {
        RoomCamera {
            focus: self.focus.map_or(room.focus, |f| f.0),
            distance: self.distance.unwrap_or(room.distance),
            pitch_deg: self.pitch.unwrap_or(room.pitch_deg),
            ..room.clone()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Triple(pub [f32; 3]);

impl FromStr for Triple {
    type Err = String;

    fn from_str(s: &str) -> Result<Triple, String> {
        let numbers = numbers(s)?;
        <[f32; 3]>::try_from(numbers)
            .map(Triple)
            .map_err(|_| format!("{s}: expected X,Y,Z"))
    }
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub pack: PathBuf,
    pub position: Vec3,
    pub yaw_deg: f32,
}

impl FromStr for Placement {
    type Err = String;

    fn from_str(s: &str) -> Result<Placement, String> {
        let (pack, at) = s
            .rsplit_once('@')
            .ok_or_else(|| format!("{s}: expected PACK@X,Y,Z,YAW"))?;
        let [x, y, z, yaw] = <[f32; 4]>::try_from(numbers(at)?)
            .map_err(|_| format!("{s}: expected PACK@X,Y,Z,YAW"))?;
        Ok(Placement {
            pack: pack.into(),
            position: Vec3::new(x, y, z),
            yaw_deg: yaw,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Switch {
    pub clip: String,
    pub at_s: f32,
}

impl FromStr for Switch {
    type Err = String;

    fn from_str(s: &str) -> Result<Switch, String> {
        let (clip, at) = s
            .rsplit_once('@')
            .ok_or_else(|| format!("{s}: expected CLIP@SECONDS"))?;
        Ok(Switch {
            clip: clip.into(),
            at_s: seconds(at)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FaceChange {
    pub face: String,
    pub frame: String,
    pub at_s: f32,
}

impl FromStr for FaceChange {
    type Err = String;

    fn from_str(s: &str) -> Result<FaceChange, String> {
        let (face, rest) = s
            .split_once('=')
            .ok_or_else(|| format!("{s}: expected FACE=FRAME[@SECONDS]"))?;
        let (frame, at_s) = match rest.rsplit_once('@') {
            Some((frame, at)) => (frame, seconds(at)?),
            None => (rest, 0.0),
        };
        Ok(FaceChange {
            face: face.into(),
            frame: frame.into(),
            at_s,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PropArg {
    pub pack: PathBuf,
    pub actor: usize,
    pub socket: String,
}

impl FromStr for PropArg {
    type Err = String;

    fn from_str(s: &str) -> Result<PropArg, String> {
        let bad = || format!("{s}: expected PACK@ACTOR:SOCKET");
        let (pack, at) = s.rsplit_once('@').ok_or_else(bad)?;
        let (actor, socket) = at.split_once(':').ok_or_else(bad)?;
        Ok(PropArg {
            pack: pack.into(),
            actor: actor.parse().map_err(|_| bad())?,
            socket: socket.into(),
        })
    }
}

fn numbers(s: &str) -> Result<Vec<f32>, String> {
    s.split(',')
        .map(|n| {
            let value: f32 = n
                .trim()
                .parse()
                .map_err(|_| format!("{s}: {n} is no number"))?;
            value
                .is_finite()
                .then_some(value)
                .ok_or_else(|| format!("{s}: {n} is not finite"))
        })
        .collect()
}

fn seconds(s: &str) -> Result<f32, String> {
    match s.parse::<f32>() {
        Ok(t) if t.is_finite() && t >= 0.0 => Ok(t),
        _ => Err(format!("{s}: expected seconds, at least 0")),
    }
}

/// A pack loaded once however many actors show it.
struct Loaded {
    path: PathBuf,
    model: ActorModel,
    rig: Rig,
}

struct Staged {
    id: ActorId,
    pack: usize,
    animator: Animator,
    model: Mat4,
    /// When each clip starts, the first at 0; and which have.
    plays: Vec<(f32, ClipId)>,
    faces: Vec<(f32, FaceId, u32)>,
    started: usize,
    faced: usize,
}

struct Prop {
    id: ActorId,
    actor: usize,
    socket: SocketId,
    palette: Vec<Mat4>,
}

/// The actors of a run and the time they have been simulated to.
pub struct Stage {
    loaded: Vec<Loaded>,
    actors: Vec<Staged>,
    props: Vec<Prop>,
    step_s: f32,
    fade_s: Option<f32>,
    steps: u64,
}

impl Stage {
    /// Loads the packs `args` names into `renderer` and places the actors.
    pub fn load(renderer: &mut Renderer, args: &StageArgs) -> Result<Stage, Failure> {
        let settings = AnimSettings::default();
        let mut stage = Stage {
            loaded: Vec::new(),
            actors: Vec::new(),
            props: Vec::new(),
            step_s: settings.step_s,
            fade_s: args.fade,
            steps: 0,
        };
        for placement in &args.actors {
            let pack = stage.pack(renderer, &placement.pack)?;
            let rig = &stage.loaded[pack].rig;
            let clip = |name: &str| clip(rig, &placement.pack, name);
            let mut plays = Vec::new();
            if let Some(first) = &args.clip {
                plays.push((0.0, clip(first)?));
            }
            for switch in &args.then {
                plays.push((switch.at_s, clip(&switch.clip)?));
            }
            let faces = (args.faces.iter())
                .map(|change| face(rig, &placement.pack, change))
                .collect::<Result<Vec<_>, _>>()?;
            let mut faces = faces;
            faces.sort_by(|a, b| a.0.total_cmp(&b.0));
            let id = renderer.spawn_actor(stage.loaded[pack].model)?;
            let model = Mat4::from_rotation_translation(
                Quat::from_rotation_y(placement.yaw_deg.to_radians()),
                placement.position,
            );
            stage.actors.push(Staged {
                id,
                pack,
                animator: Animator::new(&stage.loaded[pack].rig, settings.clone()),
                model,
                plays,
                faces,
                started: 0,
                faced: 0,
            });
        }
        for prop in &args.props {
            let Some(holder) = stage.actors.get(prop.actor) else {
                return Err(Failure::new(
                    "E_SCHEMA",
                    format!(
                        "--prop {}: there are {} actors",
                        prop.pack.display(),
                        stage.actors.len()
                    ),
                ));
            };
            let rig = &stage.loaded[holder.pack].rig;
            let socket = rig.socket(&prop.socket).ok_or_else(|| {
                let names: Vec<&str> = (0..rig.sockets())
                    .map(|s| rig.socket_name(SocketId(s as u32)))
                    .collect();
                Failure::new(
                    "E_ACTOR_NAME",
                    format!(
                        "{}: no socket {}; its sockets are {}",
                        args.actors[prop.actor].pack.display(),
                        prop.socket,
                        list(&names)
                    ),
                )
            })?;
            let pack = stage.pack(renderer, &prop.pack)?;
            let rig = &stage.loaded[pack].rig;
            let palette = Animator::new(rig, settings.clone()).palette().to_vec();
            let id = renderer.spawn_actor(stage.loaded[pack].model)?;
            stage.props.push(Prop {
                id,
                actor: prop.actor,
                socket,
                palette,
            });
        }
        Ok(stage)
    }

    fn pack(&mut self, renderer: &mut Renderer, path: &Path) -> Result<usize, Failure> {
        if let Some(i) = self.loaded.iter().position(|l| l.path == path) {
            return Ok(i);
        }
        let pack = actor::read(path)?;
        let rig = Rig::new(&pack)
            .map_err(|why| Failure::new("E_SCHEMA", format!("{}: {why}", path.display())))?;
        let model = renderer.load_actor(&pack)?;
        self.loaded.push(Loaded {
            path: path.to_path_buf(),
            model,
            rig,
        });
        Ok(self.loaded.len() - 1)
    }

    /// Simulates the actors up to `time_s`, which must not go back, and
    /// poses them there, between the two steps around it.
    pub fn pose(&mut self, renderer: &mut Renderer, time_s: f32) -> Result<(), Failure> {
        let step = f64::from(self.step_s);
        let time = f64::from(time_s.max(0.0));
        let needed = (time / step).ceil() as u64;
        while self.steps < needed {
            let now = (self.steps as f64 * step) as f32;
            for actor in &mut self.actors {
                let rig = &self.loaded[actor.pack].rig;
                while let Some(&(at, clip)) = actor.plays.get(actor.started)
                    && at <= now
                {
                    let fade_s = if actor.started == 0 {
                        Some(0.0)
                    } else {
                        self.fade_s
                    };
                    let play = Play {
                        fade_s,
                        ..Play::default()
                    };
                    actor.animator.play(rig, 0, clip, play);
                    actor.started += 1;
                }
                while let Some(&(at, face, cell)) = actor.faces.get(actor.faced)
                    && at <= now
                {
                    actor.animator.set_face(face, cell);
                    actor.faced += 1;
                }
                actor.animator.step(rig);
            }
            self.steps += 1;
        }
        let alpha = if needed == 0 {
            1.0
        } else {
            1.0 - ((needed as f64 * step - time) / step) as f32
        };
        for actor in &mut self.actors {
            let rig = &self.loaded[actor.pack].rig;
            actor.animator.evaluate(rig, alpha);
            let state = ActorState {
                model: actor.model,
                palette: actor.animator.palette(),
                faces: actor.animator.faces(),
                visible: true,
            };
            renderer.set_actor(actor.id, &state)?;
        }
        for prop in &self.props {
            let holder = &self.actors[prop.actor];
            let rig = &self.loaded[holder.pack].rig;
            let state = ActorState {
                model: holder.model * holder.animator.socket(rig, prop.socket),
                palette: &prop.palette,
                faces: &[],
                visible: true,
            };
            renderer.set_actor(prop.id, &state)?;
        }
        Ok(())
    }

    /// The duration of the first actor's first clip.
    pub fn first_clip_s(&self) -> Option<f32> {
        let actor = self.actors.first()?;
        let &(_, clip) = actor.plays.first()?;
        Some(self.loaded[actor.pack].rig.clip_info(clip).duration)
    }

    pub fn json(&self) -> Value {
        json!(
            self.actors
                .iter()
                .map(|a| {
                    let loaded = &self.loaded[a.pack];
                    json!({
                        "pack": loaded.path.display().to_string(),
                        "joints": loaded.rig.joints(),
                        "position": a.model.w_axis.truncate().to_array(),
                    })
                })
                .collect::<Vec<_>>()
        )
    }
}

fn clip(rig: &Rig, pack: &Path, name: &str) -> Result<ClipId, Failure> {
    rig.clip(name).ok_or_else(|| {
        let names: Vec<&str> = (0..rig.clips())
            .map(|c| rig.clip_info(ClipId(c as u32)).name.as_str())
            .collect();
        Failure::new(
            "E_ACTOR_NAME",
            format!(
                "{}: no clip {name}; its clips are {}",
                pack.display(),
                list(&names)
            ),
        )
    })
}

fn face(rig: &Rig, pack: &Path, change: &FaceChange) -> Result<(f32, FaceId, u32), Failure> {
    let face = rig.face(&change.face).ok_or_else(|| {
        let names: Vec<&str> = (0..rig.faces())
            .map(|f| rig.face_name(FaceId(f as u32)))
            .collect();
        Failure::new(
            "E_ACTOR_NAME",
            format!(
                "{}: no face {}; its faces are {}",
                pack.display(),
                change.face,
                list(&names)
            ),
        )
    })?;
    let cell = (change.frame.parse::<u32>().ok()).or_else(|| rig.face_frame(face, &change.frame));
    let cell = cell.ok_or_else(|| {
        Failure::new(
            "E_ACTOR_NAME",
            format!(
                "{}: face {} has no frame {}; its frames are {}",
                pack.display(),
                change.face,
                change.frame,
                list(&rig.face_frames(face).collect::<Vec<_>>())
            ),
        )
    })?;
    Ok((change.at_s, face, cell))
}

/// The camera of the room's `camera` overridden by `args`.
pub fn camera(args: &StageArgs, room: &RoomCamera, aspect: f32) -> Camera {
    sariema_core::room_camera(&args.camera(room), aspect)
}

/// Cuts the room's cutaways around the first actor, as a game whose hero it
/// is would once the line settles, seen by the room's camera.
pub fn cut_around(renderer: &mut Renderer, room: &RoomPack, actors: &[Placement]) {
    let Some(hero) = actors.first().map(|a| [a.position.x, a.position.z]) else {
        return;
    };
    let toward = sariema_runtime::scenery::toward(0.0);
    for (c, cutaway) in room.cutaways.iter().enumerate() {
        if let Some(line) = cutaway.line.at(hero, toward) {
            let height = cutaway.height;
            let state = CutState {
                toward,
                line,
                height,
                hero,
            };
            renderer.set_cutaway(0, c, state);
        }
    }
}
