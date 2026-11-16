// SPDX-License-Identifier: Apache-2.0

//! The game's rooms on the GPU. Each frame the view keeps loaded the rooms
//! the engine wants ([`Engine::wanted_rooms`]: the active one, the one a
//! switch leaves, the ones beside it, the preloaded ones, the neighbours),
//! at most
//! [`ViewSettings::max_resident`], freeing the others. A room is decoded on
//! a thread, or a share at a time where there are none, then uploaded a
//! share at a time, so loading never takes a frame's whole budget; only a
//! switch to a room not loaded yet loads it at once. Their collision worlds
//! are built the same way: whole on a thread, or a share of their walkable
//! grids each frame. The rooms beside the
//! active one, whose geometry continues it, are drawn with it. While a
//! switch lasts both rooms are drawn: whole and lit by a blend of the two
//! (continuous), or dissolving into each other (dissolve); the sky, the
//! sun, the ambient light and the look blend either way. What the game changed in a
//! room (groups, piece states, lights, hinges, nodes bound to entities) is
//! carried to the renderer when it changes.

use std::sync::Arc;
use std::time::Duration;

use glam::{Mat4, Quat, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_engine::{Engine, WorldJob};
use sariema_format::scenery::{Channel, LightField, NodeIndex, RoomIndex, Target, sun_direction};
use sariema_format::script::{Ease, RoomTransition};
use sariema_format::stream::RoomDecoder;
use sariema_format::{Background, Look, RoomCamera, RoomPack, Sun};
use sariema_render::{Lighting, Renderer};
use sariema_runtime::ease::ease;
use sariema_runtime::scenery::{Binding, RoomScene};
use sariema_runtime::sim::Logic;

use crate::ViewError;

/// How the view loads rooms.
#[derive(Clone, Debug)]
pub struct ViewSettings {
    /// Rooms on the GPU at once; the least wanted is freed past it.
    pub max_resident: usize,
    /// Bytes of room packs decoded in a frame where rooms are not decoded
    /// on threads.
    pub decode_bytes_per_frame: usize,
    /// Bytes of meshes and textures uploaded in a frame.
    pub upload_bytes_per_frame: u64,
    /// Rooms are decoded and collision worlds built on threads; ignored on
    /// wasm32, which has none.
    pub threads: bool,
    /// Milliseconds a frame spends building collision worlds where they
    /// are not built on threads, and growing the walkable grids a switch
    /// left: a share that would end past them, by the longest of the
    /// frame's so far, waits for the next frame; at least one share a frame.
    pub world_ms_per_frame: f32,
    /// Walkable-grid columns in a share; a column costs from a few to a few
    /// hundred microseconds, with the geometry above it.
    pub grid_columns_per_share: usize,
    /// Work units of a collision world's colliders and the tree over them
    /// a share of its building takes, between checks of the time budget
    /// (`sariema_physics::WorldBuild::advance`).
    pub world_units_per_share: usize,
    /// Off, no light casts a shadow, as for a device without the budget.
    pub shadows: bool,
    /// Seconds the picture of the room left takes to fade out over the next
    /// one in a dissolve whose `room_switch` gives no duration.
    pub dissolve_s: f32,
    /// The curve of that fade.
    pub dissolve_ease: Ease,
    /// Angular frequency, in rad/s, of the damped spring a game without a
    /// camera rig slides its room's camera on, across a continuous switch,
    /// to the next room's; 0 cuts.
    pub camera_slide_frequency: f32,
    /// Its damping ratio: below 1 it overshoots a little.
    pub camera_slide_damping: f32,
}

impl Default for ViewSettings {
    fn default() -> ViewSettings {
        ViewSettings {
            max_resident: 3,
            decode_bytes_per_frame: 4 << 20,
            upload_bytes_per_frame: 4 << 20,
            threads: true,
            world_ms_per_frame: 2.0,
            grid_columns_per_share: 8,
            world_units_per_share: 2048,
            shadows: true,
            dissolve_s: 0.5,
            dissolve_ease: Ease::InOutSine,
            camera_slide_frequency: 8.4,
            camera_slide_damping: 0.8,
        }
    }
}

/// What loading rooms did in the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomsFrame {
    pub bytes_decoded: u64,
    pub bytes_uploaded: u64,
    /// Rooms that finished uploading.
    pub rooms_loaded: u32,
    /// A room needed this frame was loaded whole in it.
    pub loaded_at_once: bool,
    pub rooms_resident: u32,
    /// A collision world was built a share of in the frame.
    pub world_share: bool,
    /// Collision worlds that came in.
    pub worlds_built: u32,
    /// Time spent decoding rooms, uploading them (their pipelines and
    /// buffers included) and building collision worlds in the frame.
    pub decode: Duration,
    pub upload: Duration,
    pub world: Duration,
}

