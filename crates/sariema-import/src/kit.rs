// SPDX-License-Identifier: Apache-2.0

//! Reads a kit: a GLB whose scene root nodes are reusable scenery pieces.
//!
//! Which nodes of a piece show follows `applyState` in the game's
//! `src/origem3d/casa.ts`, applied to a freshly loaded piece.
//!
//! Vertex UVs are kept as stored: materials map them with
//! `uv * uv_scale + uv_offset`, as `KHR_texture_transform` does.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use glam::{Mat3, Mat4, Vec3};
use sariema_format::{AlphaMode, Material, Mesh, Shading, Vertex};
use serde_json::Value;

use crate::gltf::{self, Gltf, invalid};
use crate::{ImportError, decode_ktx2, read};

const TRIANGLES: u32 = 4;

#[derive(Debug)]
pub struct KitImage {
    pub name: String,
    /// Pixels as stored in the file, so sRGB for colour textures.
    pub rgba: image::RgbaImage,
}

#[derive(Debug)]
pub struct KitPrimitive {
    pub mesh: Mesh,
    /// Index into [`Kit::materials`].
    pub material: u32,
}

#[derive(Debug)]
pub struct KitPiece {
    pub name: String,
    /// The piece as it starts, in its root node's space: child transforms are baked
    /// in, the root's own transform is not. Nodes below the root that start hidden
    /// are left out.
    pub primitives: Vec<KitPrimitive>,
    /// `extras.oculto_no_inicio` on the root node: the whole piece starts hidden.
    pub hidden_at_start: bool,
    /// The piece in each state of the root's `extras.estados`, sorted by state name.
    pub states: Vec<(String, Vec<KitPrimitive>)>,
    /// Nodes below the root with `extras.giro`, left out of `primitives`
    /// and `states`.
    pub hinges: Vec<KitHinge>,
}

/// A node `<piece>_<hinge>` of a piece that turns about an axis through
/// its origin.
#[derive(Debug)]
pub struct KitHinge {
    /// The node's name without the `<piece>_` prefix.
    pub name: String,
    /// In the node's own frame: +Y for `"eixo": "vertical"`, +X otherwise.
    pub axis: Vec3,
    /// The node's origin in the piece's root space.
    pub pivot: Mat4,
    /// The node and its shown descendants at rest, in the piece's root space.
    pub primitives: Vec<KitPrimitive>,
}

#[derive(Debug)]
pub struct Kit {
    pub pieces: Vec<KitPiece>,
    pub materials: Vec<Material>,
    /// Indexed by [`Material::base_color_texture`] and [`Material::emissive_texture`].
    pub images: Vec<KitImage>,
}

impl Kit {
    pub fn piece(&self, name: &str) -> Option<&KitPiece> {
        self.pieces.iter().find(|p| p.name == name)
    }

    /// Triangles of every piece as it starts.
    pub fn triangles(&self) -> u32 {
        let indices: usize = self
            .pieces
            .iter()
            .flat_map(|p| &p.primitives)
            .map(|p| p.mesh.indices.len())
            .sum();
        (indices / 3) as u32
    }
}

/// Each root node of the GLB's scene becomes a piece named after the node.
pub fn load_kit(path: &Path) -> Result<Kit, ImportError> {
    let bytes = read(path)?;
    let gltf = Gltf::read(&bytes)?;
    let doc = &gltf.doc;
    let images = doc
        .images
        .iter()
        .enumerate()
        .map(|(i, source)| image(&gltf, i, source))
        .collect::<Result<_, _>>()?;
    let (materials, default_material) = materials(doc)?;
    let reader = Reader {
        gltf: &gltf,
        default_material,
    };
    let scene = doc
        .scenes
        .get(doc.scene.unwrap_or(0))
        .ok_or_else(|| invalid("the kit has no scene"))?;
    let pieces = scene
        .nodes
        .iter()
        .map(|&root| reader.piece(root))
        .collect::<Result<_, _>>()?;
    Ok(Kit {
        pieces,
        materials,
        images,
    })
}

