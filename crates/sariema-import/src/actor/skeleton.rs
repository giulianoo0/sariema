// SPDX-License-Identifier: Apache-2.0

//! The skeleton, sockets and skinned parts of an actor's model.

use glam::{Mat3, Mat4, Quat, Vec3};
use sariema_format::actor::{Joint, Part, SkinnedMesh, SkinnedVertex, Socket, Transform};

use super::{ActorSource, name_error};
use crate::ImportError;
use crate::gltf::{self, Gltf, invalid};
use crate::kit::{extra, truthy, unorm8};

const TRIANGLES: u32 = 4;
/// How far from identity a transform may be and still count as none.
const IDENTITY_TOLERANCE: f32 = 1e-5;

pub(super) struct Model {
    pub joints: Vec<Joint>,
    /// By glTF node index.
    pub joint_of_node: Vec<Option<u32>>,
    /// By joint: the transform of the nodes above a root joint.
    pub bases: Vec<Option<Basis>>,
    pub meshes: Vec<SkinnedMesh>,
    pub parts: Vec<Part>,
    pub sockets: Vec<Socket>,
}

/// How a primitive's vertices follow the skeleton.
enum Binding<'a> {
    /// Through `JOINTS_n`, whose skin-local indices map to joints here.
    Skin(&'a [u32]),
    /// Wholly to one joint.
    Rigid(u16),
}

pub(super) fn read(gltf: &Gltf, source: &ActorSource) -> Result<Model, ImportError> {
    let doc = &gltf.doc;
    let scene = doc
        .scenes
        .get(doc.scene.unwrap_or(0))
        .ok_or_else(|| invalid("the model has no scene"))?;
    let mut parent = vec![None; doc.nodes.len()];
    let mut world = vec![Mat4::IDENTITY; doc.nodes.len()];
    let mut order = Vec::new();
    let mut stack: Vec<(usize, Option<usize>)> =
        scene.nodes.iter().rev().map(|&n| (n, None)).collect();
    while let Some((node, up)) = stack.pop() {
        parent[node] = up;
        let local = gltf.node(node)?.transform();
        world[node] = up.map_or(local, |p| world[p] * local);
        order.push(node);
        stack.extend(
            doc.nodes[node]
                .children
                .iter()
                .rev()
                .map(|&c| (c, Some(node))),
        );
    }
    order.sort_unstable();
    let mesh_nodes: Vec<usize> = order
        .iter()
        .copied()
        .filter(|&n| doc.nodes[n].mesh.is_some())
        .collect();
    let mut skins: Vec<usize> = mesh_nodes
        .iter()
        .filter_map(|&n| doc.nodes[n].skin)
        .collect();
    skins.dedup();
    skins.sort_unstable();
    skins.dedup();
    let (joints, joint_of_node, skin_map, bases) = match skins.as_slice() {
        [] => rigid_skeleton(doc.nodes.len()),
        [skin] => skeleton(gltf, *skin, &parent)?,
        _ => {
            return Err(invalid(format!(
                "its meshes use {} skins; an actor has one skeleton",
                skins.len()
            )));
        }
    };
    let bind: Vec<Mat4> = joints
        .iter()
        .map(|j| Mat4::from_cols_array(&j.inverse_bind).inverse())
        .collect();
    let nearest_joint = |node: usize| {
        let mut at = parent[node];
        while let Some(n) = at {
            if let Some(j) = joint_of_node[n] {
                return Some((j, n));
            }
            at = parent[n];
        }
        None
    };
    // In a rigid model every node hangs from the one joint, in model space.
    let relative = |node: usize, joint_node: Option<usize>| match joint_node {
        Some(j) => world[j].inverse() * world[node],
        None => world[node],
    };

    let mut meshes = Vec::new();
    let mut parts = Vec::new();
    for &node in &mesh_nodes {
        let n = &doc.nodes[node];
        let mesh = doc
            .meshes
            .get(n.mesh.expect("mesh nodes have a mesh"))
            .ok_or_else(|| invalid(format!("node {node} uses a mesh that does not exist")))?;
        let name = n.name.as_deref().unwrap_or_default();
        let (binding, transform) = match (n.skin, skins.is_empty()) {
            (Some(_), _) => (Binding::Skin(&skin_map), Mat4::IDENTITY),
            (None, true) => (Binding::Rigid(0), world[node]),
            (None, false) => {
                let (joint, joint_node) = nearest_joint(node).ok_or_else(|| {
                    invalid(format!(
                        "mesh node {name} is neither skinned nor under a joint"
                    ))
                })?;
                let to_bind = bind[joint as usize] * relative(node, Some(joint_node));
                (Binding::Rigid(joint as u16), to_bind)
            }
        };
        let part_name = name.split("__").next().unwrap_or(name).to_owned();
        let hidden = [
            extra(&n.extras, "visible"),
            extra(&n.extras, "visivel_inicial"),
        ]
        .into_iter()
        .any(|v| v.is_some_and(|v| !truthy(Some(v))))
            || source.hidden.contains(&part_name);
        for primitive in &mesh.primitives {
            let material = primitive
                .material
                .map_or(doc.materials.len() as u32, |m| m as u32);
            meshes.push(read_primitive(
                gltf,
                primitive,
                &binding,
                transform,
                joints.len(),
            )?);
            parts.push(Part {
                name: part_name.clone(),
                mesh: meshes.len() as u32 - 1,
                material,
                visible: !hidden,
                casts_shadow: true,
            });
        }
    }
    for hidden in &source.hidden {
        if !parts.iter().any(|p| &p.name == hidden) {
            let known: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
            return Err(name_error("part", hidden, &known));
        }
    }

    let mut sockets = Vec::new();
    for &node in &order {
        let n = &doc.nodes[node];
        if joint_of_node[node].is_some() || n.mesh.is_some() || !n.children.is_empty() {
            continue;
        }
        let found = if skins.is_empty() {
            Some((0, None))
        } else {
            nearest_joint(node).map(|(j, jn)| (j, Some(jn)))
        };
        if let (Some((joint, joint_node)), Some(name)) = (found, &n.name) {
            sockets.push(Socket {
                name: name.clone(),
                joint,
                offset: relative(node, joint_node).to_cols_array(),
            });
        }
    }
    for (name, s) in &source.sockets {
        let Some(joint) = joints.iter().position(|j| j.name == s.joint) else {
            let known: Vec<&str> = joints.iter().map(|j| j.name.as_str()).collect();
            return Err(name_error("joint", &s.joint, &known));
        };
        let offset = Mat4::from_rotation_translation(
            Quat::from_array(s.rotation).normalize(),
            Vec3::from(s.translation),
        );
        sockets.retain(|existing| &existing.name != name);
        sockets.push(Socket {
            name: name.clone(),
            joint: joint as u32,
            offset: offset.to_cols_array(),
        });
    }
    Ok(Model {
        joints,
        joint_of_node,
        bases,
        meshes,
        parts,
        sockets,
    })
}

