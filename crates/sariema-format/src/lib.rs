// SPDX-License-Identifier: Apache-2.0

//! The Sariema room pack: everything the renderer needs to draw one room.
//!
//! A pack is the 4-byte [`MAGIC`], the [`VERSION`] as a little-endian `u32`,
//! then a [postcard](https://docs.rs/postcard) body holding a [`RoomPack`].
//!
//! Conventions: right-handed, Y up, north is −Z, metres; matrices are
//! column-major. Light and material colours are linear; colour textures are
//! sRGB.

use std::fmt;

use serde::{Deserialize, Serialize};

pub mod actor;
pub mod collision;
#[cfg(feature = "demo")]
pub mod demo;
pub mod scenery;
pub mod script;
pub mod sound;

pub const MAGIC: [u8; 4] = *b"SRMA";
pub const VERSION: u32 = 6;

const HEADER_LEN: usize = MAGIC.len() + size_of::<u32>();

#[repr(C)]
#[derive(
    Clone, Copy, Debug, PartialEq, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable,
)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Vertex colour, multiplies the material base colour.
    pub color: [u8; 4],
}

const _: () = assert!(size_of::<Vertex>() == 36);

/// Indexed triangle list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shading {
    Lit,
    Unlit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaMode {
    Opaque,
    Blend,
    /// Added over what is behind, blocking nothing.
    Additive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    /// Index into [`RoomPack::textures`].
    pub base_color_texture: Option<u32>,
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    pub roughness: f32,
    pub metallic: f32,
    /// Linear RGB, may exceed 1.
    pub emissive: [f32; 3],
    /// Index into [`RoomPack::textures`], multiplying `emissive`. It is mapped
    /// with the same `uv_offset` and `uv_scale` as the base colour texture.
    pub emissive_texture: Option<u32>,
    pub shading: Shading,
    pub alpha: AlphaMode,
    /// Back faces are drawn too, shaded from their own side (glTF `doubleSided`).
    pub double_sided: bool,
    /// Marks the "drawn" look, which colour grading leaves untouched.
    pub drawn: bool,
    /// Rim light strength; 0 turns it off.
    pub rim: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureFormat {
    Rgba8Srgb,
    Astc4x4Srgb,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    /// Mip levels, largest first.
    pub mips: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draw {
    pub name: Option<String>,
    /// Index into [`RoomPack::meshes`].
    pub mesh: u32,
    /// Index into [`RoomPack::materials`].
    pub material: u32,
    /// Model matrix, column-major.
    pub transform: [f32; 16],
    pub visible: bool,
    pub casts_shadow: bool,
    /// World-space bounds of the mesh after `transform`.
    pub bounds: Aabb,
}

/// Axis-aligned box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    /// Bounds of `mesh`'s vertices moved by the column-major `transform`. A
    /// mesh without vertices gives `min > max`, which [`decode`] rejects.
    pub fn around(mesh: &Mesh, transform: &[f32; 16]) -> Aabb {
        let m = transform;
        let empty = Aabb {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        mesh.vertices.iter().fold(empty, |b, v| {
            let [x, y, z] = v.pos;
            let p: [f32; 3] =
                std::array::from_fn(|r| m[r] * x + m[4 + r] * y + m[8 + r] * z + m[12 + r]);
            Aabb {
                min: std::array::from_fn(|i| b.min[i].min(p[i])),
                max: std::array::from_fn(|i| b.max[i].max(p[i])),
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LightKind {
    /// `cone_deg` is the full opening angle of the cone.
    Spot {
        target: [f32; 3],
        cone_deg: f32,
    },
    Point,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub name: String,
    pub kind: LightKind,
    /// Linear RGB.
    pub color: [f32; 3],
    /// Candela.
    pub intensity: f32,
    pub position: [f32; 3],
    pub range: f32,
    pub flicker: Option<String>,
    pub casts_shadow: bool,
    pub mobility: Mobility,
    /// How much the renderer should favour this light when it must choose
    /// between lights; 1.0 is neutral. Finite and at least 0.
    pub importance: f32,
}

/// Whether a light may move or turn at run time. Flicker changes only
/// intensity, so a flickering light can be static.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mobility {
    Static,
    Dynamic,
}

/// Ambient light as 9 SH coefficients; see [`eval_sh9`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ambient {
    pub sh9: [[f32; 3]; 9],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sun {
    /// Points towards the light.
    pub direction: [f32; 3],
    /// Linear RGB.
    pub color: [f32; 3],
    pub intensity: f32,
    pub casts_shadow: bool,
    pub mobility: Mobility,
}

/// Vertical gradient drawn where no geometry is, in linear RGB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Background {
    pub top: [f32; 3],
    pub bottom: [f32; 3],
}

/// Post-processing parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Look {
    pub exposure: f32,
    pub bloom_threshold: f32,
    pub bloom_strength: f32,
    pub tint: [f32; 3],
    pub real: f32,
    pub night: f32,
    pub tilt: f32,
    pub focus: f32,
    /// How much of the light screen-space ambient occlusion takes away from
    /// fully occluded creases, in [0, 1]; 0 turns it off.
    #[serde(default = "Look::default_ao_strength")]
    pub ao_strength: f32,
    /// How far, in metres, a surface occludes the light reaching another.
    #[serde(default = "Look::default_ao_radius_m")]
    pub ao_radius_m: f32,
}

impl Look {
    /// What a room description that leaves `ao_strength` out gets.
    pub const DEFAULT_AO_STRENGTH: f32 = 0.5;
    /// What a room description that leaves `ao_radius_m` out gets.
    pub const DEFAULT_AO_RADIUS_M: f32 = 0.5;

    fn default_ao_strength() -> f32 {
        Look::DEFAULT_AO_STRENGTH
    }

    fn default_ao_radius_m() -> f32 {
        Look::DEFAULT_AO_RADIUS_M
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomCamera {
    pub fov_y_deg: f32,
    pub pitch_deg: f32,
    pub distance: f32,
    pub focus: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub triangles: u32,
    pub draws: u32,
    pub texture_bytes: u64,
}

impl Stats {
    /// Triangles and number of the visible draws, and bytes of every texture
    /// mip. Panics if a draw's mesh index is out of range.
    pub fn of(meshes: &[Mesh], textures: &[Texture], draws: &[Draw]) -> Stats {
        let visible = draws.iter().filter(|d| d.visible);
        Stats {
            triangles: visible
                .clone()
                .map(|d| meshes[d.mesh as usize].indices.len() as u32 / 3)
                .sum(),
            draws: visible.count() as u32,
            texture_bytes: textures
                .iter()
                .flat_map(|t| &t.mips)
                .map(|m| m.len() as u64)
                .sum(),
        }
    }
}

/// Draws and lights switched on and off together, such as one room of a
/// house. A draw or light belongs to at most one group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Whether it is on when the room loads.
    pub on: bool,
    /// Indices into [`RoomPack::draws`].
    pub draws: Vec<u32>,
    /// Indices into [`RoomPack::lights`].
    pub lights: Vec<u32>,
}

/// A named part of the room a game can show, hide, move or turn, drawn by
/// its draws. A draw belongs to at most one node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    /// The node's origin, column-major: moving or turning the node moves or
    /// turns its draws about it.
    pub pivot: [f32; 16],
    /// Indices into [`RoomPack::draws`].
    pub draws: Vec<u32>,
    /// Whether it shows when the room loads; showing or hiding the node at
    /// run time stands in for its draws' own `visible`.
    pub visible: bool,
    pub hinge: Option<Hinge>,
}

/// A node that turns about an axis through its pivot, such as a door leaf.
/// Its draws are stored turned to `angle_deg`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hinge {
    /// Unit axis in the pivot's own frame.
    pub axis: [f32; 3],
    /// Degrees it is turned when the room loads, from its rest pose.
    pub angle_deg: f32,
}

/// A piece of scenery with several looks, only one shown at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    pub name: String,
    /// The first is the look before any state is set, named `""`.
    pub states: Vec<PieceState>,
    /// Index into `states` of the look shown when the room loads.
    pub state: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PieceState {
    pub name: String,
    /// Indices into [`RoomPack::draws`]; a draw is in at most one state of
    /// one piece.
    pub draws: Vec<u32>,
}

impl Stats {
    /// [`Stats::of`] counting the draws shown when the room loads.
    pub fn of_room(room: &RoomPack) -> Stats {
        let shown = room.shown_at_load();
        let draws: Vec<Draw> = (room.draws.iter().zip(shown))
            .map(|(d, shown)| Draw {
                visible: shown,
                name: None,
                ..d.clone()
            })
            .collect();
        Stats::of(&room.meshes, &room.textures, &draws)
    }
}

impl RoomPack {
    /// Whether each draw shows when the room loads: its own `visible`, its
    /// node's, its group on and its piece in the state holding it.
    pub fn shown_at_load(&self) -> Vec<bool> {
        let mut shown: Vec<bool> = self.draws.iter().map(|d| d.visible).collect();
        for n in self.nodes.iter().filter(|n| !n.visible) {
            for &d in &n.draws {
                shown[d as usize] = false;
            }
        }
        for g in self.groups.iter().filter(|g| !g.on) {
            for &d in &g.draws {
                shown[d as usize] = false;
            }
        }
        for p in &self.pieces {
            for (k, state) in p.states.iter().enumerate() {
                if k != p.state as usize {
                    for &d in &state.draws {
                        shown[d as usize] = false;
                    }
                }
            }
        }
        shown
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomPack {
    pub name: String,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    /// A draw is shown when its own `visible` is set, the node holding it,
    /// if any, is visible, the group holding it is on, and the piece state
    /// holding it, if any, is its piece's.
    pub draws: Vec<Draw>,
    /// A light shines when the group holding it, if any, is on.
    pub lights: Vec<Light>,
    pub ambient: Ambient,
    pub sun: Option<Sun>,
    pub background: Background,
    pub look: Look,
    pub camera: RoomCamera,
    pub stats: Stats,
    pub groups: Vec<Group>,
    pub nodes: Vec<Node>,
    pub pieces: Vec<Piece>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatError {
    BadMagic,
    Version { found: u32 },
    Truncated,
    Invalid(String),
}

impl FormatError {
    pub fn code(&self) -> &'static str {
        match self {
            FormatError::BadMagic => "E_PACK_BAD_MAGIC",
            FormatError::Version { .. } => "E_PACK_VERSION",
            FormatError::Truncated => "E_PACK_TRUNCATED",
            FormatError::Invalid(_) => "E_SCHEMA",
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::BadMagic => {
                write!(f, "not a room pack: the file does not start with SRMA")
            }
            FormatError::Version { found } => {
                write!(
                    f,
                    "room pack version {found} is not supported (expected {VERSION})"
                )
            }
            FormatError::Truncated => write!(f, "room pack is truncated"),
            FormatError::Invalid(why) => write!(f, "room pack is invalid: {why}"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Writes [`MAGIC`], [`VERSION`] and the postcard body. Does not validate.
pub fn encode(room: &RoomPack) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    // Every field has a known length and writing to a Vec cannot run out of
    // room, so postcard has no way to fail here.
    postcard::to_extend(room, out).expect("postcard cannot fail to serialize a RoomPack")
}

/// Decodes a pack and validates it: every mesh, material, texture and vertex
/// index must exist, every draw's bounds must be finite with `min <= max`,
/// every light's importance finite and at least 0, the look's `ao_strength`
/// in [0, 1] and its `ao_radius_m` finite and positive.
///
/// Never panics. A length read from the input cannot cause a large
/// allocation: postcard only reports a sequence length as a size hint when
/// that many bytes remain, and serde caps preallocation at 1 MiB.
pub fn decode(bytes: &[u8]) -> Result<RoomPack, FormatError> {
    let seen = bytes.len().min(MAGIC.len());
    if bytes[..seen] != MAGIC[..seen] {
        return Err(FormatError::BadMagic);
    }
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(FormatError::Truncated);
    };
    let found = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if found != VERSION {
        return Err(FormatError::Version { found });
    }
    let (room, rest) = postcard::take_from_bytes::<RoomPack>(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => FormatError::Truncated,
        other => FormatError::Invalid(other.to_string()),
    })?;
    if !rest.is_empty() {
        return Err(FormatError::Invalid(format!(
            "{} unexpected bytes after the room",
            rest.len()
        )));
    }
    validate(&room)?;
    Ok(room)
}

fn validate(room: &RoomPack) -> Result<(), FormatError> {
    let invalid = |why: String| Err(FormatError::Invalid(why));
    for (i, mesh) in room.meshes.iter().enumerate() {
        let vertices = mesh.vertices.len();
        if let Some(&index) = mesh.indices.iter().find(|&&ix| ix as usize >= vertices) {
            return invalid(format!(
                "mesh {i} uses vertex {index} but has {vertices} vertices"
            ));
        }
    }
    for (i, material) in room.materials.iter().enumerate() {
        let textures = [material.base_color_texture, material.emissive_texture];
        if let Some(texture) = textures
            .into_iter()
            .flatten()
            .find(|&t| t as usize >= room.textures.len())
        {
            return invalid(format!(
                "material {i} uses texture {texture} but there are {} textures",
                room.textures.len()
            ));
        }
    }
    for (i, light) in room.lights.iter().enumerate() {
        if !(light.importance.is_finite() && light.importance >= 0.0) {
            return invalid(format!(
                "light {i} has importance {}, which is not finite or is negative",
                light.importance
            ));
        }
    }
    let Look {
        ao_strength,
        ao_radius_m,
        ..
    } = room.look;
    if !(0.0..=1.0).contains(&ao_strength) {
        return invalid(format!("look.ao_strength is {ao_strength}, not in [0, 1]"));
    }
    if !(ao_radius_m.is_finite() && ao_radius_m > 0.0) {
        return invalid(format!(
            "look.ao_radius_m is {ao_radius_m}, which is not finite or not positive"
        ));
    }
    for (i, draw) in room.draws.iter().enumerate() {
        if draw.mesh as usize >= room.meshes.len() {
            return invalid(format!(
                "draw {i} uses mesh {} but there are {} meshes",
                draw.mesh,
                room.meshes.len()
            ));
        }
        if draw.material as usize >= room.materials.len() {
            return invalid(format!(
                "draw {i} uses material {} but there are {} materials",
                draw.material,
                room.materials.len()
            ));
        }
        let Aabb { min, max } = draw.bounds;
        if !(0..3).all(|k| min[k].is_finite() && max[k].is_finite() && min[k] <= max[k]) {
            return invalid(format!(
                "draw {i} has bounds {min:?}..{max:?}, which are not finite or not ordered"
            ));
        }
    }
    validate_scenery(room).map_err(FormatError::Invalid)
}

/// Every index of the groups, nodes and pieces in range, each draw and
/// light in at most one of each kind, names distinct within each table,
/// pivots finite and hinge axes of unit length.
fn validate_scenery(room: &RoomPack) -> Result<(), String> {
    distinct("group", room.groups.iter().map(|g| g.name.as_str()))?;
    distinct("node", room.nodes.iter().map(|n| n.name.as_str()))?;
    distinct("piece", room.pieces.iter().map(|p| p.name.as_str()))?;
    let groups = room
        .groups
        .iter()
        .map(|g| (format!("group {}", g.name), &g.draws[..]));
    once_each("draw", room.draws.len(), groups)?;
    let lights = room
        .groups
        .iter()
        .map(|g| (format!("group {}", g.name), &g.lights[..]));
    once_each("light", room.lights.len(), lights)?;
    let nodes = room
        .nodes
        .iter()
        .map(|n| (format!("node {}", n.name), &n.draws[..]));
    once_each("draw", room.draws.len(), nodes)?;
    let states = room.pieces.iter().flat_map(|p| {
        (p.states.iter())
            .map(move |s| (format!("piece {} state {:?}", p.name, s.name), &s.draws[..]))
    });
    once_each("draw", room.draws.len(), states)?;
    for n in &room.nodes {
        if !n.pivot.iter().all(|v| v.is_finite()) {
            return Err(format!("node {} has a pivot that is not finite", n.name));
        }
        if let Some(h) = &n.hinge {
            let length2: f32 = h.axis.iter().map(|a| a * a).sum();
            if !(h.angle_deg.is_finite() && (length2 - 1.0).abs() < 1e-3) {
                return Err(format!(
                    "node {} has hinge axis {:?} at {}°; the axis must be of unit length and the angle finite",
                    n.name, h.axis, h.angle_deg
                ));
            }
        }
    }
    for p in &room.pieces {
        if p.state as usize >= p.states.len() {
            return Err(format!(
                "piece {} starts in state {} of {}",
                p.name,
                p.state,
                p.states.len()
            ));
        }
        let names = p.states.iter().map(|s| s.name.as_str());
        distinct(&format!("state of piece {}", p.name), names)?;
    }
    Ok(())
}

/// Each index of every list below `len`, and in at most one of the lists.
fn once_each<'a>(
    what: &str,
    len: usize,
    lists: impl Iterator<Item = (String, &'a [u32])>,
) -> Result<(), String> {
    let mut owner: Vec<Option<String>> = vec![None; len];
    for (holder, list) in lists {
        for &i in list {
            let slot = (owner.get_mut(i as usize))
                .ok_or_else(|| format!("{holder} holds {what} {i} but there are {len}"))?;
            if let Some(first) = slot {
                return Err(format!("{what} {i} is in both {first} and {holder}"));
            }
            *slot = Some(holder.clone());
        }
    }
    Ok(())
}

fn distinct<'a>(what: &str, names: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen: Vec<&str> = names.collect();
    seen.sort_unstable();
    match seen.windows(2).find(|w| w[0] == w[1]) {
        Some(w) => Err(format!("two {what}s are named {:?}", w[0])),
        None => Ok(()),
    }
}

// Real spherical harmonics up to band 2 in the order
// (l, m) = (0,0) (1,-1) (1,0) (1,1) (2,-2) (2,-1) (2,0) (2,1) (2,2),
// evaluated on the engine-space components (x, y, z) of a unit direction.
// Same layout as three.js `SphericalHarmonics3.getAt`; shaders must match it.
const SH_Y00: f32 = 0.282_094_8; // 1 / (2 sqrt(pi))
const SH_Y1: f32 = 0.488_602_5; // sqrt(3) / (2 sqrt(pi))
const SH_Y2_PRODUCT: f32 = 1.092_548_4; // sqrt(15) / (2 sqrt(pi))
const SH_Y20: f32 = 0.315_391_57; // sqrt(5) / (4 sqrt(pi))
const SH_Y22: f32 = 0.546_274_2; // sqrt(15) / (4 sqrt(pi))

/// Coefficients that make [`eval_sh9`] return
/// `mix(ground, sky, 0.5 * n.y + 0.5) * intensity`.
///
/// That function is linear in `n.y`, so it fits bands 0 and 1 exactly and
/// band 2 stays zero.
pub fn hemisphere_sh9(sky: [f32; 3], ground: [f32; 3], intensity: f32) -> [[f32; 3]; 9] {
    let mut sh9 = [[0.0; 3]; 9];
    sh9[0] = std::array::from_fn(|c| 0.5 * (sky[c] + ground[c]) * intensity / SH_Y00);
    sh9[1] = std::array::from_fn(|c| 0.5 * (sky[c] - ground[c]) * intensity / SH_Y1);
    sh9
}

/// Ambient light reaching a surface with unit `normal`.
///
/// This is the plain SH sum with no cosine convolution, so packs store the
/// coefficients already convolved.
pub fn eval_sh9(sh9: &[[f32; 3]; 9], normal: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = normal;
    let basis = [
        SH_Y00,
        SH_Y1 * y,
        SH_Y1 * z,
        SH_Y1 * x,
        SH_Y2_PRODUCT * x * y,
        SH_Y2_PRODUCT * y * z,
        SH_Y20 * (3.0 * z * z - 1.0),
        SH_Y2_PRODUCT * x * z,
        SH_Y22 * (x * x - y * y),
    ];
    sh9.iter().zip(basis).fold([0.0; 3], |sum, (coeff, b)| {
        std::array::from_fn(|c| sum[c] + coeff[c] * b)
    })
}