enum Load {
    Absent,
    Decoding(Box<RoomDecoder<Arc<[u8]>>>),
    #[cfg(not(target_arch = "wasm32"))]
    Threaded(Receiver<Decoded>),
    Uploading(Box<RoomPack>),
    Resident,
}

/// What a room's pack lights and frames it with, before the game changes
/// any parameter.
#[derive(Clone)]
struct Base {
    sun: Option<Sun>,
    sh9: [[f32; 3]; 9],
    look: Look,
    camera: RoomCamera,
}

/// What the renderer's slot shows of the room's scene.
struct Applied {
    groups: Vec<bool>,
    pieces: Vec<u32>,
    params: Vec<f32>,
    offsets: Vec<Mat4>,
    visible: Vec<bool>,
    /// By node, where the entity bound to it moved it this frame.
    bound: Vec<Option<Mat4>>,
}

struct RoomView {
    bytes: Option<Arc<[u8]>>,
    load: Load,
    base: Option<Base>,
    applied: Option<Applied>,
    /// By light, where its colour, intensity and range parameters are.
    light_params: Vec<[usize; 5]>,
    /// Rooms drawn with this one.
    beside: Vec<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
type Built = Result<sariema_engine::BuiltWorld, String>;
#[cfg(not(target_arch = "wasm32"))]
type Decoded = Box<Result<RoomPack, sariema_format::FormatError>>;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, Sender};

pub(crate) struct Rooms {
    views: Vec<RoomView>,
    settings: ViewSettings,
    drawn: Vec<u32>,
    lighting: Option<Lighting>,
    look: Option<Look>,
    frame: RoomsFrame,
    #[cfg(not(target_arch = "wasm32"))]
    worlds: (Sender<Built>, Receiver<Built>),
    /// The world built a share a frame.
    job: Option<WorldJob>,
    /// The longest share of collision work in the last frame that had any.
    longest_share: Duration,
    /// The step of the last switch drawn.
    switched: Option<u64>,
    /// The dissolve under way: the step that switched, and its seconds.
    dissolve: Option<(u64, f32)>,
}

/// Runs `share` while it has more to do and the next share, expected to
/// take as long as the longest of this frame's or the last one's, would end
/// within `budget`; at least once. The last frame's counts for half the
/// budget at most: a share the system paused says nothing of the next.
/// Whether any share had work.
fn within(budget: Duration, longest: &mut Duration, mut share: impl FnMut() -> bool) -> bool {
    let start = web_time::Instant::now();
    let last = std::mem::take(longest).min(budget / 2);
    let mut worked = false;
    loop {
        let begun = web_time::Instant::now();
        if !share() {
            break;
        }
        worked = true;
        *longest = (*longest).max(begun.elapsed());
        if start.elapsed() + (*longest).max(last) > budget {
            break;
        }
    }
    if !worked {
        *longest = last;
    }
    worked
}

