// SPDX-License-Identifier: Apache-2.0

//! Assembles a room pack from a kit, the game's placement file (`montagem.json`)
//! and its lighting file (`luz.json`).
//!
//! Placement, rotation, states and hinges (`giros`) follow the game's own
//! house builder. Each shown sub-room is a group of its draws
//! and lights; each copy of a loose piece is a node carrying every state of
//! the piece, and each of its hinged parts a node of its own.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use glam::{Mat3, Mat4, Quat, Vec3};
use sariema_format::{
    Aabb, AlphaMode, Ambient, Area, Background, CharacterLight, CutAnchor, CutLine, CutStyle,
    Cutaway, Draw, Fog, Group, Hinge, Light, LightKind, Look, Material, Mesh, Mobility, Node,
    Piece, PieceState, RoomCamera, RoomPack, Stats, Sun, Texture, Vertex, hemisphere_sh9,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::actor::matches;
use crate::collision::CollisionSource;
use crate::texture::srgb_to_linear;
use crate::{
    ImportError, Kit, KitHinge, KitPiece, KitPrimitive, TextureMode, encode_texture, load_kit, read,
};

const SCHEMA: &str = include_str!("../../../schemas/room.schema.json");

/// A room description, as `schemas/room.schema.json` defines it. `kit`,
/// `montagem` and `luz` are relative to the assets root.
#[derive(Debug, Deserialize)]
pub struct RoomSource {
    pub name: String,
    pub kit: PathBuf,
    pub montagem: PathBuf,
    pub luz: PathBuf,
    /// Sub-rooms (`comodos`) to include; the first gives the ambient light and the sun.
    pub show: Vec<String>,
    /// Pieces that keep their own draws instead of merging by material.
    pub loose: Vec<String>,
    /// Pieces that start hidden: loose ones as invisible draws, merged ones left out.
    #[serde(default)]
    pub hidden: Vec<String>,
    /// Sub-rooms of `show` whose group starts switched off.
    #[serde(default)]
    pub groups_off: Vec<String>,
    /// Lights, named `<room>.<nome>`, that cast no shadow.
    #[serde(default)]
    pub shadowless_lights: Vec<String>,
    pub drawn_materials: Vec<String>,
    pub rim_materials: Vec<(String, f32)>,
    pub additive_materials: Vec<String>,
    pub look: Look,
    pub camera: RoomCamera,
    pub background: Background,
    #[serde(default)]
    pub extra_hemisphere: Option<Hemisphere>,
    #[serde(default)]
    pub character_light: CharacterLight,
    /// Off when absent.
    #[serde(default)]
    pub fog: FogSource,
    /// ASTC 4×4 when absent.
    #[serde(default)]
    pub textures: Option<TextureMode>,
    /// What to bake into the room's collision pack; no pack when absent.
    #[serde(default)]
    pub collision: Option<CollisionSource>,
    #[serde(default)]
    pub cutaways: Vec<CutawaySource>,
}

/// A [`Cutaway`] over the pieces whose names match `pieces`.
#[derive(Debug, Deserialize)]
pub struct CutawaySource {
    pub name: String,
    /// `*` matches any run of characters.
    pub pieces: Vec<String>,
    pub height: f32,
    #[serde(default)]
    pub areas: Vec<Area>,
    #[serde(default)]
    pub offset: f32,
    #[serde(default = "CutawaySource::default_follow")]
    pub follow: f32,
    #[serde(default)]
    pub radius: f32,
    #[serde(default)]
    pub soft: f32,
    #[serde(default)]
    pub cap: Option<[f32; 3]>,
    #[serde(default = "CutawaySource::default_cut_shadows")]
    pub cut_shadows: bool,
    /// Metres below which a merged piece's triangles are drawn apart, never
    /// cut; the height when absent.
    #[serde(default)]
    pub whole_below: Option<f32>,
    #[serde(default)]
    pub style: StyleSource,
    /// Metres from a room's edge within which the point a piece is placed at makes it
    /// belong to that edge's wall: each part of it is cut where the wall behind it is.
    #[serde(default = "CutawaySource::default_attach")]
    pub attach: f32,
    /// Metres between the top of a piece and the bottom of one above it
    /// within which the first holds up the second.
    #[serde(default = "CutawaySource::default_rest")]
    pub rest: f32,
}

/// [`CutStyle`] as a room description names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StyleSource {
    #[default]
    Cut,
    Fade,
}

/// A room's floor across the engine's x and z.
#[derive(Clone, Copy)]
struct Floor {
    min: glam::Vec2,
    max: glam::Vec2,
}

impl Floor {
    /// From the room's south-west `corner` in metres and its size in cells.
    fn of(corner: [f32; 2], cells: [f32; 2], celula: f32) -> Floor {
        let far = [corner[0] + cells[0] * celula, corner[1] + cells[1] * celula];
        Floor {
            min: glam::Vec2::new(corner[0], -far[1]),
            max: glam::Vec2::new(far[0], -corner[1]),
        }
    }
}