type Skeleton = (Vec<Joint>, Vec<Option<u32>>, Vec<u32>, Vec<Option<Basis>>);

/// One joint at the origin that every node of a rigid model hangs from.
fn rigid_skeleton(nodes: usize) -> Skeleton {
    let root = Joint {
        name: "root".into(),
        parent: None,
        rest: Transform::IDENTITY,
        inverse_bind: Mat4::IDENTITY.to_cols_array(),
    };
    (vec![root], vec![None; nodes], Vec::new(), vec![None])
}

/// The skin's joints with parents first, which glTF does not promise, and
/// the map from the skin's joint order to that order.
fn skeleton(gltf: &Gltf, skin: usize, parent: &[Option<usize>]) -> Result<Skeleton, ImportError> {
    let doc = &gltf.doc;
    let skin = doc
        .skins
        .get(skin)
        .ok_or_else(|| invalid(format!("skin {skin} does not exist")))?;
    let inverse_binds = match skin.inverse_bind_matrices {
        Some(accessor) => {
            let (m, _) = gltf.floats(accessor, &["MAT4"])?;
            if m.len() != skin.joints.len() * 16 {
                return Err(invalid(
                    "the skin has a different number of joints and inverse bind matrices",
                ));
            }
            m.as_chunks::<16>().0.to_vec()
        }
        None => vec![Mat4::IDENTITY.to_cols_array(); skin.joints.len()],
    };
    let is_joint = |n: usize| skin.joints.contains(&n);
    let mut depth = Vec::with_capacity(skin.joints.len());
    let mut joint_parent = Vec::with_capacity(skin.joints.len());
    let mut basis = Vec::with_capacity(skin.joints.len());
    for &node in &skin.joints {
        let mut up = parent.get(node).copied().flatten();
        let mut d = 0;
        let mut found = None;
        let mut between = Mat4::IDENTITY;
        while let Some(p) = up {
            if is_joint(p) {
                found = found.or(Some(p));
                d += 1;
            } else if found.is_none() {
                between = doc.nodes[p].transform() * between;
            }
            up = parent[p];
        }
        let joint_name = doc.nodes[node].name.as_deref().unwrap_or("?");
        basis.push(match found {
            _ if is_identity(between) => None,
            Some(_) => {
                return Err(invalid(format!(
                    "a node between joint {joint_name} and its parent joint moves it; apply its transform before exporting"
                )));
            }
            None => Some(Basis::of(between).ok_or_else(|| {
                invalid(format!(
                    "the nodes above joint {joint_name} scale it unevenly; apply their transform before exporting"
                ))
            })?),
        });
        depth.push(d);
        joint_parent.push(found);
    }
    let mut sorted: Vec<usize> = (0..skin.joints.len()).collect();
    sorted.sort_by_key(|&i| depth[i]);
    let mut joint_of_node = vec![None; doc.nodes.len()];
    for (new, &i) in sorted.iter().enumerate() {
        joint_of_node[skin.joints[i]] = Some(new as u32);
    }
    let joints = sorted
        .iter()
        .map(|&i| {
            let node = &doc.nodes[skin.joints[i]];
            let (scale, rotation, translation) = node.trs();
            let base = basis[i].unwrap_or(Basis::IDENTITY);
            Joint {
                name: node.name.clone().unwrap_or_else(|| format!("joint{i}")),
                parent: joint_parent[i].and_then(|p| joint_of_node[p]),
                rest: Transform {
                    translation: base.translation(translation).into(),
                    rotation: base.rotation(rotation.normalize()).into(),
                    scale: base.scale(scale).into(),
                },
                inverse_bind: inverse_binds[i],
            }
        })
        .collect();
    let skin_map = skin
        .joints
        .iter()
        .map(|&n| joint_of_node[n].expect("every skin joint was placed"))
        .collect();
    let bases = sorted.iter().map(|&i| basis[i]).collect();
    Ok((joints, joint_of_node, skin_map, bases))
}

