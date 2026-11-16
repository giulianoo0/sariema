// SPDX-License-Identifier: Apache-2.0

//! The glTF layer under the kit reader: GLB chunks, `EXT_meshopt_compression`
//! buffer views, accessors (`KHR_mesh_quantization` included) and the document.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;

use crate::ImportError;

const SUPPORTED_EXTENSIONS: [&str; 5] = [
    "EXT_meshopt_compression",
    "KHR_mesh_quantization",
    "KHR_texture_transform",
    "KHR_materials_emissive_strength",
    "KHR_materials_unlit",
];

const BYTE: u32 = 5120;
const UNSIGNED_BYTE: u32 = 5121;
const SHORT: u32 = 5122;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const FLOAT: u32 = 5126;

pub fn invalid(why: impl fmt::Display) -> ImportError {
    ImportError::Gltf(why.to_string())
}

/// A parsed GLB with its buffer views decoded. Its nodes form trees.
pub struct Gltf<'a> {
    pub doc: Document,
    views: Vec<Cow<'a, [u8]>>,
}

impl<'a> Gltf<'a> {
    pub fn read(bytes: &'a [u8]) -> Result<Self, ImportError> {
        let (json, bin) = split_glb(bytes)?;
        let required: Required = serde_json::from_slice(json).map_err(invalid)?;
        if let Some(name) = required
            .extensions_required
            .into_iter()
            .find(|e| !SUPPORTED_EXTENSIONS.contains(&e.as_str()))
        {
            return Err(ImportError::Extension(name));
        }
        let doc: Document = serde_json::from_slice(json).map_err(invalid)?;
        check_trees(&doc)?;
        let views = doc
            .buffer_views
            .iter()
            .enumerate()
            .map(|(i, view)| decode_view(&doc, bin, i, view))
            .collect::<Result<_, _>>()?;
        Ok(Gltf { doc, views })
    }
}

impl Gltf<'_> {
    /// The accessor's components as floats, with the glTF rules for normalized
    /// integers, and the number of components per element.
    pub fn floats(&self, index: usize, kinds: &[&str]) -> Result<(Vec<f32>, usize), ImportError> {
        let accessor = self.accessor(index, kinds)?;
        let ty = accessor.component_type;
        let size = match ty {
            BYTE | UNSIGNED_BYTE => 1,
            SHORT | UNSIGNED_SHORT => 2,
            UNSIGNED_INT | FLOAT => 4,
            _ => return Err(invalid(format!("accessor {index}: component type {ty}"))),
        };
        let components = match accessor.kind.as_str() {
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            _ => 1,
        };
        let elements = self.elements(index, accessor, components * size)?;
        let mut values = Vec::with_capacity(elements.len() * components);
        for element in elements {
            values.extend(
                element
                    .chunks_exact(size)
                    .map(|c| to_float(c, ty, accessor.normalized)),
            );
        }
        Ok((values, components))
    }

    pub fn indices(&self, index: usize) -> Result<Vec<u32>, ImportError> {
        let accessor = self.accessor(index, &["SCALAR"])?;
        let size = match accessor.component_type {
            UNSIGNED_BYTE => 1,
            UNSIGNED_SHORT => 2,
            UNSIGNED_INT => 4,
            ty => return Err(invalid(format!("accessor {index}: indices of type {ty}"))),
        };
        let elements = self.elements(index, accessor, size)?;
        Ok(elements
            .map(|c| match *c {
                [b] => u32::from(b),
                [b0, b1] => u32::from(u16::from_le_bytes([b0, b1])),
                [b0, b1, b2, b3] => u32::from_le_bytes([b0, b1, b2, b3]),
                _ => unreachable!("index elements are 1, 2 or 4 bytes"),
            })
            .collect())
    }

    pub fn view(&self, index: usize) -> Result<&[u8], ImportError> {
        self.views
            .get(index)
            .map(|v| &**v)
            .ok_or_else(|| invalid(format!("buffer view {index} does not exist")))
    }

    pub fn node(&self, index: usize) -> Result<&Node, ImportError> {
        self.doc
            .nodes
            .get(index)
            .ok_or_else(|| invalid(format!("node {index} does not exist")))
    }

    fn accessor(&self, index: usize, kinds: &[&str]) -> Result<&Accessor, ImportError> {
        let accessor = self
            .doc
            .accessors
            .get(index)
            .ok_or_else(|| invalid(format!("accessor {index} does not exist")))?;
        if !kinds.contains(&accessor.kind.as_str()) {
            return Err(invalid(format!(
                "accessor {index} is {}, expected {}",
                accessor.kind,
                kinds.join(" or ")
            )));
        }
        Ok(accessor)
    }

    /// The bytes of each element of `accessor`, `size` bytes long. Every element
    /// lies inside the buffer view, so the count is bounded by the view's length.
    fn elements(
        &self,
        index: usize,
        accessor: &Accessor,
        size: usize,
    ) -> Result<impl ExactSizeIterator<Item = &[u8]>, ImportError> {
        let bad = |why: &str| invalid(format!("accessor {index}: {why}"));
        if accessor.sparse.is_some() {
            return Err(bad("sparse accessors are not supported"));
        }
        let view = accessor.buffer_view.ok_or_else(|| bad("no buffer view"))?;
        let bytes = self.view(view)?;
        let stride = self.doc.buffer_views[view].byte_stride.unwrap_or(size);
        if stride < size {
            return Err(bad("the byte stride is shorter than an element"));
        }
        let offset = accessor.byte_offset;
        let end = match accessor.count.checked_sub(1) {
            None => Some(0),
            Some(last) => last
                .checked_mul(stride)
                .and_then(|start| start.checked_add(offset)?.checked_add(size)),
        };
        if end.is_none_or(|end| end > bytes.len()) {
            return Err(bad("reads past the end of its buffer view"));
        }
        Ok((0..accessor.count).map(move |i| &bytes[offset + i * stride..][..size]))
    }
}

