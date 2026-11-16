// SPDX-License-Identifier: Apache-2.0

//! Two small test actors built in code, with no asset files.

use super::{
    ActorPack, Channel, Clip, FaceAtlas, FaceFrame, FaceTrack, Interpolation, Joint, Marker, Part,
    Property, SkinnedMesh, SkinnedVertex, Socket, Transform, Values,
};
use crate::{Aabb, AlphaMode, Material, Shading, Texture, TextureFormat};

const FULL: u16 = u16::MAX;

/// A 1.2 m square column standing on the origin that bends like an arm:
/// joint 0 `shoulder` at the floor, joint 1 `elbow` 0.5 m up and joint 2
/// `wrist` 1 m up. Vertices follow the shoulder below 0.4 m, the elbow from
/// 0.6 to 0.9 m and the wrist above 1.1 m, blending linearly between. A part
/// `hand`, a 0.2 m cube on top, follows the wrist, and socket `tip` sits 0.2 m
/// above the wrist.
///
/// Clips: `bend` loops over 1 s, turning the elbow about +Z by 0°, 90°, 0°
/// at 0, 0.5 and 1 s, with marker `kick` at 0.5 s. `wave` plays once over 0.8 s, turning the wrist about +X
/// by 0°, 60°, 0° and scaling it by 1, 1.5, 1 at 0, 0.4 and 0.8 s, with markers
/// `tap` at 0.25 s and `end` at 0.8 s. `spin` loops over 1 s, turning the
/// shoulder about +Y from 170° to 190°, with its second key stored as the
/// negated quaternion, so only the shortest arc passes through 180°.
pub fn demo_arm() -> ActorPack {
    let mut column = mesh();
    let levels = [0.0, 0.2, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0, 1.1, 1.2];
    push_column(&mut column, &levels, 0.12, arm_weights);
    push_quad(
        &mut column,
        [0.0; 3],
        [0.24, 0.0, 0.0],
        [0.0, 0.0, 0.24],
        |p| arm_weights(p[1]),
    );
    let mut hand = mesh();
    push_cube(&mut hand, [0.0, 1.3, 0.0], 0.1, |_| weights(2, 2, 1.0));
    let bend = Clip {
        channels: vec![rotation(
            1,
            [0.0, 0.0, 1.0],
            &[(0.0, 0.0), (0.5, 90.0), (1.0, 0.0)],
        )],
        markers: vec![marker("kick", 0.5)],
        ..clip("bend", 1.0, true, [[-1.0, -0.05, -0.2], [0.3, 1.45, 0.2]])
    };
    let wave = Clip {
        channels: vec![
            rotation(2, [1.0, 0.0, 0.0], &[(0.0, 0.0), (0.4, 60.0), (0.8, 0.0)]),
            Channel {
                joint: 2,
                property: Property::Scale,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 0.4, 0.8],
                values: Values::Vec3(vec![[1.0; 3], [1.5; 3], [1.0; 3]]),
            },
        ],
        markers: vec![marker("tap", 0.25), marker("end", 0.8)],
        ..clip("wave", 0.8, false, [[-0.3, -0.05, -0.3], [0.3, 1.65, 0.6]])
    };
    let half = |deg: f32| (deg.to_radians() * 0.5).sin_cos();
    let ((s0, c0), (s1, c1)) = (half(170.0), half(190.0));
    let spin = Clip {
        channels: vec![Channel {
            joint: 0,
            property: Property::Rotation,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: Values::Quat(vec![[0.0, s0, 0.0, c0], [0.0, -s1, 0.0, -c1]]),
        }],
        ..clip("spin", 1.0, true, [[-0.3, -0.05, -0.3], [0.3, 1.45, 0.3]])
    };
    ActorPack {
        name: "arm".into(),
        meshes: vec![column, hand],
        materials: vec![
            Material {
                rim: 0.3,
                ..material("arm", [0.9, 0.35, 0.08])
            },
            material("hand", [0.1, 0.3, 0.9]),
        ],
        textures: Vec::new(),
        parts: vec![part("arm", 0, 0), part("hand", 1, 1)],
        joints: vec![
            joint("shoulder", None, [0.0; 3], [0.0; 3]),
            joint("elbow", Some(0), [0.0, 0.5, 0.0], [0.0, 0.5, 0.0]),
            joint("wrist", Some(1), [0.0, 0.5, 0.0], [0.0, 1.0, 0.0]),
        ],
        clips: vec![bend, wave, spin],
        sockets: vec![Socket {
            name: "tip".into(),
            joint: 2,
            offset: translation([0.0, 0.2, 0.0]),
        }],
        faces: Vec::new(),
        bounds: aabb([[-1.0, -0.05, -0.3], [0.3, 1.65, 0.6]]),
    }
}