pub(crate) fn image(
    gltf: &Gltf,
    index: usize,
    source: &gltf::Image,
) -> Result<KitImage, ImportError> {
    let view = source
        .buffer_view
        .ok_or_else(|| invalid(format!("image {index} is not stored in the GLB")))?;
    let bytes = gltf.view(view)?;
    let rgba = if source.mime_type.as_deref() == Some("image/ktx2") {
        decode_ktx2(bytes).map_err(|e| invalid(format!("image {index}: {e}")))?
    } else {
        image::load_from_memory(bytes)
            .map_err(|e| invalid(format!("image {index}: {e}")))?
            .into_rgba8()
    };
    Ok(KitImage {
        name: source.name.clone().unwrap_or_default(),
        rgba,
    })
}

/// The file's materials, then glTF's default material if some primitive names
/// none, and the index such primitives use.
pub(crate) fn materials(doc: &gltf::Document) -> Result<(Vec<Material>, u32), ImportError> {
    let mut materials = doc
        .materials
        .iter()
        .map(|m| material(doc, m))
        .collect::<Result<Vec<_>, _>>()?;
    let default_index = materials.len() as u32;
    let primitives = doc.meshes.iter().flat_map(|m| &m.primitives);
    if primitives.clone().any(|p| p.material.is_none()) {
        materials.push(default_material());
    }
    if let Some(m) = primitives
        .filter_map(|p| p.material)
        .find(|&m| m >= doc.materials.len())
    {
        return Err(invalid(format!("material {m} does not exist")));
    }
    Ok((materials, default_index))
}

fn material(doc: &gltf::Document, m: &gltf::Material) -> Result<Material, ImportError> {
    let name = m.name.clone().unwrap_or_default();
    let pbr = &m.pbr_metallic_roughness;
    let base = texture(doc, &name, pbr.base_color_texture.as_ref())?;
    let emissive = texture(doc, &name, m.emissive_texture.as_ref())?;
    let (uv_offset, uv_scale) = match (base, emissive) {
        (Some((_, b)), Some((_, e))) if b != e => {
            return Err(invalid(format!(
                "material {name}: the base colour and emissive textures use different texture transforms"
            )));
        }
        (Some((_, uv)), _) | (None, Some((_, uv))) => uv,
        (None, None) => ([0.0; 2], [1.0; 2]),
    };
    let additive = truthy(extra(&m.extras, "aditivo")) || name.ends_with("_aditivo");
    let shading = if m.extensions.unlit.is_some() {
        Shading::Unlit
    } else {
        Shading::Lit
    };
    let strength = m
        .extensions
        .emissive_strength
        .as_ref()
        .and_then(|e| e.emissive_strength)
        .unwrap_or(1.0);
    Ok(Material {
        name,
        base_color: pbr.base_color_factor,
        base_color_texture: base.map(|(image, _)| image),
        uv_offset,
        uv_scale,
        roughness: pbr.roughness_factor,
        metallic: pbr.metallic_factor,
        emissive: m.emissive_factor.map(|c| c * strength),
        emissive_texture: emissive.map(|(image, _)| image),
        shading,
        alpha: match m.alpha_mode.as_deref() {
            _ if additive => AlphaMode::Additive,
            Some("BLEND") => AlphaMode::Blend,
            _ => AlphaMode::Opaque,
        },
        double_sided: m.double_sided,
        drawn: false,
        rim: 0.0,
    })
}

/// `KHR_texture_transform` as `(offset, scale)`.
type UvTransform = ([f32; 2], [f32; 2]);

/// The image a material's texture slot samples, and its texture transform.
fn texture(
    doc: &gltf::Document,
    material: &str,
    info: Option<&gltf::TextureInfo>,
) -> Result<Option<(u32, UvTransform)>, ImportError> {
    let Some(info) = info else {
        return Ok(None);
    };
    let transform = info.extensions.transform.clone().unwrap_or_default();
    if info.tex_coord != 0
        || transform.tex_coord.is_some_and(|set| set != 0)
        || transform.rotation != 0.0
    {
        return Err(invalid(format!(
            "material {material}: only TEXCOORD_0 without rotation is supported"
        )));
    }
    let image = doc
        .textures
        .get(info.index)
        .and_then(gltf::Texture::image)
        .filter(|&i| i < doc.images.len())
        .ok_or_else(|| invalid(format!("material {material}: missing texture image")))?;
    Ok(Some((image as u32, (transform.offset, transform.scale))))
}