/// Every child exists and has one parent, and no scene root is a child, so a walk
/// down from a root visits each node once.
fn check_trees(doc: &Document) -> Result<(), ImportError> {
    let mut has_parent = vec![false; doc.nodes.len()];
    for child in doc.nodes.iter().flat_map(|n| &n.children) {
        match has_parent.get_mut(*child) {
            None => return Err(invalid(format!("node {child} does not exist"))),
            Some(true) => return Err(invalid(format!("node {child} has two parents"))),
            Some(seen) => *seen = true,
        }
    }
    for &root in doc.scenes.iter().flat_map(|s| &s.nodes) {
        if has_parent.get(root).copied().unwrap_or(false) {
            return Err(invalid(format!("scene root {root} is also a child")));
        }
    }
    Ok(())
}

fn split_glb(bytes: &[u8]) -> Result<(&[u8], &[u8]), ImportError> {
    let Some((header, mut chunks)) = bytes.split_first_chunk::<12>() else {
        return Err(invalid("not a GLB: the file is too short"));
    };
    if header[..4] != *b"glTF" {
        return Err(invalid("not a GLB: the file does not start with glTF"));
    }
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if version != 2 {
        return Err(invalid(format!("GLB version {version} is not supported")));
    }
    let json =
        take_chunk(&mut chunks, b"JSON")?.ok_or_else(|| invalid("the GLB has no JSON chunk"))?;
    let bin = take_chunk(&mut chunks, b"BIN\0")?.unwrap_or_default();
    Ok((json, bin))
}

/// Returns the first chunk of `bytes` and moves `bytes` past it, if it is of `kind`.
fn take_chunk<'a>(bytes: &mut &'a [u8], kind: &[u8; 4]) -> Result<Option<&'a [u8]>, ImportError> {
    let Some((head, rest)) = bytes.split_first_chunk::<8>() else {
        return Ok(None);
    };
    if head[4..] != kind[..] {
        return Ok(None);
    }
    let length = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let Some((chunk, rest)) = rest.split_at_checked(length) else {
        return Err(invalid("the GLB is truncated"));
    };
    *bytes = rest;
    Ok(Some(chunk))
}

fn decode_view<'a>(
    doc: &Document,
    bin: &'a [u8],
    index: usize,
    view: &BufferView,
) -> Result<Cow<'a, [u8]>, ImportError> {
    let bad = |why: &dyn fmt::Display| invalid(format!("buffer view {index}: {why}"));
    let outside = || bad(&"reads outside the GLB binary chunk");
    let Some(compressed) = &view.extensions.meshopt else {
        let bytes = glb_slice(doc, bin, view.buffer, view.byte_offset, view.byte_length);
        return bytes.map(Cow::Borrowed).ok_or_else(outside);
    };
    let source = glb_slice(
        doc,
        bin,
        compressed.buffer,
        compressed.byte_offset,
        compressed.byte_length,
    )
    .ok_or_else(outside)?;
    meshopt_decode(compressed, source)
        .map(Cow::Owned)
        .map_err(|why| bad(&why))
}

/// Only buffer 0, when it has no URI, lives in the GLB; the reader reaches no other.
fn glb_slice<'a>(
    doc: &Document,
    bin: &'a [u8],
    buffer: usize,
    offset: usize,
    length: usize,
) -> Option<&'a [u8]> {
    let in_glb = buffer == 0 && doc.buffers.first().is_some_and(|b| b.uri.is_none());
    bin.get(offset..offset.checked_add(length)?)
        .filter(|_| in_glb)
}