/// A room's wall, by where its edge runs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Wall {
    AtX(f32),
    AtZ(f32),
}

impl Eq for Wall {}

impl Ord for Wall {
    fn cmp(&self, other: &Wall) -> std::cmp::Ordering {
        let key = |w: &Wall| match *w {
            Wall::AtX(x) => (0, x),
            Wall::AtZ(z) => (1, z),
        };
        let ((a, x), (b, y)) = (key(self), key(other));
        a.cmp(&b).then(x.total_cmp(&y))
    }
}

impl PartialOrd for Wall {
    fn partial_cmp(&self, other: &Wall) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The draws a room's merged pieces go to, with their material.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Merged {
    Room,
    Cut(usize),
    /// The part of a cutaway's pieces under where it cuts at build, kept
    /// apart so it is drawn uncut until the height comes down to it; by the
    /// wall they are on.
    Below(usize, Option<Wall>),
}

impl fmt::Display for Wall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Wall::AtX(x) => write!(f, "x{x}"),
            Wall::AtZ(z) => write!(f, "z{z}"),
        }
    }
}

impl Wall {
    fn anchor(self) -> CutAnchor {
        match self {
            Wall::AtX(x) => CutAnchor {
                at: [x, 0.0],
                along: [0.0, 1.0],
            },
            Wall::AtZ(z) => CutAnchor {
                at: [0.0, z],
                along: [1.0, 0.0],
            },
        }
    }
}

impl CutawaySource {
    /// Seconds: the line eases, so it never jumps.
    pub const DEFAULT_FOLLOW: f32 = 0.1;

    fn default_follow() -> f32 {
        CutawaySource::DEFAULT_FOLLOW
    }

    fn default_cut_shadows() -> bool {
        true
    }

    /// Metres: a piece on a wall stands within half a metre of its edge.
    pub const DEFAULT_ATTACH: f32 = 0.5;

    fn default_attach() -> f32 {
        CutawaySource::DEFAULT_ATTACH
    }

    pub const DEFAULT_REST: f32 = 0.02;

    fn default_rest() -> f32 {
        CutawaySource::DEFAULT_REST
    }
}

/// The room's [`Fog`]; its colour is the background's bottom when absent.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FogSource {
    pub color: Option<[f32; 3]>,
    pub density: f32,
    pub start: f32,
    pub end: f32,
}

/// A hemisphere light, added to the one luz.json gives the first shown room;
/// colours are linear RGB.
#[derive(Clone, Debug, Deserialize)]
pub struct Hemisphere {
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub intensity: f32,
}

impl RoomSource {
    /// Parses a room description after checking it against `schemas/room.schema.json`.
    pub fn from_json(json: &str) -> Result<RoomSource, ImportError> {
        let schema: Value = serde_json::from_str(SCHEMA).expect("room.schema.json is JSON");
        let value: Value = serde_json::from_str(json).map_err(schema_error)?;
        jsonschema::validate(&schema, &value).map_err(|e| {
            let at = e.instance_path().to_string();
            ImportError::Schema(if at.is_empty() {
                e.to_string()
            } else {
                format!("{at}: {e}")
            })
        })?;
        serde_json::from_value(value).map_err(schema_error)
    }
}

/// `Montagem` in `casa.ts`.
#[derive(Deserialize)]
pub(crate) struct Montagem {
    /// Metres per grid cell.
    pub(crate) celula: f32,
    pub(crate) comodos: HashMap<String, Comodo>,
}

#[derive(Deserialize)]
pub(crate) struct Comodo {
    /// The room's south-west corner, in cells.
    pub(crate) sw_celulas: [f32; 2],
    /// The room's size in cells, east and north.
    #[serde(default)]
    pub(crate) n_celulas: Option<[f32; 2]>,
    pub(crate) instancias: Vec<Inst>,
}

/// One piece placed in a room. Positions are metres from the room's corner on
/// Blender's axes: x east, y north, z up.
#[derive(Deserialize)]
pub(crate) struct Inst {
    pub(crate) peca: String,
    x: f32,
    y: f32,
    z: f32,
    /// Degrees about the vertical axis, used when `rot` is absent.
    rz: f32,
    /// Euler angles about Blender's axes, in degrees; see [`placement`].
    rot: Option<[f32; 3]>,
    pub(crate) estado: Option<String>,
    /// Degrees each hinge of the piece is turned, by hinge name.
    #[serde(default)]
    giros: Option<BTreeMap<String, f32>>,
    escala: Option<f32>,
}

/// `CasaLuz` in `casa.ts`.
#[derive(Deserialize)]
struct CasaLuz {
    comodos: HashMap<String, LuzComodo>,
}

#[derive(Deserialize)]
struct LuzComodo {
    hemi_ceu: String,
    hemi_chao: String,
    hemi_int: f32,
    sol: String,
    sol_int: f32,
    /// Towards the sun, already on the engine's axes.
    sol_dir_three: [f32; 3],
    sol_sombra: Option<bool>,
    luzes: Vec<LuzItem>,
}