fn light_params(index: &RoomIndex) -> Vec<[usize; 5]> {
    let at = |target: Target| index.params.iter().position(|p| p.target == target);
    (0..index.lights.len() as u32)
        .map(|l| {
            let field = |f| at(Target::Light(l, f)).expect("every light has its parameters");
            [
                field(LightField::Color(Channel::R)),
                field(LightField::Color(Channel::G)),
                field(LightField::Color(Channel::B)),
                field(LightField::Intensity),
                field(LightField::Range),
            ]
        })
        .collect()
}

impl Rooms {
    pub fn new<L: Logic>(engine: &Engine<L>, settings: ViewSettings) -> Rooms {
        let views = (engine.rooms().iter())
            .map(|r| RoomView {
                bytes: r.room.clone(),
                load: Load::Absent,
                base: None,
                applied: None,
                light_params: light_params(&r.index),
                beside: r.beside.clone(),
            })
            .collect();
        Rooms {
            views,
            settings,
            drawn: Vec::with_capacity(2),
            lighting: None,
            look: None,
            frame: RoomsFrame::default(),
            #[cfg(not(target_arch = "wasm32"))]
            worlds: std::sync::mpsc::channel(),
            job: None,
            longest_share: Duration::ZERO,
            switched: None,
            dissolve: None,
        }
    }

    /// Loads room `r` and the rooms beside it whole now; `r`'s pack's
    /// camera.
    pub fn load_now(
        &mut self,
        renderer: &mut Renderer,
        r: usize,
    ) -> Result<Option<RoomCamera>, ViewError> {
        if self.views.get(r).is_none_or(|v| v.bytes.is_none()) {
            return Ok(None);
        }
        let beside = self.views[r].beside.clone();
        let room = std::iter::once(r as u32).chain(beside);
        let drawn: Vec<u32> = (room.take(self.settings.max_resident.max(1)))
            .filter(|&b| {
                self.views
                    .get(b as usize)
                    .is_some_and(|v| v.bytes.is_some())
            })
            .collect();
        for &b in &drawn {
            self.finish_now(renderer, b as usize)?;
        }
        renderer.set_drawn(&drawn);
        let base = self.views[r]
            .base
            .as_ref()
            .expect("an uploaded room has its base");
        renderer.set_look(&base.look);
        Ok(Some(base.camera.clone()))
    }

    pub fn frame(&self) -> RoomsFrame {
        self.frame
    }

    /// The camera of the active room's pack, for a game that set none.
    pub fn camera(&self, active: usize) -> Option<&RoomCamera> {
        Some(&self.views.get(active)?.base.as_ref()?.camera)
    }

    /// Loads, frees and shows the rooms for the frame at `alpha`.
    pub fn update<L: Logic>(
        &mut self,
        renderer: &mut Renderer,
        engine: &mut Engine<L>,
        alpha: f32,
    ) -> Result<(), ViewError> {
        self.frame = RoomsFrame::default();
        let mut wanted = engine.wanted_rooms();
        wanted.retain(|&r| self.views[r].bytes.is_some());
        wanted.truncate(self.settings.max_resident.max(1));
        let start = web_time::Instant::now();
        self.build_worlds(engine, &wanted);
        self.frame.world = start.elapsed();
        for (r, view) in self.views.iter_mut().enumerate() {
            if !wanted.contains(&r) && !matches!(view.load, Load::Absent) {
                renderer.unload_room(r as u32);
                view.load = Load::Absent;
                view.applied = None;
            }
        }
        let mut decode = self.settings.decode_bytes_per_frame;
        let mut upload = self.settings.upload_bytes_per_frame;
        for &r in &wanted {
            self.advance(renderer, r, &mut decode, &mut upload)?;
        }
        let active = engine.state().scenery.active();
        let beside = self.views.get(active).map_or(0, |v| v.beside.len());
        for i in 0..=beside {
            let r = match i {
                0 => active,
                i => self.views[active].beside[i - 1] as usize,
            };
            let due = (self.views.get(r))
                .is_some_and(|v| v.bytes.is_some() && !matches!(v.load, Load::Resident));
            if due && wanted.contains(&r) {
                self.frame.loaded_at_once = true;
                self.finish_now(renderer, r)?;
            }
        }
        self.frame.rooms_resident = (self.views.iter())
            .filter(|v| matches!(v.load, Load::Resident))
            .count() as u32;
        self.show(renderer, engine, alpha);
        Ok(())
    }