/// glTF's default material.
fn default_material() -> Material {
    Material {
        name: String::new(),
        base_color: [1.0; 4],
        base_color_texture: None,
        uv_offset: [0.0; 2],
        uv_scale: [1.0; 2],
        roughness: 1.0,
        metallic: 1.0,
        emissive: [0.0; 3],
        emissive_texture: None,
        shading: Shading::Lit,
        alpha: AlphaMode::Opaque,
        double_sided: false,
        drawn: false,
        rim: 0.0,
    }
}

struct Reader<'a> {
    gltf: &'a Gltf<'a>,
    /// Where [`materials`] put glTF's default material.
    default_material: u32,
}

impl Reader<'_> {
    fn piece(&self, root: usize) -> Result<KitPiece, ImportError> {
        let node = self.gltf.node(root)?;
        let name = node.name.clone().unwrap_or_default();
        let estados: BTreeMap<String, Vec<String>> =
            match extra(&node.extras, "estados").filter(|v| !v.is_null()) {
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| invalid(format!("piece {name}: extras.estados: {e}")))?,
                None => BTreeMap::new(),
            };
        let state_nodes: HashSet<&str> = estados.values().flatten().map(String::as_str).collect();
        let gather = |state: Option<&[String]>| {
            let mut primitives = Vec::new();
            let look = Look {
                state,
                state_nodes: &state_nodes,
            };
            self.collect(root, Mat4::IDENTITY, &look, &mut primitives)?;
            Ok::<_, ImportError>(primitives)
        };
        let mut hinges = Vec::new();
        self.hinges(root, Mat4::IDENTITY, &name, &state_nodes, &mut hinges)?;
        Ok(KitPiece {
            primitives: gather(None)?,
            hidden_at_start: truthy(extra(&node.extras, "oculto_no_inicio")),
            states: estados
                .iter()
                .map(|(state, nodes)| Ok((state.clone(), gather(Some(nodes.as_slice()))?)))
                .collect::<Result<_, ImportError>>()?,
            hinges,
            name,
        })
    }

    /// The hinged nodes below `index`, whose chain of transforms from the
    /// root is `transform`.
    fn hinges(
        &self,
        index: usize,
        transform: Mat4,
        piece: &str,
        state_nodes: &HashSet<&str>,
        out: &mut Vec<KitHinge>,
    ) -> Result<(), ImportError> {
        for &child in &self.gltf.node(index)?.children {
            let node = self.gltf.node(child)?;
            let pivot = transform * node.transform();
            let Some(giro) = extra(&node.extras, "giro").filter(|g| truthy(Some(g))) else {
                self.hinges(child, pivot, piece, state_nodes, out)?;
                continue;
            };
            let full = node.name.clone().unwrap_or_default();
            let vertical = giro.get("eixo").and_then(Value::as_str) == Some("vertical");
            let mut primitives = Vec::new();
            let look = Look {
                state: None,
                state_nodes,
            };
            self.collect(child, pivot, &look, &mut primitives)?;
            out.push(KitHinge {
                name: (full.strip_prefix(piece))
                    .and_then(|rest| rest.strip_prefix('_'))
                    .unwrap_or(&full)
                    .to_owned(),
                axis: if vertical { Vec3::Y } else { Vec3::X },
                pivot,
                primitives,
            });
        }
        Ok(())
    }

    /// Appends the primitives of `index` and of its shown descendants.
    fn collect(
        &self,
        index: usize,
        transform: Mat4,
        look: &Look,
        out: &mut Vec<KitPrimitive>,
    ) -> Result<(), ImportError> {
        let node = self.gltf.node(index)?;
        if let Some(mesh) = node.mesh {
            let mesh = self
                .gltf
                .doc
                .meshes
                .get(mesh)
                .ok_or_else(|| invalid(format!("mesh {mesh} does not exist")))?;
            for primitive in &mesh.primitives {
                out.push(self.primitive(primitive, transform)?);
            }
        }
        for &child in &node.children {
            let child_node = self.gltf.node(child)?;
            let hinged = truthy(extra(&child_node.extras, "giro"));
            if !hinged && look.shows(child_node) {
                self.collect(child, transform * child_node.transform(), look, out)?;
            }
        }
        Ok(())
    }

    fn primitive(&self, p: &gltf::Primitive, transform: Mat4) -> Result<KitPrimitive, ImportError> {
        if let Some(mode) = p.mode.filter(|&mode| mode != TRIANGLES) {
            return Err(invalid(format!(
                "primitive mode {mode} is not supported, only triangle lists"
            )));
        }
        let position = p
            .attributes
            .get("POSITION")
            .ok_or_else(|| invalid("a primitive has no POSITION"))?;
        let (positions, _) = self.gltf.floats(*position, &["VEC3"])?;
        let count = positions.len() / 3;
        let attribute = |name: &str, kinds: &[&str]| {
            let Some(&index) = p.attributes.get(name) else {
                return Ok(None);
            };
            let (values, components) = self.gltf.floats(index, kinds)?;
            if values.len() / components != count {
                return Err(invalid(format!("{name} and POSITION differ in length")));
            }
            Ok(Some((values, components)))
        };
        let normals = attribute("NORMAL", &["VEC3"])?;
        let uvs = attribute("TEXCOORD_0", &["VEC2"])?;
        let colors = attribute("COLOR_0", &["VEC3", "VEC4"])?;

        let normal_matrix = Mat3::from_mat4(transform).inverse().transpose();
        let vertices = (0..count)
            .map(|i| Vertex {
                pos: transform
                    .transform_point3(Vec3::from_slice(&positions[3 * i..]))
                    .to_array(),
                normal: normals.as_ref().map_or([0.0; 3], |(n, _)| {
                    (normal_matrix * Vec3::from_slice(&n[3 * i..]))
                        .normalize_or_zero()
                        .to_array()
                }),
                uv: uvs
                    .as_ref()
                    .map_or([0.0; 2], |(t, _)| [t[2 * i], t[2 * i + 1]]),
                color: colors.as_ref().map_or([255; 4], |(c, n)| {
                    std::array::from_fn(|k| if k < *n { unorm8(c[n * i + k]) } else { 255 })
                }),
            })
            .collect();

        let mut indices = match p.indices {
            Some(index) => self.gltf.indices(index)?,
            None => (0..count as u32).collect(),
        };
        if indices.len() % 3 != 0 {
            return Err(invalid("a primitive's indices do not make whole triangles"));
        }
        if let Some(&i) = indices.iter().find(|&&i| i as usize >= count) {
            return Err(invalid(format!(
                "a primitive uses vertex {i} but has {count} vertices"
            )));
        }
        // A mirroring transform turns counter-clockwise front faces clockwise.
        if transform.determinant() < 0.0 {
            for triangle in indices.as_chunks_mut::<3>().0 {
                triangle.swap(1, 2);
            }
        }
        let mut mesh = Mesh { vertices, indices };
        if normals.is_none() {
            flat_normals(&mut mesh);
        }
        Ok(KitPrimitive {
            mesh,
            material: p.material.map_or(self.default_material, |m| m as u32),
        })
    }
}