/// `pos` and `alvo` are placed like [`Inst`] positions.
#[derive(Deserialize)]
struct LuzItem {
    nome: String,
    tipo: Tipo,
    cor: String,
    /// Candela.
    int: f32,
    pos: [f32; 3],
    alvo: Option<[f32; 3]>,
    /// Full opening angle, in degrees.
    cone: Option<f32>,
    dist: f32,
    tremido: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Tipo {
    Spot,
    Ponto,
}

/// Builds the sub-rooms listed in `source.show` into one pack.
pub fn build_room(source: &RoomSource, assets_root: &Path) -> Result<RoomPack, ImportError> {
    let mut kit = load_kit(&assets_root.join(&source.kit))?;
    let montagem: Montagem = read_json(&assets_root.join(&source.montagem))?;
    let luz: CasaLuz = read_json(&assets_root.join(&source.luz))?;
    dress(&mut kit.materials, source);
    if let Some(off) = source.groups_off.iter().find(|g| !source.show.contains(g)) {
        return Err(ImportError::Schema(format!(
            "room {}: groups_off lists {off}, which show does not",
            source.name
        )));
    }

    let mut scene = Scene {
        kit: &kit,
        loose: &source.loose,
        hidden: &source.hidden,
        cutaways: &source.cutaways,
        celula: 0.0,
        meshes: Vec::new(),
        draws: Vec::new(),
        nodes: Vec::new(),
        pieces: Vec::new(),
        cut_draws: vec![Vec::new(); source.cutaways.len()],
        matched: (source.cutaways.iter())
            .map(|c| vec![false; c.pieces.len()])
            .collect(),
    };
    let mut lights = Vec::new();
    let mut groups = Vec::new();
    for room in &source.show {
        let comodo = montagem
            .comodos
            .get(room)
            .ok_or_else(|| ImportError::Schema(format!("montagem has no room {room}")))?;
        let corner = comodo.sw_celulas.map(|cells| cells * montagem.celula);
        let first_draw = scene.draws.len() as u32;
        scene.celula = montagem.celula;
        scene.add_room(room, comodo, corner)?;
        let first_light = lights.len() as u32;
        for item in luz.comodos.get(room).map_or(&[][..], |l| &l.luzes) {
            let mut light = light(room, item, corner)?;
            light.casts_shadow &= !source.shadowless_lights.contains(&light.name);
            lights.push(light);
        }
        groups.push(Group {
            name: room.clone(),
            on: !source.groups_off.contains(room),
            draws: (first_draw..scene.draws.len() as u32).collect(),
            lights: (first_light..lights.len() as u32).collect(),
        });
    }
    let Scene {
        meshes,
        draws,
        nodes,
        pieces,
        cut_draws,
        matched,
        ..
    } = scene;
    for (c, matched) in source.cutaways.iter().zip(&matched) {
        if let Some((pattern, _)) = c.pieces.iter().zip(matched).find(|(_, m)| !**m) {
            return Err(ImportError::Schema(format!(
                "room {}: cutaway {} lists {pattern}, which no placed piece matches",
                source.name, c.name
            )));
        }
    }
    let cutaways = (source.cutaways.iter().zip(cut_draws))
        .map(|(c, draws)| Cutaway {
            name: c.name.clone(),
            anchors: draws.iter().map(|d| d.1).collect(),
            draws: draws.iter().map(|d| d.0).collect(),
            style: match c.style {
                StyleSource::Cut => CutStyle::Cut,
                StyleSource::Fade => CutStyle::Fade,
            },
            height: c.height,
            line: CutLine {
                areas: c.areas.clone(),
                offset: c.offset,
                follow: c.follow,
            },
            radius: c.radius,
            soft: c.soft,
            cap: c.cap,
            cut_shadows: c.cut_shadows,
        })
        .collect();
    let placed = |piece: &String| {
        source
            .show
            .iter()
            .filter_map(|room| montagem.comodos.get(room))
            .any(|comodo| comodo.instancias.iter().any(|inst| inst.peca == *piece))
    };
    if let Some(piece) = source.hidden.iter().find(|piece| !placed(piece)) {
        return Err(ImportError::HiddenPieceMissing {
            piece: piece.clone(),
            room: source.name.clone(),
        });
    }
    let lit = |name: &String| lights.iter().any(|l| l.name == *name);
    if let Some(name) = source.shadowless_lights.iter().find(|name| !lit(name)) {
        return Err(ImportError::Schema(format!(
            "room {}: shadowless_lights lists {name}, which no room in show lights",
            source.name
        )));
    }

    let first = source
        .show
        .first()
        .ok_or_else(|| ImportError::Schema("show lists no room".into()))?;
    let fallback = fallback_sky();
    let sky = luz.comodos.get(first).unwrap_or(&fallback);
    let sun_direction = Vec3::from(sky.sol_dir_three)
        .try_normalize()
        .ok_or_else(|| {
            ImportError::Schema(format!("luz: room {first}: sol_dir_three has no direction"))
        })?;
    let mut sh9 = hemisphere_sh9(
        linear_rgb(&sky.hemi_ceu)?,
        linear_rgb(&sky.hemi_chao)?,
        sky.hemi_int,
    );
    if let Some(extra) = &source.extra_hemisphere {
        // Light adds, and so do the coefficients of a linear projection.
        let added = hemisphere_sh9(extra.sky, extra.ground, extra.intensity);
        for (sum, add) in sh9.iter_mut().flatten().zip(added.iter().flatten()) {
            *sum += add;
        }
    }
    let mode = source.textures.unwrap_or_default();
    let textures: Vec<Texture> = kit
        .images
        .iter()
        .map(|image| encode_texture(&image.name, &image.rgba, mode))
        .collect::<Result<_, _>>()?;
    let mut room = RoomPack {
        name: source.name.clone(),
        stats: Stats::of(&[], &[], &[]),
        meshes,
        materials: kit.materials,
        textures,
        draws,
        lights,
        ambient: Ambient { sh9 },
        sun: Some(Sun {
            direction: sun_direction.into(),
            color: linear_rgb(&sky.sol)?,
            intensity: sky.sol_int,
            casts_shadow: sky.sol_sombra.unwrap_or(true),
            mobility: Mobility::Static,
        }),
        background: source.background.clone(),
        look: source.look.clone(),
        camera: source.camera.clone(),
        groups,
        nodes,
        pieces,
        character: source.character_light.clone(),
        fog: Fog {
            color: source.fog.color.unwrap_or(source.background.bottom),
            density: source.fog.density,
            start: source.fog.start,
            end: source.fog.end,
        },
        cutaways,
    };
    room.stats = Stats::of_room(&room);
    Ok(room)
}

/// How casa.ts lights a room that luz.json does not list.
fn fallback_sky() -> LuzComodo {
    LuzComodo {
        hemi_ceu: "#CFE0F0".into(),
        hemi_chao: "#8A7F70".into(),
        hemi_int: 0.9,
        sol: "#FFFFFF".into(),
        sol_int: 1.4,
        sol_dir_three: [-0.5, 1.0, 0.4],
        sol_sombra: None,
        luzes: Vec::new(),
    }
}

/// Applies the description's per-material settings, matched by name.
fn dress(materials: &mut [Material], source: &RoomSource) {
    for material in materials {
        let named = |names: &[String]| names.contains(&material.name);
        material.drawn = named(&source.drawn_materials);
        if named(&source.additive_materials) {
            material.alpha = AlphaMode::Additive;
        }
        if let Some((_, rim)) = source
            .rim_materials
            .iter()
            .find(|(name, _)| *name == material.name)
        {
            material.rim = *rim;
        }
    }
}

struct Scene<'a> {
    kit: &'a Kit,
    loose: &'a [String],
    hidden: &'a [String],
    cutaways: &'a [CutawaySource],
    /// Metres per cell of the placement being built.
    celula: f32,
    meshes: Vec<Mesh>,
    draws: Vec<Draw>,
    nodes: Vec<Node>,
    pieces: Vec<Piece>,
    /// By cutaway, its draws with their anchors, and whether each of its
    /// patterns matched a piece.
    cut_draws: Vec<Vec<(u32, Option<CutAnchor>)>>,
    matched: Vec<Vec<bool>>,
}