/// Decodes an `EXT_meshopt_compression` buffer view. The size and stride checks
/// guard the C decoder's assertions.
fn meshopt_decode(m: &Meshopt, source: &[u8]) -> Result<Vec<u8>, String> {
    let (count, stride) = (m.count, m.byte_stride);
    let size = count
        .checked_mul(stride)
        .ok_or("the decoded size overflows")?;
    let meshopt_failed = |e: meshopt::Error| format!("meshopt: {e}");
    match m.mode {
        Mode::Attributes => {
            if stride == 0 || stride > 256 || stride % 4 != 0 {
                return Err(format!("byte stride {stride} is not valid for ATTRIBUTES"));
            }
            // u32 storage keeps the filters' i16 and u32 views aligned.
            let mut words = vec![0u32; size / 4];
            // SAFETY: `words` holds `count * stride` bytes, all the decoder writes,
            // and the decoder reads only the `source.len()` bytes of `source`.
            let status = unsafe {
                meshopt::ffi::meshopt_decodeVertexBuffer(
                    words.as_mut_ptr().cast(),
                    count,
                    stride,
                    source.as_ptr(),
                    source.len(),
                )
            };
            if status != 0 {
                return Err(format!("meshopt could not decode the vertices ({status})"));
            }
            match (m.filter, stride) {
                (Filter::None, _) => {}
                (Filter::Octahedral, 4) => {
                    meshopt::decode_filter_oct::<i8>(bytemuck::cast_slice_mut(&mut words))
                }
                (Filter::Octahedral, 8) => {
                    meshopt::decode_filter_oct::<i16>(bytemuck::cast_slice_mut(&mut words))
                }
                (Filter::Quaternion, 8) => {
                    meshopt::decode_filter_quat(bytemuck::cast_slice_mut(&mut words))
                }
                (Filter::Exponential, _) => {
                    meshopt::decode_filter_exp::<1>(bytemuck::cast_slice_mut(&mut words))
                }
                (filter, _) => return Err(format!("{filter:?} filter with byte stride {stride}")),
            }
            Ok(bytemuck::cast_slice(&words).to_vec())
        }
        _ if m.filter != Filter::None => Err("index data cannot be filtered".into()),
        Mode::Triangles if count % 3 != 0 => {
            Err(format!("{count} indices do not make whole triangles"))
        }
        Mode::Triangles => match stride {
            2 => meshopt::decode_index_buffer::<u16>(source, count).map(|i| bytes_of(&i)),
            4 => meshopt::decode_index_buffer::<u32>(source, count).map(|i| bytes_of(&i)),
            _ => return Err(format!("byte stride {stride} is not valid for indices")),
        }
        .map_err(meshopt_failed),
        Mode::Indices => match stride {
            2 => meshopt::decode_index_sequence::<u16>(source, count).map(|i| bytes_of(&i)),
            4 => meshopt::decode_index_sequence::<u32>(source, count).map(|i| bytes_of(&i)),
            _ => return Err(format!("byte stride {stride} is not valid for indices")),
        }
        .map_err(meshopt_failed),
    }
}

fn bytes_of<T: bytemuck::Pod>(values: &[T]) -> Vec<u8> {
    bytemuck::cast_slice(values).to_vec()
}

