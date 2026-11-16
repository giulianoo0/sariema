// SPDX-License-Identifier: Apache-2.0

//! A small test room built entirely in code, with no asset files.

use crate::{
    Aabb, AlphaMode, Ambient, Background, CharacterLight, Draw, Fog, Light, LightKind, Look,
    Material, Mesh, Mobility, RoomCamera, RoomPack, Shading, Stats, Texture, TextureFormat, Vertex,
    hemisphere_sh9,
};

const FLOOR_MESH: u32 = 0;
const BOX_MESH: u32 = 1;
const WHITE: [u8; 4] = [255; 4];

/// A 10×10 m checkered floor, three boxes of different colours, an emissive
/// box, a "drawn" box, a shadow-casting spot, a point light, hemisphere
/// ambient, a dusk background gradient and a camera 12 m from the centre,
/// pitched 45°.
pub fn demo_room() -> RoomPack {
    let meshes = vec![floor_mesh(), box_mesh()];
    let textures = vec![checker_texture()];
    let materials = vec![
        Material {
            base_color_texture: Some(0),
            uv_scale: [2.5, 2.5],
            roughness: 0.9,
            ..material("floor", [1.0, 1.0, 1.0])
        },
        material("box_red", [0.8, 0.08, 0.06]),
        Material {
            roughness: 0.4,
            ..material("box_green", [0.1, 0.6, 0.12])
        },
        Material {
            roughness: 0.3,
            metallic: 0.5,
            ..material("box_blue", [0.08, 0.2, 0.8])
        },
        Material {
            emissive: [4.0, 3.0, 1.2],
            ..material("box_emissive", [1.0, 0.85, 0.5])
        },
        Material {
            drawn: true,
            rim: 0.4,
            ..material("box_drawn", [0.9, 0.45, 0.04])
        },
    ];
    let mut draws = vec![
        Draw {
            casts_shadow: false,
            ..draw("floor", FLOOR_MESH, 0, transform([0.0; 3], 0.0, [1.0; 3]))
        },
        draw(
            "box_red",
            BOX_MESH,
            1,
            transform([-2.5, 0.0, -1.0], 0.0, [1.0; 3]),
        ),
        draw(
            "box_green",
            BOX_MESH,
            2,
            transform([0.0, 0.0, -2.0], 30.0, [1.0, 1.5, 1.0]),
        ),
        draw(
            "box_blue",
            BOX_MESH,
            3,
            transform([2.5, 0.0, -1.0], 0.0, [1.2, 0.8, 1.2]),
        ),
        draw(
            "box_emissive",
            BOX_MESH,
            4,
            transform([-1.0, 0.0, 1.5], 0.0, [0.6; 3]),
        ),
        draw(
            "box_drawn",
            BOX_MESH,
            5,
            transform([1.5, 0.0, 1.5], -20.0, [1.0; 3]),
        ),
    ];
    let lights = vec![
        Light {
            name: "spot".into(),
            kind: LightKind::Spot {
                target: [-1.0, 0.0, -1.0],
                cone_deg: 60.0,
            },
            color: [1.0, 0.9, 0.75],
            intensity: 60.0,
            position: [-3.0, 6.0, 3.0],
            range: 15.0,
            flicker: None,
            casts_shadow: true,
            mobility: Mobility::Static,
            importance: 1.0,
        },
        Light {
            name: "point".into(),
            kind: LightKind::Point,
            color: [0.4, 0.6, 1.0],
            intensity: 4.0,
            position: [3.0, 2.0, 2.5],
            range: 6.0,
            flicker: None,
            casts_shadow: false,
            mobility: Mobility::Static,
            importance: 1.0,
        },
    ];
    for draw in &mut draws {
        draw.bounds = Aabb::around(&meshes[draw.mesh as usize], &draw.transform);
    }
    let stats = Stats::of(&meshes, &textures, &draws);
    RoomPack {
        name: "demo".into(),
        meshes,
        materials,
        textures,
        draws,
        lights,
        ambient: Ambient {
            sh9: hemisphere_sh9([0.6, 0.7, 0.85], [0.3, 0.25, 0.2], 0.5),
        },
        sun: None,
        background: Background {
            top: [0.05, 0.07, 0.12],
            bottom: [0.012, 0.01, 0.01],
        },
        look: Look {
            exposure: 2.0,
            bloom_threshold: 1.0,
            bloom_strength: 0.3,
            tint: [1.0, 1.0, 1.0],
            real: 0.0,
            night: 0.0,
            tilt: 0.0,
            focus: 12.0,
            ao_strength: Look::DEFAULT_AO_STRENGTH,
            ao_radius_m: Look::DEFAULT_AO_RADIUS_M,
        },
        camera: RoomCamera {
            fov_y_deg: 40.0,
            pitch_deg: 45.0,
            distance: 12.0,
            focus: [0.0, 0.0, 0.0],
        },
        stats,
        groups: Vec::new(),
        nodes: Vec::new(),
        pieces: Vec::new(),
        character: CharacterLight::default(),
        fog: Fog::default(),
    }
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

fn draw(name: &str, mesh: u32, material: u32, transform: [f32; 16]) -> Draw {
    Draw {
        name: Some(name.into()),
        mesh,
        material,
        transform,
        visible: true,
        casts_shadow: true,
        bounds: Aabb {
            min: [0.0; 3],
            max: [0.0; 3],
        },
    }
}

/// Column-major `translate * rotate_y(yaw_deg) * scale`.
fn transform(t: [f32; 3], yaw_deg: f32, s: [f32; 3]) -> [f32; 16] {
    let (sin, cos) = yaw_deg.to_radians().sin_cos();
    let columns: [[f32; 4]; 4] = [
        [s[0] * cos, 0.0, -s[0] * sin, 0.0],
        [0.0, s[1], 0.0, 0.0],
        [s[2] * sin, 0.0, s[2] * cos, 0.0],
        [t[0], t[1], t[2], 1.0],
    ];
    bytemuck::cast(columns)
}

/// Appends a quad centred on `c` spanning `u` and `v`, facing `u × v`, wound
/// counter-clockwise when seen from the front.
fn push_quad(mesh: &mut Mesh, c: [f32; 3], u: [f32; 3], v: [f32; 3]) {
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
        mesh.vertices.push(Vertex {
            pos: std::array::from_fn(|i| c[i] + du * u[i] + dv * v[i]),
            normal,
            uv,
            color: WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
}

fn floor_mesh() -> Mesh {
    let mut mesh = Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    push_quad(&mut mesh, [0.0; 3], [10.0, 0.0, 0.0], [0.0, 0.0, -10.0]);
    mesh
}

/// Unit cube standing on the origin: x and z in −0.5..0.5, y in 0..1.
fn box_mesh() -> Mesh {
    let mut mesh = Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let faces = [
        ([0.5, 0.5, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-0.5, 0.5, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.5, 0.5], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.5, -0.5], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    for (c, u, v) in faces {
        push_quad(&mut mesh, c, u, v);
    }
    mesh
}

/// 32×32 sRGB checker, 8 squares a side. Its mips stop at 8×8, one texel per
/// square, so every level is exact without filtering.
fn checker_texture() -> Texture {
    let level = |size: u32| -> Vec<u8> {
        let square = size / 8;
        (0..size * size)
            .flat_map(|i| {
                let light = (i % size / square + i / size / square).is_multiple_of(2);
                let g = if light { 200 } else { 120 };
                [g, g, g, 255]
            })
            .collect()
    };
    Texture {
        name: "checker".into(),
        width: 32,
        height: 32,
        format: TextureFormat::Rgba8Srgb,
        mips: [32, 16, 8].map(level).into(),
    }
}