impl Scene<'_> {
    /// The cutaway whose patterns match `piece`, if any.
    fn cutaway_of(&mut self, piece: &str, room: &str) -> Result<Option<usize>, ImportError> {
        let mut found: Option<usize> = None;
        for (c, cutaway) in self.cutaways.iter().enumerate() {
            let mut hit = false;
            for (pattern, matched) in cutaway.pieces.iter().zip(&mut self.matched[c]) {
                if matches(pattern, piece) {
                    *matched = true;
                    hit = true;
                }
            }
            match found {
                Some(first) if hit => {
                    return Err(ImportError::Schema(format!(
                        "room {room}: piece {piece} is in cutaways {} and {}",
                        self.cutaways[first].name, cutaway.name
                    )));
                }
                None if hit => found = Some(c),
                _ => {}
            }
        }
        Ok(found)
    }

    /// Adds one draw per material for the room's merged pieces, and for each
    /// copy of a loose piece a node with a draw per primitive of each of its
    /// looks, and a node per hinged part.
    fn add_room(
        &mut self,
        room: &str,
        comodo: &Comodo,
        corner: [f32; 2],
    ) -> Result<(), ImportError> {
        // By cutaway, if any, then material: the pieces of none come first.
        let mut merged: BTreeMap<(Merged, u32), Mesh> = BTreeMap::new();
        let mut copies: HashMap<&str, u32> = HashMap::new();
        let floor = comodo.n_celulas.map(|n| Floor::of(corner, n, self.celula));
        let cuts = self.cuts_of(room, comodo, corner, floor)?;
        for (inst, &cut) in comodo.instancias.iter().zip(&cuts) {
            let piece = self
                .kit
                .piece(&inst.peca)
                .ok_or_else(|| ImportError::PieceMissing {
                    piece: inst.peca.clone(),
                    room: room.to_owned(),
                })?;
            let placement = placement(inst, corner, room)?;
            let turns = hinge_turns(piece, inst, room)?;
            // As casa.ts treats nodes flagged oculto_no_inicio: hidden only while no estado is set.
            let hidden_at_start = piece.hidden_at_start && inst.estado.is_none();
            let hidden = self.hidden.contains(&inst.peca);
            let origin = placement.transform_point3(Vec3::ZERO);
            let first = self.draws.len() as u32;
            let copy = copies.entry(&inst.peca).or_default();
            *copy += 1;
            let name = match *copy {
                1 => format!("{room}.{}", inst.peca),
                n => format!("{room}.{}.{n}", inst.peca),
            };
            if self.loose.contains(&inst.peca) {
                self.add_loose(&name, piece, inst, room, placement, !hidden)?;
                for (hinge, angle) in piece.hinges.iter().zip(&turns) {
                    let pivot = placement * hinge.pivot;
                    let first = self.draws.len() as u32;
                    let turned = placement * turn(hinge, *angle);
                    let shown = !hidden && !hidden_at_start;
                    for p in &hinge.primitives {
                        self.push(name.clone() + "_" + &hinge.name, p, turned, true);
                    }
                    self.nodes.push(Node {
                        name: format!("{name}_{}", hinge.name),
                        pivot: pivot.to_cols_array(),
                        draws: (first..self.draws.len() as u32).collect(),
                        visible: shown,
                        hinge: Some(Hinge {
                            axis: hinge.axis.into(),
                            angle_deg: *angle,
                        }),
                    });
                }
                if let Some(c) = cut {
                    let draws: Vec<u32> = (first..self.draws.len() as u32).collect();
                    let bounds = self.bounds(&draws);
                    let anchor = bounds.and(self.wall_of(c, floor, origin)).map(Wall::anchor);
                    self.cut_draws[c].extend(draws.into_iter().map(|d| (d, anchor)));
                }
            } else if !hidden && !hidden_at_start {
                let primitives = instance_primitives(piece, inst, room)?;
                let Some(c) = cut else {
                    for p in primitives {
                        let mesh = merged.entry((Merged::Room, p.material)).or_default();
                        append(mesh, &p.mesh, placement);
                    }
                    continue;
                };
                let cutaway = &self.cutaways[c];
                // A dissolve takes the piece whole; a cut never reaches below its height.
                let below = match cutaway.style {
                    StyleSource::Fade => f32::NEG_INFINITY,
                    StyleSource::Cut => cutaway.whole_below.unwrap_or(cutaway.height),
                };
                let placed: Vec<KitPrimitive> = (primitives.iter())
                    .map(|p| {
                        let mut mesh = Mesh::default();
                        append(&mut mesh, &p.mesh, placement);
                        KitPrimitive {
                            mesh,
                            material: p.material,
                        }
                    })
                    .collect();
                let identity = Mat4::IDENTITY.to_cols_array();
                let bounds = (placed.iter())
                    .filter(|p| !p.mesh.indices.is_empty())
                    .map(|p| Aabb::around(&p.mesh, &identity))
                    .map(|b| (Vec3::from(b.min), Vec3::from(b.max)))
                    .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)));
                let wall = bounds.and(self.wall_of(c, floor, origin));
                for p in placed {
                    let (low, high) = split_below(&p.mesh, below);
                    append(
                        merged
                            .entry((Merged::Below(c, wall), p.material))
                            .or_default(),
                        &low,
                        Mat4::IDENTITY,
                    );
                    if wall.is_none() {
                        let mesh = merged.entry((Merged::Cut(c), p.material)).or_default();
                        append(mesh, &high, Mat4::IDENTITY);
                        continue;
                    }
                    let at = self.draws.len() as u32;
                    let name = format!("{room}/{}/{}", cutaway.name, &name[room.len() + 1..]);
                    let high = KitPrimitive {
                        mesh: high,
                        material: p.material,
                    };
                    self.push(name, &high, Mat4::IDENTITY, true);
                    if self.draws.len() as u32 > at {
                        self.cut_draws[c].push((at, wall.map(Wall::anchor)));
                    }
                }
            }
        }
        for ((into, material), mesh) in merged {
            let material_name = &self.kit.materials[material as usize].name;
            let cutaway = |c: usize| &self.cutaways[c].name;
            let (name, cut) = match into {
                Merged::Room => (format!("{room}/{material_name}"), None),
                Merged::Cut(c) => (
                    format!("{room}/{}/{material_name}", cutaway(c)),
                    Some((c, None)),
                ),
                Merged::Below(c, None) => (
                    format!("{room}/{}/below/{material_name}", cutaway(c)),
                    Some((c, None)),
                ),
                Merged::Below(c, Some(wall)) => (
                    format!("{room}/{}/below/{wall}/{material_name}", cutaway(c)),
                    Some((c, Some(wall.anchor()))),
                ),
            };
            let primitive = KitPrimitive { mesh, material };
            let at = self.draws.len() as u32;
            self.push(name, &primitive, Mat4::IDENTITY, true);
            if let (Some((c, anchor)), true) = (cut, self.draws.len() as u32 > at) {
                self.cut_draws[c].push((at, anchor));
            }
        }
        Ok(())
    }

    /// The bounds of `draws`, if any has some.
    fn bounds(&self, draws: &[u32]) -> Option<(Vec3, Vec3)> {
        (draws.iter())
            .map(|&d| &self.draws[d as usize].bounds)
            .map(|b| (Vec3::from(b.min), Vec3::from(b.max)))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// By placed piece of the room, the cutaway it is in: the one whose
    /// patterns match it or, matched by none, the one of its walls it hangs
    /// on. A piece hangs on a cutaway's wall when the room holds pieces of
    /// that cutaway, it is placed within its `attach` of an edge, lies
    /// wholly above the part kept whole, and nothing that stays standing
    /// holds it up: left out, it would float over its wall cut away. A
    /// piece holds up another when its top is within the cutaway's `rest`
    /// of the other's bottom, under it.
    fn cuts_of(
        &mut self,
        room: &str,
        comodo: &Comodo,
        corner: [f32; 2],
        floor: Option<Floor>,
    ) -> Result<Vec<Option<usize>>, ImportError> {
        let instances = &comodo.instancias;
        let mut cuts = Vec::with_capacity(instances.len());
        for inst in instances {
            cuts.push(self.cutaway_of(&inst.peca, room)?);
        }
        let held: Vec<bool> = (0..self.cutaways.len())
            .map(|c| cuts.contains(&Some(c)))
            .collect();
        if !held.contains(&true) {
            return Ok(cuts);
        }
        let mut placed = Vec::with_capacity(instances.len());
        for inst in instances {
            let placement = placement(inst, corner, room)?;
            let bounds = match (self.kit.piece(&inst.peca), self.hidden.contains(&inst.peca)) {
                (Some(piece), false) => {
                    let matrix = placement.to_cols_array();
                    (instance_primitives(piece, inst, room)?.iter())
                        .filter(|p| !p.mesh.indices.is_empty())
                        .map(|p| Aabb::around(&p.mesh, &matrix))
                        .map(|b| (Vec3::from(b.min), Vec3::from(b.max)))
                        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                }
                _ => None,
            };
            placed.push((placement.transform_point3(Vec3::ZERO), bounds));
        }
        let under = |a: (Vec3, Vec3), b: (Vec3, Vec3), rest: f32| {
            let across = (0..3)
                .step_by(2)
                .all(|k| a.0[k] <= b.1[k] && b.0[k] <= a.1[k]);
            across && (a.0.y - b.1.y).abs() <= rest
        };
        loop {
            let mut changed = false;
            for (a, &(origin, bounds)) in placed.iter().enumerate() {
                let Some(bounds) = bounds.filter(|_| cuts[a].is_none()) else {
                    continue;
                };
                let hangs = |c: usize| {
                    let cutaway = &self.cutaways[c];
                    let whole = match cutaway.style {
                        StyleSource::Fade => cutaway.height,
                        StyleSource::Cut => cutaway.whole_below.unwrap_or(cutaway.height),
                    };
                    let standing = (placed.iter().enumerate()).any(|(b, &(_, other))| {
                        let holds = other.is_some_and(|o| under(bounds, o, cutaway.rest));
                        b != a && holds && cuts[b].is_none()
                    });
                    held[c]
                        && bounds.0.y >= whole
                        && self.wall_of(c, floor, origin).is_some()
                        && !standing
                };
                if let Some(c) = (0..self.cutaways.len()).find(|&c| hangs(c)) {
                    cuts[a] = Some(c);
                    changed = true;
                }
            }
            if !changed {
                return Ok(cuts);
            }
        }
    }

    /// The wall of the room's `floor` a piece of cutaway `c` placed at
    /// `centre` belongs to: the edge it stands nearest, within the cutaway's
    /// `attach`. Where it is placed, not its bounds: a door's open leaf
    /// swings away from its wall. Without the room's floor, none.
    fn wall_of(&self, c: usize, floor: Option<Floor>, centre: Vec3) -> Option<Wall> {
        let floor = floor?;
        let edges = [
            (Wall::AtZ(floor.max.y), (centre.z - floor.max.y).abs()),
            (Wall::AtZ(floor.min.y), (centre.z - floor.min.y).abs()),
            (Wall::AtX(floor.min.x), (centre.x - floor.min.x).abs()),
            (Wall::AtX(floor.max.x), (centre.x - floor.max.x).abs()),
        ];
        let (wall, distance) = edges.into_iter().min_by(|a, b| a.1.total_cmp(&b.1))?;
        (distance <= self.cutaways[c].attach).then_some(wall)
    }

    /// A loose copy `name`: a node over its draws, and a piece over its
    /// looks when it has states. A piece hidden at start shows nothing until
    /// a state is set: with states its start look is empty, without them
    /// the node starts hidden.
    fn add_loose(
        &mut self,
        name: &str,
        piece: &KitPiece,
        inst: &Inst,
        room: &str,
        placement: Mat4,
        shown: bool,
    ) -> Result<(), ImportError> {
        let first = self.draws.len() as u32;
        let stateless = piece.states.is_empty();
        let shown = shown && !(stateless && piece.hidden_at_start && inst.estado.is_none());
        let start: &[KitPrimitive] = if piece.hidden_at_start && !stateless {
            &[]
        } else {
            &piece.primitives
        };
        let mut looks = vec![(String::new(), start)];
        looks.extend(piece.states.iter().map(|(n, p)| (n.clone(), p.as_slice())));
        if stateless {
            for p in start {
                self.push(name.to_owned(), p, placement, true);
            }
        } else {
            let state = match &inst.estado {
                None => 0,
                Some(estado) => looks.iter().position(|(n, _)| n == estado).ok_or_else(|| {
                    ImportError::Schema(format!(
                        "room {room}: piece {} has no state {estado}",
                        piece.name
                    ))
                })?,
            };
            let mut states = Vec::with_capacity(looks.len());
            for (look, primitives) in looks {
                let from = self.draws.len() as u32;
                for p in primitives {
                    self.push(name.to_owned(), p, placement, true);
                }
                states.push(PieceState {
                    name: look,
                    draws: (from..self.draws.len() as u32).collect(),
                });
            }
            self.pieces.push(Piece {
                name: name.to_owned(),
                states,
                state: state as u32,
            });
        }
        self.nodes.push(Node {
            name: name.to_owned(),
            pivot: placement.to_cols_array(),
            draws: (first..self.draws.len() as u32).collect(),
            visible: shown,
            hinge: None,
        });
        Ok(())
    }

    fn push(&mut self, name: String, p: &KitPrimitive, transform: Mat4, visible: bool) {
        // A mesh with no triangles would have no bounds.
        if p.mesh.indices.is_empty() {
            return;
        }
        let transform = transform.to_cols_array();
        self.draws.push(Draw {
            name: Some(name),
            mesh: self.meshes.len() as u32,
            material: p.material,
            bounds: Aabb::around(&p.mesh, &transform),
            transform,
            visible,
            // Additive surfaces (light cones, glows) add light and block none.
            casts_shadow: self.kit.materials[p.material as usize].alpha != AlphaMode::Additive,
        });
        self.meshes.push(p.mesh.clone());
    }
}