/// A 0.5 m body with a head on joint 1 `head`, 0.5 m up, whose front is a
/// 0.3 m square on +Z textured from a 2×2 atlas: face `eyes` with frames
/// `open`, `closed`, `happy` and `angry` in cells 0 to 3, red, green, blue and
/// yellow. Clip `nod` loops over 1 s, turning the head about +X by 0°, 20°,
/// 0° at 0, 0.5 and 1 s, and shows `closed` from 0.5 s on.
pub fn demo_face() -> ActorPack {
    let mut body = mesh();
    push_cube(&mut body, [0.0, 0.25, 0.0], 0.25, |_| weights(0, 0, 1.0));
    let mut head = mesh();
    push_cube(&mut head, [0.0, 0.7, 0.0], 0.2, |_| weights(1, 1, 1.0));
    let mut front = mesh();
    push_quad(
        &mut front,
        [0.0, 0.7, 0.201],
        [0.3, 0.0, 0.0],
        [0.0, 0.3, 0.0],
        |_| weights(1, 1, 1.0),
    );
    for v in &mut front.vertices {
        v.uv = v.uv.map(|c| c * 0.5);
    }
    let nod = Clip {
        channels: vec![rotation(
            1,
            [1.0, 0.0, 0.0],
            &[(0.0, 0.0), (0.5, 20.0), (1.0, 0.0)],
        )],
        faces: vec![FaceTrack {
            face: 0,
            times: vec![0.0, 0.5],
            cells: vec![0, 1],
        }],
        ..clip("nod", 1.0, true, [[-0.3, -0.05, -0.3], [0.3, 1.0, 0.45]])
    };
    ActorPack {
        name: "face".into(),
        meshes: vec![body, head, front],
        materials: vec![
            material("body", [0.6, 0.6, 0.6]),
            Material {
                base_color: [1.0; 4],
                base_color_texture: Some(0),
                ..material("face", [1.0; 3])
            },
        ],
        textures: vec![atlas_texture()],
        parts: vec![part("body", 0, 0), part("head", 1, 0), part("face", 2, 1)],
        joints: vec![
            joint("root", None, [0.0; 3], [0.0; 3]),
            joint("head", Some(0), [0.0, 0.5, 0.0], [0.0, 0.5, 0.0]),
        ],
        clips: vec![nod],
        sockets: Vec::new(),
        faces: vec![FaceAtlas {
            name: "eyes".into(),
            materials: vec![1],
            columns: 2,
            rows: 2,
            origin: [0.0, 0.0],
            step: [0.5, 0.5],
            frames: ["open", "closed", "happy", "angry"]
                .iter()
                .zip(0..)
                .map(|(name, cell)| FaceFrame {
                    name: (*name).into(),
                    cell,
                })
                .collect(),
            start: 0,
        }],
        bounds: aabb([[-0.3, -0.05, -0.3], [0.3, 1.0, 0.45]]),
    }
}

/// Joint weights of the arm column at height `y`.
fn arm_weights(y: f32) -> ([u16; 4], [u16; 4]) {
    match y {
        y if y <= 0.4 => weights(0, 0, 1.0),
        y if y < 0.6 => weights(0, 1, (0.6 - y) / 0.2),
        y if y <= 0.9 => weights(1, 1, 1.0),
        y if y < 1.1 => weights(1, 2, (1.1 - y) / 0.2),
        _ => weights(2, 2, 1.0),
    }
}

/// Joint `a` with weight `wa` and joint `b` with the rest, summing to 65535.
fn weights(a: u16, b: u16, wa: f32) -> ([u16; 4], [u16; 4]) {
    let first = (wa.clamp(0.0, 1.0) * FULL as f32).round() as u16;
    ([a, b, 0, 0], [first, FULL - first, 0, 0])
}

fn mesh() -> SkinnedMesh {
    SkinnedMesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    }
}

/// Four sides of a square column of half width `half`, one ring of
/// vertices per height in `levels`.
fn push_column(
    mesh: &mut SkinnedMesh,
    levels: &[f32],
    half: f32,
    skin: fn(f32) -> ([u16; 4], [u16; 4]),
) {
    let sides = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ];
    for (normal, across) in sides {
        let base = mesh.vertices.len() as u32;
        for &y in levels {
            for side in [-1.0f32, 1.0] {
                let (joints, weights) = skin(y);
                mesh.vertices.push(SkinnedVertex {
                    pos: std::array::from_fn(|i| {
                        (normal[i] + side * across[i]) * half + [0.0, y, 0.0][i]
                    }),
                    normal,
                    uv: [0.5 + 0.5 * side, 1.0 - y / levels[levels.len() - 1]],
                    color: [255; 4],
                    joints,
                    weights,
                });
            }
        }
        for k in 0..levels.len() as u32 - 1 {
            let [a, b, c, d] = [0, 1, 3, 2].map(|i| base + 2 * k + i);
            mesh.indices.extend([a, b, c, a, c, d]);
        }
    }
}