fn to_float(c: &[u8], ty: u32, normalized: bool) -> f32 {
    let (value, max) = match *c {
        [b] if ty == BYTE => (f32::from(b as i8), 127.0),
        [b] => (f32::from(b), 255.0),
        [b0, b1] if ty == SHORT => (f32::from(i16::from_le_bytes([b0, b1])), 32767.0),
        [b0, b1] => (f32::from(u16::from_le_bytes([b0, b1])), 65535.0),
        [b0, b1, b2, b3] if ty == FLOAT => return f32::from_le_bytes([b0, b1, b2, b3]),
        [b0, b1, b2, b3] => (u32::from_le_bytes([b0, b1, b2, b3]) as f32, u32::MAX as f32),
        _ => unreachable!("components are 1, 2 or 4 bytes"),
    };
    if normalized {
        (value / max).max(-1.0)
    } else {
        value
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Required {
    #[serde(default)]
    extensions_required: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Document {
    buffers: Vec<Buffer>,
    buffer_views: Vec<BufferView>,
    accessors: Vec<Accessor>,
    pub images: Vec<Image>,
    pub textures: Vec<Texture>,
    pub materials: Vec<Material>,
    pub meshes: Vec<Mesh>,
    pub nodes: Vec<Node>,
    pub scenes: Vec<Scene>,
    pub scene: Option<usize>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind_matrices: Option<usize>,
}

#[derive(Deserialize)]
pub struct Animation {
    pub name: Option<String>,
    pub channels: Vec<AnimationChannel>,
    pub samplers: Vec<AnimationSampler>,
    pub extras: Option<Value>,
}

#[derive(Deserialize)]
pub struct AnimationChannel {
    pub sampler: usize,
    pub target: AnimationTarget,
}

#[derive(Deserialize)]
pub struct AnimationTarget {
    pub node: Option<usize>,
    pub path: String,
}

#[derive(Deserialize)]
pub struct AnimationSampler {
    pub input: usize,
    pub output: usize,
    pub interpolation: Option<String>,
}

#[derive(Deserialize)]
struct Buffer {
    uri: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferView {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
    byte_stride: Option<usize>,
    #[serde(default)]
    extensions: BufferViewExtensions,
}

#[derive(Deserialize, Default)]
struct BufferViewExtensions {
    #[serde(rename = "EXT_meshopt_compression")]
    meshopt: Option<Meshopt>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meshopt {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
    byte_stride: usize,
    count: usize,
    mode: Mode,
    #[serde(default)]
    filter: Filter,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Mode {
    Attributes,
    Triangles,
    Indices,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Filter {
    #[default]
    None,
    Octahedral,
    Quaternion,
    Exponential,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Accessor {
    buffer_view: Option<usize>,
    #[serde(default)]
    byte_offset: usize,
    component_type: u32,
    #[serde(default)]
    normalized: bool,
    count: usize,
    #[serde(rename = "type")]
    kind: String,
    sparse: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub name: Option<String>,
    pub buffer_view: Option<usize>,
}

#[derive(Deserialize)]
pub struct Texture {
    pub source: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub name: Option<String>,
    #[serde(default)]
    pub pbr_metallic_roughness: Pbr,
    #[serde(default)]
    pub emissive_factor: [f32; 3],
    pub emissive_texture: Option<TextureInfo>,
    pub alpha_mode: Option<String>,
    #[serde(default)]
    pub double_sided: bool,
    #[serde(default)]
    pub extensions: MaterialExtensions,
    pub extras: Option<Value>,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pbr {
    pub base_color_factor: [f32; 4],
    pub base_color_texture: Option<TextureInfo>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub metallic_roughness_texture: Option<TextureInfo>,
}

impl Default for Pbr {
    fn default() -> Self {
        Pbr {
            base_color_factor: [1.0; 4],
            base_color_texture: None,
            metallic_factor: 1.0,
            roughness_factor: 1.0,
            metallic_roughness_texture: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureInfo {
    pub index: usize,
    #[serde(default)]
    pub tex_coord: u32,
    #[serde(default)]
    pub extensions: TextureInfoExtensions,
}

#[derive(Deserialize, Default)]
pub struct TextureInfoExtensions {
    #[serde(rename = "KHR_texture_transform")]
    pub transform: Option<TextureTransform>,
}

#[derive(Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct TextureTransform {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
    pub tex_coord: Option<u32>,
}

impl Default for TextureTransform {
    fn default() -> Self {
        TextureTransform {
            offset: [0.0; 2],
            scale: [1.0; 2],
            rotation: 0.0,
            tex_coord: None,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct MaterialExtensions {
    #[serde(rename = "KHR_materials_unlit")]
    pub unlit: Option<IgnoredAny>,
    #[serde(rename = "KHR_materials_emissive_strength")]
    pub emissive_strength: Option<EmissiveStrength>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmissiveStrength {
    pub emissive_strength: Option<f32>,
}

#[derive(Deserialize)]
pub struct Mesh {
    pub primitives: Vec<Primitive>,
}

#[derive(Deserialize)]
pub struct Primitive {
    pub attributes: BTreeMap<String, usize>,
    pub indices: Option<usize>,
    pub material: Option<usize>,
    pub mode: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Node {
    pub name: Option<String>,
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    matrix: Option<[f32; 16]>,
    translation: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    scale: Option<[f32; 3]>,
    pub extras: Option<Value>,
}

impl Node {
    /// Scale, rotation and translation; a `matrix` is decomposed.
    pub fn trs(&self) -> (Vec3, Quat, Vec3) {
        match self.matrix {
            Some(m) => Mat4::from_cols_array(&m).to_scale_rotation_translation(),
            None => (
                self.scale.map_or(Vec3::ONE, Vec3::from),
                self.rotation.map_or(Quat::IDENTITY, Quat::from_array),
                self.translation.map_or(Vec3::ZERO, Vec3::from),
            ),
        }
    }

    pub fn transform(&self) -> Mat4 {
        match self.matrix {
            Some(m) => Mat4::from_cols_array(&m),
            None => Mat4::from_scale_rotation_translation(
                self.scale.map_or(Vec3::ONE, Vec3::from),
                self.rotation.map_or(Quat::IDENTITY, Quat::from_array),
                self.translation.map_or(Vec3::ZERO, Vec3::from),
            ),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Scene {
    pub nodes: Vec<usize>,
}