/// Each of the piece's hinges' angle in degrees, as the instance's `giros`
/// sets it (0 when it does not).
fn hinge_turns(piece: &KitPiece, inst: &Inst, room: &str) -> Result<Vec<f32>, ImportError> {
    let giros = inst.giros.as_ref();
    if let Some(unknown) = giros
        .into_iter()
        .flat_map(|g| g.keys())
        .find(|g| !piece.hinges.iter().any(|h| h.name == **g))
    {
        return Err(ImportError::Schema(format!(
            "room {room}: piece {} has no hinge {unknown}",
            piece.name
        )));
    }
    Ok((piece.hinges.iter())
        .map(|h| giros.and_then(|g| g.get(&h.name)).copied().unwrap_or(0.0))
        .collect())
}

/// What turning `hinge` by `degrees` does in its piece's root space, as
/// `applyGiro` turns the node from its rest pose about its own axis.
fn turn(hinge: &KitHinge, degrees: f32) -> Mat4 {
    let spin = Quat::from_axis_angle(hinge.axis, degrees.to_radians());
    hinge.pivot * Mat4::from_quat(spin) * hinge.pivot.inverse()
}

/// The primitives the instance shows, its hinged parts turned as its
/// `giros` say, in the piece's root space.
pub(crate) fn instance_primitives(
    piece: &KitPiece,
    inst: &Inst,
    room: &str,
) -> Result<Vec<KitPrimitive>, ImportError> {
    let mut out: Vec<KitPrimitive> = (shown_primitives(piece, inst, room)?.iter())
        .map(|p| KitPrimitive {
            mesh: p.mesh.clone(),
            material: p.material,
        })
        .collect();
    let turns = hinge_turns(piece, inst, room)?;
    for (hinge, angle) in piece.hinges.iter().zip(turns) {
        let moved = turn(hinge, angle);
        for p in &hinge.primitives {
            let mut mesh = Mesh {
                vertices: Vec::new(),
                indices: Vec::new(),
            };
            append(&mut mesh, &p.mesh, moved);
            out.push(KitPrimitive {
                mesh,
                material: p.material,
            });
        }
    }
    Ok(out)
}