/// One look of a piece: its start (`state` is `None`) or one of its states.
struct Look<'a> {
    state: Option<&'a [String]>,
    /// Every node named by any state of the piece.
    state_nodes: &'a HashSet<&'a str>,
}

impl Look<'_> {
    fn shows(&self, node: &gltf::Node) -> bool {
        let name = node.name.as_deref().unwrap_or_default();
        let hidden_at_start = truthy(extra(&node.extras, "oculto_no_inicio"));
        match self.state {
            _ if !self.state_nodes.contains(name) => !hidden_at_start || self.state.is_some(),
            Some(shown) => shown.iter().any(|n| n == name),
            None => !hidden_at_start,
        }
    }
}

pub(crate) fn unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Gives every triangle its own vertices, with the face normal.
fn flat_normals(mesh: &mut Mesh) {
    let mut vertices = Vec::with_capacity(mesh.indices.len());
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|i| mesh.vertices[i as usize]);
        let [pa, pb, pc] = [a, b, c].map(|v| Vec3::from(v.pos));
        let normal = (pb - pa).cross(pc - pa).normalize_or_zero().to_array();
        vertices.extend([a, b, c].map(|v| Vertex { normal, ..v }));
    }
    mesh.indices = (0..vertices.len() as u32).collect();
    mesh.vertices = vertices;
}

pub(crate) fn extra<'a>(extras: &'a Option<Value>, key: &str) -> Option<&'a Value> {
    extras.as_ref()?.get(key)
}

/// JavaScript truthiness: the game tests its extras flags with it.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}
