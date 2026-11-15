// SPDX-License-Identifier: Apache-2.0

//! Assembles a room pack from a kit, the game's placement file (`montagem.json`)
//! and its lighting file (`luz.json`).
//!
//! Placement, rotation and states follow `buildCasa` in the game's
//! `src/origem3d/casa.ts`. Hinges (`giros`) are not applied: hinged parts stay
//! at their rest pose.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use glam::{Mat3, Mat4, Vec3};
use sariema_format::{
    Aabb, Ambient, Background, Draw, Light, LightKind, Look, Material, Mesh, Mobility, RoomCamera,
    RoomPack, Shading, Stats, Sun, Texture, TextureFormat, Vertex, hemisphere_sh9,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::texture::srgb_to_linear;
use crate::{ImportError, Kit, KitPiece, KitPrimitive, load_kit, mip_chain_rgba8, read};

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
    pub drawn_materials: Vec<String>,
    pub rim_materials: Vec<(String, f32)>,
    pub additive_materials: Vec<String>,
    pub look: Look,
    pub camera: RoomCamera,
    pub background: Background,
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
struct Montagem {
    /// Metres per grid cell.
    celula: f32,
    comodos: HashMap<String, Comodo>,
}

#[derive(Deserialize)]
struct Comodo {
    /// The room's south-west corner, in cells.
    sw_celulas: [f32; 2],
    instancias: Vec<Inst>,
}

/// One piece placed in a room. Positions are metres from the room's corner on
/// Blender's axes: x east, y north, z up.
#[derive(Deserialize)]
struct Inst {
    peca: String,
    x: f32,
    y: f32,
    z: f32,
    /// Degrees about the vertical axis, used when `rot` is absent.
    rz: f32,
    /// Euler angles about Blender's axes, in degrees; see [`placement`].
    rot: Option<[f32; 3]>,
    estado: Option<String>,
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

    let mut scene = Scene {
        kit: &kit,
        loose: &source.loose,
        meshes: Vec::new(),
        draws: Vec::new(),
    };
    let mut lights = Vec::new();
    for room in &source.show {
        let comodo = montagem
            .comodos
            .get(room)
            .ok_or_else(|| ImportError::Schema(format!("montagem has no room {room}")))?;
        let corner = comodo.sw_celulas.map(|cells| cells * montagem.celula);
        scene.add_room(room, comodo, corner)?;
        for item in luz.comodos.get(room).map_or(&[][..], |l| &l.luzes) {
            lights.push(light(room, item, corner)?);
        }
    }
    let Scene { meshes, draws, .. } = scene;

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
    let textures: Vec<Texture> = kit
        .images
        .iter()
        .map(|image| Texture {
            name: image.name.clone(),
            width: image.rgba.width(),
            height: image.rgba.height(),
            format: TextureFormat::Rgba8Srgb,
            mips: mip_chain_rgba8(&image.rgba),
        })
        .collect();
    Ok(RoomPack {
        name: source.name.clone(),
        stats: Stats::of(&meshes, &textures, &draws),
        meshes,
        materials: kit.materials,
        textures,
        draws,
        lights,
        ambient: Ambient {
            sh9: hemisphere_sh9(
                linear_rgb(&sky.hemi_ceu)?,
                linear_rgb(&sky.hemi_chao)?,
                sky.hemi_int,
            ),
        },
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
    })
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
            material.shading = Shading::Additive;
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
    meshes: Vec<Mesh>,
    draws: Vec<Draw>,
}

impl Scene<'_> {
    /// Adds one draw per material for the room's merged pieces, and one per
    /// primitive of each loose piece.
    fn add_room(
        &mut self,
        room: &str,
        comodo: &Comodo,
        corner: [f32; 2],
    ) -> Result<(), ImportError> {
        let mut merged: BTreeMap<u32, Mesh> = BTreeMap::new();
        let mut copies: HashMap<&str, u32> = HashMap::new();
        for inst in &comodo.instancias {
            let piece = self
                .kit
                .piece(&inst.peca)
                .ok_or_else(|| ImportError::PieceMissing {
                    piece: inst.peca.clone(),
                    room: room.to_owned(),
                })?;
            let primitives = shown_primitives(piece, inst, room)?;
            let placement = placement(inst, corner, room)?;
            // As casa.ts treats nodes flagged oculto_no_inicio: hidden only while no estado is set.
            let hidden = piece.hidden_at_start && inst.estado.is_none();
            if self.loose.contains(&inst.peca) {
                let copy = copies.entry(&inst.peca).or_default();
                *copy += 1;
                let name = match *copy {
                    1 => format!("{room}.{}", inst.peca),
                    n => format!("{room}.{}.{n}", inst.peca),
                };
                for p in primitives {
                    self.push(name.clone(), p.mesh.clone(), p.material, placement, !hidden);
                }
            } else if !hidden {
                for p in primitives {
                    let mesh = merged.entry(p.material).or_insert_with(|| Mesh {
                        vertices: Vec::new(),
                        indices: Vec::new(),
                    });
                    append(mesh, &p.mesh, placement);
                }
            }
        }
        for (material, mesh) in merged {
            let name = format!("{room}/{}", self.kit.materials[material as usize].name);
            self.push(name, mesh, material, Mat4::IDENTITY, true);
        }
        Ok(())
    }

    fn push(&mut self, name: String, mesh: Mesh, material: u32, transform: Mat4, visible: bool) {
        // A mesh with no triangles would have no bounds.
        if mesh.indices.is_empty() {
            return;
        }
        let transform = transform.to_cols_array();
        self.draws.push(Draw {
            name: Some(name),
            mesh: self.meshes.len() as u32,
            material,
            bounds: Aabb::around(&mesh, &transform),
            transform,
            visible,
            // Additive surfaces (light cones, glows) add light and block none.
            casts_shadow: self.kit.materials[material as usize].shading != Shading::Additive,
        });
        self.meshes.push(mesh);
    }
}

/// The primitives the instance's `estado` shows, or the piece's start ones.
fn shown_primitives<'k>(
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

fn placement(inst: &Inst, corner: [f32; 2], room: &str) -> Result<Mat4, ImportError> {
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

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ImportError> {
    serde_json::from_slice(&read(path)?)
        .map_err(|e| ImportError::Schema(format!("{}: {e}", path.display())))
}

fn schema_error(e: impl fmt::Display) -> ImportError {
    ImportError::Schema(e.to_string())
}