/// The primitives the instance's `estado` shows, or the piece's start ones.
pub(crate) fn shown_primitives<'k>(
    piece: &'k KitPiece,
    inst: &Inst,
    room: &str,
) -> Result<&'k [KitPrimitive], ImportError> {
    let Some(state) = &inst.estado else {
        return Ok(&piece.primitives);
    };
    piece
        .states
        .iter()
        .find(|(name, _)| name == state)
        .map(|(_, primitives)| primitives.as_slice())
        .ok_or_else(|| {
            ImportError::Schema(format!(
                "room {room}: piece {} has no state {state}",
                piece.name
            ))
        })
}

/// A point given on Blender's axes from a room's corner, on the engine's axes.
fn engine_point(corner: [f32; 2], [x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(corner[0] + x, z, -(corner[1] + y))
}

pub(crate) fn placement(inst: &Inst, corner: [f32; 2], room: &str) -> Result<Mat4, ImportError> {
    // casa.ts reads `escala || 1`.
    let scale = inst.escala.filter(|&s| s != 0.0).unwrap_or(1.0);
    if !(scale.is_finite() && scale > 0.0) {
        return Err(ImportError::Schema(format!(
            "room {room}: piece {} has escala {scale}, which is not a positive number",
            inst.peca
        )));
    }
    let rotation = match inst.rot {
        Some([x, y, z]) => {
            // B · Rx·Ry·Rz · Bᵀ, as casa.ts does, with B taking Blender axes to the engine's.
            let b = Mat3::from_cols(Vec3::X, Vec3::NEG_Z, Vec3::Y);
            b * Mat3::from_rotation_x(x.to_radians())
                * Mat3::from_rotation_y(y.to_radians())
                * Mat3::from_rotation_z(z.to_radians())
                * b.transpose()
        }
        None => Mat3::from_rotation_y(inst.rz.to_radians()),
    };
    let position = engine_point(corner, [inst.x, inst.y, inst.z]);
    Ok(Mat4::from_translation(position) * Mat4::from_mat3(rotation * scale))
}

/// The triangles lying wholly at or below `y`, and the rest.
fn split_below(mesh: &Mesh, y: f32) -> (Mesh, Mesh) {
    let (mut low, mut high) = (Mesh::default(), Mesh::default());
    let mut at = [
        vec![None; mesh.vertices.len()],
        vec![None; mesh.vertices.len()],
    ];
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let under = triangle
            .iter()
            .all(|&i| mesh.vertices[i as usize].pos[1] <= y);
        let (into, at) = match under {
            true => (&mut low, &mut at[0]),
            false => (&mut high, &mut at[1]),
        };
        for &i in triangle {
            let new = *at[i as usize].get_or_insert_with(|| {
                into.vertices.push(mesh.vertices[i as usize]);
                into.vertices.len() as u32 - 1
            });
            into.indices.push(new);
        }
    }
    (low, high)
}