fn push_cube(
    mesh: &mut SkinnedMesh,
    c: [f32; 3],
    half: f32,
    skin: impl Fn([f32; 3]) -> ([u16; 4], [u16; 4]) + Copy,
) {
    let s = 2.0 * half;
    let at = |d: [f32; 3]| std::array::from_fn(|i| c[i] + d[i] * half);
    let faces = [
        (at([1.0, 0.0, 0.0]), [0.0, 0.0, -s], [0.0, s, 0.0]),
        (at([-1.0, 0.0, 0.0]), [0.0, 0.0, s], [0.0, s, 0.0]),
        (at([0.0, 1.0, 0.0]), [s, 0.0, 0.0], [0.0, 0.0, -s]),
        (at([0.0, -1.0, 0.0]), [s, 0.0, 0.0], [0.0, 0.0, s]),
        (at([0.0, 0.0, 1.0]), [s, 0.0, 0.0], [0.0, s, 0.0]),
        (at([0.0, 0.0, -1.0]), [-s, 0.0, 0.0], [0.0, s, 0.0]),
    ];
    for (centre, u, v) in faces {
        push_quad(mesh, centre, u, v, skin);
    }
}

/// A quad centred on `c` spanning `u` and `v`, facing `u × v`, wound
/// counter-clockwise seen from the front, with UVs 0..1.
fn push_quad(
    mesh: &mut SkinnedMesh,
    c: [f32; 3],
    u: [f32; 3],
    v: [f32; 3],
    skin: impl Fn([f32; 3]) -> ([u16; 4], [u16; 4]),
) {
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let normal = n.map(|x| x / len);
    let base = mesh.vertices.len() as u32;
    for (du, dv, uv) in [
        (-0.5, -0.5, [0.0, 1.0]),
        (0.5, -0.5, [1.0, 1.0]),
        (0.5, 0.5, [1.0, 0.0]),
        (-0.5, 0.5, [0.0, 0.0]),
    ] {
        let pos = std::array::from_fn(|i| c[i] + du * u[i] + dv * v[i]);
        let (joints, weights) = skin(pos);
        mesh.vertices.push(SkinnedVertex {
            pos,
            normal,
            uv,
            color: [255; 4],
            joints,
            weights,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
}

fn material(name: &str, rgb: [f32; 3]) -> Material {
    Material {
        name: name.into(),
        base_color: [rgb[0], rgb[1], rgb[2], 1.0],
        base_color_texture: None,
        uv_offset: [0.0, 0.0],
        uv_scale: [1.0, 1.0],
        roughness: 0.6,
        metallic: 0.0,
        emissive: [0.0; 3],
        emissive_texture: None,
        shading: Shading::Lit,
        alpha: AlphaMode::Opaque,
        double_sided: false,
        drawn: false,
        rim: 0.0,
    }
}

fn part(name: &str, mesh: u32, material: u32) -> Part {
    Part {
        name: name.into(),
        mesh,
        material,
        visible: true,
        casts_shadow: true,
    }
}

/// `local` is the rest translation from the parent, `bind` the joint's
/// position in the bind pose; neither joint turns or scales at rest.
fn joint(name: &str, parent: Option<u32>, local: [f32; 3], bind: [f32; 3]) -> Joint {
    Joint {
        name: name.into(),
        parent,
        rest: Transform {
            translation: local,
            ..Transform::IDENTITY
        },
        inverse_bind: translation(bind.map(|x| -x)),
    }
}

fn translation(t: [f32; 3]) -> [f32; 16] {
    let mut m = [0.0; 16];
    for i in 0..4 {
        m[i * 5] = 1.0;
    }
    m[12..15].copy_from_slice(&t);
    m
}

/// Rotations of `joint` about the unit `axis` by `(time, degrees)` keys.
fn rotation(joint: u32, axis: [f32; 3], keys: &[(f32, f32)]) -> Channel {
    Channel {
        joint,
        property: Property::Rotation,
        interpolation: Interpolation::Linear,
        times: keys.iter().map(|k| k.0).collect(),
        values: Values::Quat(
            keys.iter()
                .map(|&(_, deg)| {
                    let (s, c) = (deg.to_radians() * 0.5).sin_cos();
                    [axis[0] * s, axis[1] * s, axis[2] * s, c]
                })
                .collect(),
        ),
    }
}

fn marker(name: &str, time: f32) -> Marker {
    Marker {
        name: name.into(),
        time,
    }
}

fn clip(name: &str, duration: f32, looping: bool, bounds: [[f32; 3]; 2]) -> Clip {
    Clip {
        name: name.into(),
        duration,
        looping,
        channels: Vec::new(),
        markers: Vec::new(),
        faces: Vec::new(),
        bounds: aabb(bounds),
        ground_speed: None,
    }
}

fn aabb([min, max]: [[f32; 3]; 2]) -> Aabb {
    Aabb { min, max }
}

/// 8×8 sRGB, 2×2 cells of 4×4 texels: red, green, blue, yellow in row-major order.
fn atlas_texture() -> Texture {
    let colors = [
        [220, 30, 30, 255],
        [30, 200, 40, 255],
        [30, 60, 220, 255],
        [230, 210, 30, 255],
    ];
    let texels = (0..64)
        .flat_map(|i: usize| colors[(i % 8) / 4 + 2 * (i / 8 / 4)])
        .collect();
    Texture {
        name: "face_atlas".into(),
        width: 8,
        height: 8,
        format: TextureFormat::Rgba8Srgb,
        mips: vec![texels],
    }
}