    /// Loads room `r` the rest of the way now, from wherever it is.
    fn finish_now(&mut self, renderer: &mut Renderer, r: usize) -> Result<(), ViewError> {
        let (mut decode, mut upload) = (usize::MAX, u64::MAX);
        if matches!(self.views[r].load, Load::Absent) {
            let bytes = self.views[r]
                .bytes
                .clone()
                .expect("a room loaded has bytes");
            self.uploading(renderer, r, RoomDecoder::decode(bytes))?;
        }
        while !matches!(self.views[r].load, Load::Resident) {
            #[cfg(not(target_arch = "wasm32"))]
            if let Load::Threaded(rx) = &self.views[r].load {
                let decoded = rx.recv().expect("a decoding thread sends its room");
                self.uploading(renderer, r, *decoded)?;
                continue;
            }
            self.advance(renderer, r, &mut decode, &mut upload)?;
        }
        Ok(())
    }

    fn build_worlds<L: Logic>(&mut self, engine: &mut Engine<L>, wanted: &[usize]) {
        let budget = Duration::from_secs_f32(self.settings.world_ms_per_frame.max(0.0) / 1000.0);
        let columns = self.settings.grid_columns_per_share.max(1);
        let units = self.settings.world_units_per_share.max(1);
        #[cfg(not(target_arch = "wasm32"))]
        if self.settings.threads {
            while let Ok(built) = self.worlds.1.try_recv() {
                if let Ok(built) = built {
                    engine.install_world(built);
                    self.frame.worlds_built += 1;
                }
            }
            for job in engine.world_jobs(wanted) {
                let send = self.worlds.0.clone();
                std::thread::spawn(move || send.send(job.build()));
            }
            self.frame.world_share = within(budget, &mut self.longest_share, || {
                engine.grow_grids(columns)
            });
            return;
        }
        let job = &mut self.job;
        let worlds_built = &mut self.frame.worlds_built;
        self.frame.world_share = within(budget, &mut self.longest_share, || {
            if job.as_ref().is_some_and(|j| engine.world_ready(j.room)) {
                *job = None;
            }
            if job.is_none() {
                let next = wanted.iter().copied().find(|&r| !engine.world_ready(r));
                *job = next.and_then(|r| engine.world_jobs(&[r]).pop());
            }
            match job {
                Some(building) => {
                    match building.advance(columns, units) {
                        Ok(None) => return true,
                        Ok(Some(built)) => {
                            engine.install_world(built);
                            *worlds_built += 1;
                        }
                        Err(_) => {}
                    }
                    *job = None;
                    true
                }
                None => engine.grow_grids(columns),
            }
        });
    }