/// Appends `mesh`, moved by `transform`, to `into`.
fn append(into: &mut Mesh, mesh: &Mesh, transform: Mat4) {
    let normals = Mat3::from_mat4(transform).inverse().transpose();
    let base = into.vertices.len() as u32;
    into.vertices.extend(mesh.vertices.iter().map(|v| Vertex {
        pos: transform.transform_point3(v.pos.into()).into(),
        normal: (normals * Vec3::from(v.normal)).normalize_or_zero().into(),
        ..*v
    }));
    into.indices.extend(mesh.indices.iter().map(|i| base + i));
}

fn light(room: &str, item: &LuzItem, corner: [f32; 2]) -> Result<Light, ImportError> {
    let kind = match item.tipo {
        // casa.ts aims a spot without `alvo` straight down and opens it 60°.
        Tipo::Spot => LightKind::Spot {
            target: engine_point(corner, item.alvo.unwrap_or([item.pos[0], item.pos[1], 0.0]))
                .into(),
            cone_deg: item.cone.unwrap_or(60.0),
        },
        Tipo::Ponto => LightKind::Point,
    };
    Ok(Light {
        name: format!("{room}.{}", item.nome),
        // luz.json has no shadow flag for local lights.
        casts_shadow: matches!(kind, LightKind::Spot { .. }),
        kind,
        color: linear_rgb(&item.cor)?,
        intensity: item.int,
        position: engine_point(corner, item.pos).into(),
        range: item.dist,
        flicker: item.tremido.clone(),
        mobility: Mobility::Static,
        importance: 1.0,
    })
}

/// `#RRGGBB` in sRGB, as linear RGB.
fn linear_rgb(hex: &str) -> Result<[f32; 3], ImportError> {
    let value = hex
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .ok_or_else(|| ImportError::Schema(format!("{hex} is not a #RRGGBB colour")))?;
    let [_, r, g, b] = value.to_be_bytes();
    Ok([r, g, b].map(srgb_to_linear))
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ImportError> {
    serde_json::from_slice(&read(path)?)
        .map_err(|e| ImportError::Schema(format!("{}: {e}", path.display())))
}

fn schema_error(e: impl fmt::Display) -> ImportError {
    ImportError::Schema(e.to_string())
}