/// What the nodes above a root joint do to it: a uniform scale, then a
/// rotation, then a translation, folded into the joint's own transforms.
#[derive(Clone, Copy, Debug)]
pub(super) struct Basis {
    scale: f32,
    rotation: Quat,
    translation: Vec3,
}

impl Basis {
    const IDENTITY: Basis = Basis {
        scale: 1.0,
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// `None` when `m` scales unevenly, which no joint transform can carry.
    fn of(m: Mat4) -> Option<Basis> {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        let even = (scale.max_element() - scale.min_element()).abs()
            <= IDENTITY_TOLERANCE * scale.max_element();
        even.then_some(Basis {
            scale: scale.x,
            rotation,
            translation,
        })
    }

    pub fn translation(&self, t: Vec3) -> Vec3 {
        self.translation + self.rotation * (t * self.scale)
    }

    pub fn rotation(&self, r: Quat) -> Quat {
        (self.rotation * r).normalize()
    }

    pub fn scale(&self, s: Vec3) -> Vec3 {
        s * self.scale
    }
}

fn is_identity(m: Mat4) -> bool {
    m.abs_diff_eq(Mat4::IDENTITY, IDENTITY_TOLERANCE)
}

fn read_primitive(
    gltf: &Gltf,
    p: &gltf::Primitive,
    binding: &Binding,
    transform: Mat4,
    joints: usize,
) -> Result<SkinnedMesh, ImportError> {
    if let Some(mode) = p.mode.filter(|&mode| mode != TRIANGLES) {
        return Err(invalid(format!(
            "primitive mode {mode} is not supported, only triangle lists"
        )));
    }
    let position = p
        .attributes
        .get("POSITION")
        .ok_or_else(|| invalid("a primitive has no POSITION"))?;
    let (positions, _) = gltf.floats(*position, &["VEC3"])?;
    let count = positions.len() / 3;
    let attribute = |name: &str, kinds: &[&str]| {
        let Some(&index) = p.attributes.get(name) else {
            return Ok(None);
        };
        let (values, components) = gltf.floats(index, kinds)?;
        if values.len() / components != count {
            return Err(invalid(format!("{name} and POSITION differ in length")));
        }
        Ok(Some((values, components)))
    };
    let normals = attribute("NORMAL", &["VEC3"])?;
    let uvs = attribute("TEXCOORD_0", &["VEC2"])?;
    let colors = attribute("COLOR_0", &["VEC3", "VEC4"])?;
    let sets: Vec<_> = (0..)
        .map_while(|set| {
            let j = attribute(&format!("JOINTS_{set}"), &["VEC4"]).transpose()?;
            let w = attribute(&format!("WEIGHTS_{set}"), &["VEC4"]).transpose()?;
            Some(j.and_then(|j| Ok((j.0, w?.0))))
        })
        .collect::<Result<_, _>>()?;
    if matches!(binding, Binding::Skin(_)) && sets.is_empty() {
        return Err(invalid("a skinned primitive has no JOINTS_0 and WEIGHTS_0"));
    }
    let normal_matrix = Mat3::from_mat4(transform).inverse().transpose();
    let mut vertices = Vec::with_capacity(count);
    let mut influences = Vec::with_capacity(4 * sets.len());
    for i in 0..count {
        let (joints_of, weights_of) = match binding {
            Binding::Rigid(joint) => ([*joint, 0, 0, 0], [u16::MAX, 0, 0, 0]),
            Binding::Skin(map) => {
                influences.clear();
                for (js, ws) in &sets {
                    for k in 0..4 {
                        let local = js[4 * i + k] as usize;
                        let joint = *map.get(local).ok_or_else(|| {
                            invalid(format!("a vertex uses skin joint {local} of {}", map.len()))
                        })?;
                        influences.push((joint as u16, ws[4 * i + k].max(0.0)));
                    }
                }
                quantize(&mut influences)
            }
        };
        debug_assert!(joints_of.iter().all(|&j| (j as usize) < joints.max(1)));
        vertices.push(SkinnedVertex {
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
            joints: joints_of,
            weights: weights_of,
        });
    }
    let mut indices = match p.indices {
        Some(index) => gltf.indices(index)?,
        None => (0..count as u32).collect(),
    };
    if !indices.len().is_multiple_of(3) {
        return Err(invalid("a primitive's indices do not make whole triangles"));
    }
    if let Some(&i) = indices.iter().find(|&&i| i as usize >= count) {
        return Err(invalid(format!(
            "a primitive uses vertex {i} but has {count} vertices"
        )));
    }
    if transform.determinant() < 0.0 {
        for triangle in indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
    }
    let mut mesh = SkinnedMesh { vertices, indices };
    if normals.is_none() {
        flat_normals(&mut mesh);
    }
    Ok(mesh)
}

/// The four strongest influences in unsigned normalized 16 bits summing to
/// 65535; a vertex without weight follows its first joint.
fn quantize(influences: &mut [(u16, f32)]) -> ([u16; 4], [u16; 4]) {
    influences.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let kept = &influences[..influences.len().min(4)];
    let sum: f32 = kept.iter().map(|k| k.1).sum();
    let mut joints = [0; 4];
    let mut weights = [0u16; 4];
    if sum <= 0.0 {
        joints[0] = kept.first().map_or(0, |k| k.0);
        weights[0] = u16::MAX;
        return (joints, weights);
    }
    for (k, &(joint, w)) in kept.iter().enumerate() {
        joints[k] = joint;
        weights[k] = (w / sum * u16::MAX as f32).round() as u16;
    }
    let total: i32 = weights.iter().map(|&w| i32::from(w)).sum();
    weights[0] = (i32::from(weights[0]) + i32::from(u16::MAX) - total) as u16;
    (joints, weights)
}

/// Gives every triangle its own vertices, with the face normal.
fn flat_normals(mesh: &mut SkinnedMesh) {
    let mut vertices = Vec::with_capacity(mesh.indices.len());
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|i| mesh.vertices[i as usize]);
        let [pa, pb, pc] = [a, b, c].map(|v| Vec3::from(v.pos));
        let normal = (pb - pa).cross(pc - pa).normalize_or_zero().to_array();
        vertices.extend([a, b, c].map(|v| SkinnedVertex { normal, ..v }));
    }
    mesh.indices = (0..vertices.len() as u32).collect();
    mesh.vertices = vertices;
}

#[cfg(test)]
mod tests {
    use super::quantize;

    #[test]
    fn weights_keep_the_four_strongest_and_sum_to_one() {
        let mut influences = vec![(3, 0.1), (1, 0.4), (7, 0.05), (2, 0.3), (5, 0.15)];
        let (joints, weights) = quantize(&mut influences);
        assert_eq!(joints, [1, 2, 5, 3]);
        assert_eq!(weights.iter().map(|&w| u32::from(w)).sum::<u32>(), 65535);
        let (joints, weights) = quantize(&mut [(4, 0.0), (6, 0.0)]);
        assert_eq!((joints[0], weights), (4, [65535, 0, 0, 0]));
    }
}