    /// Moves room `r`'s load on within what is left of this frame's budgets.
    fn advance(
        &mut self,
        renderer: &mut Renderer,
        r: usize,
        decode: &mut usize,
        upload: &mut u64,
    ) -> Result<(), ViewError> {
        let next = match std::mem::replace(&mut self.views[r].load, Load::Absent) {
            Load::Absent => match self.views[r].bytes.clone() {
                Some(bytes) => self.start(bytes),
                None => Load::Absent,
            },
            Load::Decoding(mut decoder) if *decode > 0 => {
                let before = decoder.progress().0;
                let start = web_time::Instant::now();
                let room = decoder.step(*decode).map_err(failed);
                self.frame.decode += start.elapsed();
                let room = room?;
                let read = decoder.progress().0 - before;
                *decode = decode.saturating_sub(read);
                self.frame.bytes_decoded += read as u64;
                match room {
                    Some(room) => return self.uploading(renderer, r, Ok(room)),
                    None => Load::Decoding(decoder),
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Load::Threaded(rx) => match rx.try_recv() {
                Ok(decoded) => return self.uploading(renderer, r, *decoded),
                Err(std::sync::mpsc::TryRecvError::Empty) => Load::Threaded(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Load::Absent,
            },
            Load::Uploading(room) if *upload > 0 => {
                let before = renderer.upload_progress(r as u32).map_or(0, |p| p.0);
                let start = web_time::Instant::now();
                let done = renderer.upload_room(r as u32, &room, *upload);
                self.frame.upload += start.elapsed();
                let sent = match renderer.upload_progress(r as u32) {
                    Some((after, _)) => after - before,
                    None => room_bytes(&room).saturating_sub(before),
                };
                *upload = upload.saturating_sub(sent);
                self.frame.bytes_uploaded += sent;
                if done {
                    self.frame.rooms_loaded += 1;
                    Load::Resident
                } else {
                    Load::Uploading(room)
                }
            }
            load => load,
        };
        self.views[r].load = next;
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&self, bytes: Arc<[u8]>) -> Load {
        if !self.settings.threads {
            return Load::Decoding(Box::new(RoomDecoder::new(bytes)));
        }
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || send.send(Box::new(RoomDecoder::decode(bytes))));
        Load::Threaded(receive)
    }

    #[cfg(target_arch = "wasm32")]
    fn start(&self, bytes: Arc<[u8]>) -> Load {
        Load::Decoding(Box::new(RoomDecoder::new(bytes)))
    }

    fn uploading(
        &mut self,
        renderer: &mut Renderer,
        r: usize,
        decoded: Result<RoomPack, sariema_format::FormatError>,
    ) -> Result<(), ViewError> {
        let mut room = decoded.map_err(failed)?;
        if !self.settings.shadows {
            crate::without_shadows(&mut room);
        }
        self.views[r].base = Some(Base {
            sun: room.sun.clone(),
            sh9: room.ambient.sh9,
            look: room.look.clone(),
            camera: room.camera.clone(),
        });
        let start = web_time::Instant::now();
        let begun = renderer.begin_room(r as u32, &room);
        self.frame.upload += start.elapsed();
        begun?;
        self.views[r].load = Load::Uploading(Box::new(room));
        Ok(())
    }

    /// The drawn rooms, their presence and lighting, and every change the
    /// game made to a resident room.
    fn show<L: Logic>(&mut self, renderer: &mut Renderer, engine: &Engine<L>, alpha: f32) {
        self.fade(renderer, engine, alpha);
        let state = engine.state();
        let scenery = &state.scenery;
        let active = scenery.active();
        for (r, view) in self.views.iter_mut().enumerate() {
            if let (Load::Resident, Some(scene)) = (&view.load, scenery.room(r)) {
                sync(renderer, r, view, scene, scenery, &state.world, alpha);
            }
        }
        let resident = |r: usize| {
            self.views
                .get(r)
                .is_some_and(|v| matches!(v.load, Load::Resident))
        };
        let switch = scenery
            .switch()
            .filter(|s| resident(s.from) && s.kind == RoomTransition::Continuous);
        self.drawn.clear();
        if resident(active) {
            self.drawn.push(active as u32);
        }
        let lit = |r: usize| {
            (self.views[r].base.as_ref())
                .zip(scenery.room(r))
                .map(|(b, s)| lit(b, s))
        };
        let beside = &self.views.get(active).map_or(&[][..], |v| &v.beside[..]);
        let (lighting, look) = match (switch, lit(active)) {
            (Some(switch), Some(to)) => {
                let p = switch.progress(alpha);
                // A room beside the next one keeps its light its own.
                let lights_cross = !beside.contains(&(switch.from as u32));
                self.drawn.push(switch.from as u32);
                let (into, out) = (active as u32, switch.from as u32);
                renderer.set_room_presence(into, 1.0, false);
                renderer.set_room_presence(out, 1.0, false);
                renderer.set_room_light_scale(into, if lights_cross { p } else { 1.0 });
                renderer.set_room_light_scale(out, if lights_cross { 1.0 - p } else { 1.0 });
                match lit(switch.from) {
                    Some(from) => blend(&from, &to, p),
                    None => to,
                }
            }
            (_, Some(to)) => {
                renderer.set_room_presence(active as u32, 1.0, false);
                renderer.set_room_light_scale(active as u32, 1.0);
                to
            }
            (_, None) => return,
        };
        for &b in beside.iter() {
            if resident(b as usize) && !self.drawn.contains(&b) {
                self.drawn.push(b);
                renderer.set_room_presence(b, 1.0, false);
                renderer.set_room_light_scale(b, 1.0);
            }
        }
        renderer.set_drawn(&self.drawn);
        if self.lighting.as_ref() != Some(&lighting) {
            renderer.set_lighting(Some(lighting.clone()));
            self.lighting = Some(lighting);
        }
        if self.look.as_ref() != Some(&look) {
            renderer.set_look(&look);
            self.look = Some(look);
        }
    }

    /// A dissolve shows the frame drawn last, of the room left, fading out
    /// over the next room as its own camera and light show it; the room
    /// left is not drawn. A frame that runs a whole dissolve's steps still
    /// shows it from its start.
    fn fade<L: Logic>(&mut self, renderer: &mut Renderer, engine: &Engine<L>, alpha: f32) {
        let switched = engine.switched();
        if switched.map(|s| s.step) != self.switched {
            self.switched = switched.map(|s| s.step);
            let dissolve = switched.and_then(|s| Some((s.step, s.switch?)));
            self.dissolve = match dissolve {
                Some((step, s)) if s.kind == RoomTransition::Dissolve => {
                    renderer.hold_frame();
                    let seconds = match s.steps {
                        0 | 1 => self.settings.dissolve_s,
                        steps => steps as f32 * STEP_SECONDS,
                    };
                    Some((step, seconds))
                }
                _ => None,
            };
        }
        let opacity = match self.dissolve {
            Some((start, seconds)) => {
                let t = ((engine.steps() - start) as f32 - 1.0 + alpha) * STEP_SECONDS;
                let t = (t / seconds.max(STEP_SECONDS)).clamp(0.0, 1.0);
                if t >= 1.0 {
                    self.dissolve = None;
                }
                1.0 - ease(self.settings.dissolve_ease, t)
            }
            None => 0.0,
        };
        renderer.set_held_frame(opacity);
    }
}

fn failed(e: sariema_format::FormatError) -> ViewError {
    ViewError {
        code: e.code(),
        message: format!("game pack room: {e}"),
    }
}

fn room_bytes(room: &RoomPack) -> u64 {
    let meshes: u64 = (room.meshes.iter())
        .map(|m| {
            (m.vertices.len() * size_of::<sariema_format::Vertex>() + m.indices.len() * 4) as u64
        })
        .sum();
    let textures: u64 = room
        .textures
        .iter()
        .flat_map(|t| &t.mips)
        .map(|m| m.len() as u64)
        .sum();
    meshes + textures
}

/// The room's lighting and look with the game's parameters applied.
fn lit(base: &Base, scene: &RoomScene) -> (Lighting, Look) {
    let mut look = base.look.clone();
    let mut sun = base.sun.clone();
    let (mut angles, mut ambient, mut tint) = (None::<(f32, f32)>, 1.0, [1.0f32; 3]);
    let mut background = Background {
        top: [0.0; 3],
        bottom: [0.0; 3],
    };
    for (param, &value) in scene.index.params.iter().zip(&scene.params) {
        let c = |c: Channel| c as usize;
        match param.target {
            Target::Exposure => look.exposure = value,
            Target::BloomThreshold => look.bloom_threshold = value,
            Target::BloomStrength => look.bloom_strength = value,
            Target::Tint(ch) => look.tint[c(ch)] = value,
            Target::Real => look.real = value,
            Target::Night => look.night = value,
            Target::Tilt => look.tilt = value,
            Target::Focus => look.focus = value,
            Target::AoStrength => look.ao_strength = value.clamp(0.0, 1.0),
            Target::SunIntensity => sun.iter_mut().for_each(|s| s.intensity = value),
            Target::SunColor(ch) => sun.iter_mut().for_each(|s| s.color[c(ch)] = value),
            Target::SunAzimuth => angles = Some((value, angles.map_or(0.0, |a| a.1))),
            Target::SunElevation => angles = Some((angles.map_or(0.0, |a| a.0), value)),
            Target::Ambient => ambient = value,
            Target::AmbientColor(ch) => tint[c(ch)] = value,
            Target::BackgroundTop(ch) => background.top[c(ch)] = value,
            Target::BackgroundBottom(ch) => background.bottom[c(ch)] = value,
            Target::FogDensity | Target::FogColor(_) | Target::FogStart => {}
            Target::Light(..) | Target::Hinge(_) => {}
        }
    }
    if let (Some(sun), Some((azimuth, elevation))) = (&mut sun, angles) {
        sun.direction = sun_direction(azimuth, elevation);
    }
    let sh9 = base
        .sh9
        .map(|c| [0, 1, 2].map(|k| c[k] * ambient * tint[k]));
    (
        Lighting {
            sun,
            ambient: sh9,
            background,
        },
        look,
    )
}

fn blend((a, la): &(Lighting, Look), (b, lb): &(Lighting, Look), p: f32) -> (Lighting, Look) {
    let mix = |x: f32, y: f32| x + (y - x) * p;
    let mix3 = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|k| mix(x[k], y[k]));
    let sun = match (&a.sun, &b.sun) {
        (Some(x), Some(y)) => Some(Sun {
            direction: Vec3::from(x.direction)
                .lerp(Vec3::from(y.direction), p)
                .normalize_or(Vec3::Y)
                .into(),
            color: mix3(x.color, y.color),
            intensity: mix(x.intensity, y.intensity),
            ..if p < 0.5 { x.clone() } else { y.clone() }
        }),
        (Some(x), None) => Some(Sun {
            intensity: mix(x.intensity, 0.0),
            ..x.clone()
        }),
        (None, Some(y)) => Some(Sun {
            intensity: mix(0.0, y.intensity),
            ..y.clone()
        }),
        (None, None) => None,
    };
    let lighting = Lighting {
        sun,
        ambient: std::array::from_fn(|i| mix3(a.ambient[i], b.ambient[i])),
        background: Background {
            top: mix3(a.background.top, b.background.top),
            bottom: mix3(a.background.bottom, b.background.bottom),
        },
    };
    let look = Look {
        exposure: mix(la.exposure, lb.exposure),
        bloom_threshold: mix(la.bloom_threshold, lb.bloom_threshold),
        bloom_strength: mix(la.bloom_strength, lb.bloom_strength),
        tint: mix3(la.tint, lb.tint),
        real: mix(la.real, lb.real),
        night: mix(la.night, lb.night),
        tilt: mix(la.tilt, lb.tilt),
        focus: mix(la.focus, lb.focus),
        ao_strength: mix(la.ao_strength, lb.ao_strength),
        ao_radius_m: mix(la.ao_radius_m, lb.ao_radius_m),
    };
    (lighting, look)
}

/// Where a node's draws go: turned on its hinge by `degrees` from where
/// the pack stored them, then moved by `binding`.
fn node_offset(node: &NodeIndex, degrees: Option<f32>, binding: Option<Mat4>) -> Mat4 {
    let hinge = match (&node.hinge, degrees) {
        (Some(h), Some(angle)) if angle != h.angle_deg => {
            let pivot = Mat4::from_cols_array(&node.pivot);
            let turn =
                Quat::from_axis_angle(Vec3::from(h.axis), (angle - h.angle_deg).to_radians());
            pivot * Mat4::from_quat(turn) * pivot.inverse()
        }
        _ => Mat4::IDENTITY,
    };
    binding.unwrap_or(Mat4::IDENTITY) * hinge
}

fn sync(
    renderer: &mut Renderer,
    r: usize,
    view: &mut RoomView,
    scene: &RoomScene,
    scenery: &sariema_runtime::scenery::Scenery,
    world: &sariema_runtime::world::World,
    alpha: f32,
) {
    let slot = r as u32;
    let index = &scene.index;
    let applied = view.applied.get_or_insert_with(|| Applied {
        groups: index.groups.iter().map(|g| g.1).collect(),
        pieces: index.pieces.iter().map(|p| p.state).collect(),
        params: index.params.iter().map(|p| p.value).collect(),
        offsets: vec![Mat4::IDENTITY; index.nodes.len()],
        visible: index.nodes.iter().map(|n| n.visible).collect(),
        bound: vec![None; index.nodes.len()],
    });
    for (g, (&on, was)) in scene.groups.iter().zip(&mut applied.groups).enumerate() {
        if on != *was {
            renderer.set_group(slot, g, on);
            *was = on;
        }
    }
    for (p, (&state, was)) in scene.pieces.iter().zip(&mut applied.pieces).enumerate() {
        if state != *was {
            renderer.set_piece_state(slot, p, state);
            *was = state;
        }
    }
    let mut hinges_moved = false;
    for (k, (&value, was)) in scene.params.iter().zip(&mut applied.params).enumerate() {
        if value.to_bits() == was.to_bits() {
            continue;
        }
        *was = value;
        match index.params[k].target {
            Target::Light(l, _) => {
                let at = view.light_params[l as usize];
                let v = |i: usize| scene.params[at[i]];
                renderer.set_light_values(slot, l as usize, [v(0), v(1), v(2)], v(3), v(4));
            }
            Target::Hinge(_) => hinges_moved = true,
            _ => {}
        }
    }
    applied.bound.iter_mut().for_each(|b| *b = None);
    for (entity, binding) in scenery.bound(world) {
        let Some(pose) = world.interpolated(entity, alpha) else {
            continue;
        };
        let placed = Mat4::from_scale_rotation_translation(
            Vec3::splat(pose.scale),
            pose.rotation,
            pose.position,
        );
        let visible = world.node(entity).is_some_and(|n| n.visible);
        match binding {
            Binding::Node { room, node, start } if room == r => {
                applied.bound[node] = Some(placed * start.inverse());
                if visible != applied.visible[node] {
                    renderer.set_node_visible(slot, node, visible);
                    applied.visible[node] = visible;
                }
            }
            Binding::Light { room, light } if room == r => {
                renderer.set_light_position(slot, light, pose.position.into(), None);
            }
            _ => {}
        }
    }
    for (n, node) in index.nodes.iter().enumerate() {
        let binding = applied.bound[n];
        if !hinges_moved && binding.is_none() && applied.offsets[n] == Mat4::IDENTITY {
            continue;
        }
        let degrees =
            (node.hinge.as_ref()).map(|_| scene.value(Target::Hinge(n as u32)).unwrap_or(0.0));
        let offset = node_offset(node, degrees, binding);
        if offset != applied.offsets[n] {
            renderer.set_node_offset(slot, n, offset);
            applied.offsets[n] = offset;
        }
    }
}
