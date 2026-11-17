// SPDX-License-Identifier: Apache-2.0

//! Builds an actor pack from a glTF model with a skin and clips, or from a
//! rigid model, and the actor description that says how to read them.
//!
//! The skinned mesh nodes' own transforms are ignored, as glTF requires:
//! gltfpack bakes its dequantization into the inverse bind matrices.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sariema_format::Material;
use sariema_format::actor::{ActorPack, FaceAtlas, FaceFrame, FaceTrack, Marker};
use serde::Deserialize;
use serde_json::Value;

use crate::gltf::{Gltf, invalid};
use crate::{ImportError, TextureMode, encode_texture, kit, read};

mod clips;
mod finish;
mod skeleton;

const SCHEMA: &str = include_str!("../../../schemas/actor.schema.json");

/// An actor description, as `schemas/actor.schema.json` defines it.
#[derive(Clone, Debug, Deserialize)]
pub struct ActorSource {
    pub name: String,
    pub model: PathBuf,
    #[serde(default)]
    pub clips_from: Vec<PathBuf>,
    /// Patterns of looping clip names; `None` reads glTF `extras.loop`.
    #[serde(rename = "loop")]
    pub looping: Option<Vec<String>>,
    pub feet: Option<Vec<String>>,
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub sockets: BTreeMap<String, SocketSource>,
    #[serde(default)]
    pub faces: BTreeMap<String, FaceSource>,
    #[serde(default)]
    pub clip_faces: BTreeMap<String, BTreeMap<String, Vec<(f32, String)>>>,
    #[serde(default)]
    pub markers: BTreeMap<String, BTreeMap<String, f32>>,
    #[serde(default)]
    pub materials: BTreeMap<String, MaterialLook>,
    #[serde(default)]
    pub textures: Option<TextureMode>,
    #[serde(default)]
    pub keys: KeySettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SocketSource {
    pub joint: String,
    #[serde(default)]
    pub translation: [f32; 3],
    #[serde(default = "identity_rotation")]
    pub rotation: [f32; 4],
}

fn identity_rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

#[derive(Clone, Debug, Deserialize)]
pub struct FaceSource {
    pub materials: Vec<String>,
    /// `[columns, rows]`.
    pub grid: [u32; 2],
    #[serde(default)]
    pub origin: [f32; 2],
    pub step: Option<[f32; 2]>,
    pub frames: Vec<String>,
    #[serde(default)]
    pub first_cell: u32,
    pub start: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MaterialLook {
    pub rim: Option<f32>,
    pub drawn: Option<bool>,
    pub additive: Option<bool>,
}

/// How clips are stored; each field has its documented default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct KeySettings {
    /// A key is dropped when its neighbours reproduce it within these.
    pub translation_tolerance_m: f32,
    pub rotation_tolerance_deg: f32,
    pub scale_tolerance: f32,
    /// 16-bit rotations are within 0.004° of the original.
    pub quantize_rotations: bool,
    /// Cubic spline channels become linear keys at this rate.
    pub cubic_resample_fps: f32,
    /// Poses per second sampled for each clip's bounds.
    pub bounds_fps: f32,
    /// Poses per second sampled for each clip's ground speed.
    pub ground_speed_fps: f32,
}

impl Default for KeySettings {
    fn default() -> KeySettings {
        KeySettings {
            translation_tolerance_m: 1e-4,
            rotation_tolerance_deg: 0.01,
            scale_tolerance: 1e-4,
            quantize_rotations: true,
            cubic_resample_fps: 30.0,
            bounds_fps: 60.0,
            ground_speed_fps: 120.0,
        }
    }
}

impl ActorSource {
    /// Parses an actor description after checking it against `schemas/actor.schema.json`.
    pub fn from_json(json: &str) -> Result<ActorSource, ImportError> {
        let schema: Value = serde_json::from_str(SCHEMA).expect("actor.schema.json is JSON");
        let schema_error = |e: serde_json::Error| ImportError::Schema(e.to_string());
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

pub(crate) fn name_error(what: &str, name: &str, known: &[&str]) -> ImportError {
    ImportError::ActorName(format!(
        "no {what} {name}; there are {}",
        if known.is_empty() {
            "none".to_owned()
        } else {
            known.join(", ")
        }
    ))
}

/// Reads the model and every `clips_from` file under `assets_root`.
pub fn build_actor(source: &ActorSource, assets_root: &Path) -> Result<ActorPack, ImportError> {
    let bytes = read(&assets_root.join(&source.model))?;
    let gltf = Gltf::read(&bytes)?;
    let model = skeleton::read(&gltf, source)?;
    let (mut materials, _) = kit::materials(&gltf.doc)?;
    dress(&mut materials, &source.materials)?;
    let images = (gltf.doc.images.iter().enumerate())
        .map(|(i, image)| kit::image(&gltf, i, image))
        .collect::<Result<Vec<_>, _>>()?;
    let uvs = model
        .parts
        .iter()
        .flat_map(|p| (model.meshes[p.mesh as usize].vertices.iter()).map(|v| (p.material, v.uv)));
    let rgba: Vec<&image::RgbaImage> = images.iter().map(|i| &i.rgba).collect();
    kit::bake_metal_rough(
        &mut materials,
        &kit::metal_rough_maps(&gltf.doc)?,
        &rgba,
        uvs,
    );
    let mode = source.textures.unwrap_or_default();
    let textures = (images.iter())
        .map(|image| encode_texture(&image.name, &image.rgba, mode))
        .collect::<Result<Vec<_>, _>>()?;

    let rest: Vec<_> = model.joints.iter().map(|j| j.rest).collect();
    let mut clips = clips::read(
        &gltf,
        &|node| model.joint_of_node[node],
        &rest,
        &model.bases,
        source,
    )?;
    for path in &source.clips_from {
        let bytes = read(&assets_root.join(path))?;
        let other = Gltf::read(&bytes)?;
        let by_name = |node: usize| {
            let name = other.doc.nodes[node].name.as_deref()?;
            model
                .joints
                .iter()
                .position(|j| j.name == name)
                .map(|j| j as u32)
        };
        for clip in clips::read(&other, &by_name, &rest, &model.bases, source)? {
            if clips.iter().any(|c| c.name == clip.name) {
                return Err(invalid(format!(
                    "{}: clip {} is already in an earlier file",
                    path.display(),
                    clip.name
                )));
            }
            clips.push(clip);
        }
    }

    let faces = faces(source, &materials)?;
    let names = |v: &[&str]| v.join(", ");
    let clip_names: Vec<&str> = clips.iter().map(|c| c.name.as_str()).collect();
    let known_clips = names(&clip_names);
    for (clip_name, markers) in &source.markers {
        let clip = clips
            .iter_mut()
            .find(|c| &c.name == clip_name)
            .ok_or_else(|| {
                ImportError::ActorName(format!(
                    "markers name clip {clip_name}; the clips are {known_clips}"
                ))
            })?;
        for (name, &time) in markers {
            if time > clip.duration {
                return Err(ImportError::Schema(format!(
                    "marker {name} at {time} s is past the end of clip {clip_name}, {} s long",
                    clip.duration
                )));
            }
            clip.markers.push(Marker {
                name: name.clone(),
                time,
            });
        }
        clip.markers.sort_by(|a, b| a.time.total_cmp(&b.time));
    }
    for (clip_name, tracks) in &source.clip_faces {
        let clip = clips
            .iter_mut()
            .find(|c| &c.name == clip_name)
            .ok_or_else(|| {
                ImportError::ActorName(format!(
                    "clip_faces name clip {clip_name}; the clips are {known_clips}"
                ))
            })?;
        for (face_name, keys) in tracks {
            let index = faces
                .iter()
                .position(|f| &f.name == face_name)
                .ok_or_else(|| {
                    ImportError::ActorName(format!(
                        "clip_faces name face {face_name}, which is not in faces"
                    ))
                })?;
            let face = &faces[index];
            let mut keys = keys.clone();
            keys.sort_by(|a, b| a.0.total_cmp(&b.0));
            let cells = keys
                .iter()
                .map(|(_, frame)| {
                    let found = face.frames.iter().find(|f| &f.name == frame);
                    found.map(|f| f.cell).ok_or_else(|| {
                        ImportError::ActorName(format!("face {face_name} has no frame {frame}"))
                    })
                })
                .collect::<Result<_, _>>()?;
            clip.faces.push(FaceTrack {
                face: index as u32,
                times: keys.iter().map(|k| k.0.min(clip.duration)).collect(),
                cells,
            });
        }
    }

    let mut pack = ActorPack {
        name: source.name.clone(),
        meshes: model.meshes,
        materials,
        textures,
        parts: model.parts,
        joints: model.joints,
        clips,
        sockets: model.sockets,
        faces,
        bounds: sariema_format::Aabb {
            min: [0.0; 3],
            max: [0.0; 3],
        },
    };
    sariema_format::actor::turn_half(&mut pack);
    finish::measure(&mut pack, source)?;
    Ok(pack)
}

fn dress(
    materials: &mut [Material],
    looks: &BTreeMap<String, MaterialLook>,
) -> Result<(), ImportError> {
    for (name, look) in looks {
        let Some(m) = materials.iter_mut().find(|m| &m.name == name) else {
            let known: Vec<&str> = materials.iter().map(|m| m.name.as_str()).collect();
            return Err(name_error("material", name, &known));
        };
        if let Some(rim) = look.rim {
            m.rim = rim;
        }
        if let Some(drawn) = look.drawn {
            m.drawn = drawn;
        }
        if look.additive == Some(true) {
            m.alpha = sariema_format::AlphaMode::Additive;
        }
    }
    Ok(())
}

fn faces(source: &ActorSource, materials: &[Material]) -> Result<Vec<FaceAtlas>, ImportError> {
    let known: Vec<&str> = materials.iter().map(|m| m.name.as_str()).collect();
    source
        .faces
        .iter()
        .map(|(name, f)| {
            let [columns, rows] = f.grid;
            let indices = f
                .materials
                .iter()
                .map(|m| {
                    known
                        .iter()
                        .position(|k| k == m)
                        .map(|i| i as u32)
                        .ok_or_else(|| name_error("material", m, &known))
                })
                .collect::<Result<_, _>>()?;
            let frames: Vec<FaceFrame> = (f.first_cell..)
                .zip(&f.frames)
                .map(|(cell, name)| FaceFrame {
                    name: name.clone(),
                    cell,
                })
                .collect();
            let cells = u64::from(columns) * u64::from(rows);
            if frames.iter().any(|fr| u64::from(fr.cell) >= cells) {
                return Err(ImportError::Schema(format!(
                    "face {name} has {} frames from cell {}, past its {columns}×{rows} grid",
                    frames.len(),
                    f.first_cell
                )));
            }
            let start = match &f.start {
                None => frames[0].cell,
                Some(frame) => frames
                    .iter()
                    .find(|fr| &fr.name == frame)
                    .map(|fr| fr.cell)
                    .ok_or_else(|| {
                        ImportError::ActorName(format!("face {name} has no frame {frame}"))
                    })?,
            };
            Ok(FaceAtlas {
                name: name.clone(),
                materials: indices,
                columns,
                rows,
                origin: f.origin,
                step: f.step.unwrap_or([1.0 / columns as f32, 1.0 / rows as f32]),
                frames,
                start,
            })
        })
        .collect()
}

/// `*` matches any run of characters.
pub(crate) fn matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, rest)) => {
            let Some(tail) = name.strip_prefix(head) else {
                return false;
            };
            (0..=tail.len())
                .filter(|&i| tail.is_char_boundary(i))
                .any(|i| matches(rest, &tail[i..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn patterns_match_like_globs() {
        for (pattern, name, expected) in [
            ("walk", "walk", true),
            ("walk", "walker", false),
            ("*_lp", "sit_lp", true),
            ("*_lp", "sit_lp2", false),
            ("idle*", "idle", true),
            ("idle*", "idle_4", true),
            ("*a*b*", "xaxxbx", true),
            ("*a*b*", "xbxa", false),
            ("*", "", true),
        ] {
            assert_eq!(matches(pattern, name), expected, "{pattern} {name}");
        }
    }
}
