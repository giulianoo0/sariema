// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use common::write_glb;
use sariema_format::{AlphaMode, Shading};
use sariema_import::{KitPiece, KitPrimitive, load_kit};

mod common;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// One piece, `abajur`: a 1-triangle base, a 2-triangle `abajur_aceso` that starts
/// hidden and a 4-triangle `abajur_apagado`. Positions only, no material.
fn lamp_kit() -> PathBuf {
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut bin: Vec<u8> = bytemuck::cast_slice(&positions).to_vec();
    bin.extend([0u16, 1, 2].repeat(7).iter().flat_map(|i| i.to_le_bytes()));
    let json = r#"{
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 78 }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 42 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
            { "bufferView": 1, "byteOffset": 0, "componentType": 5123, "count": 3, "type": "SCALAR" },
            { "bufferView": 1, "byteOffset": 6, "componentType": 5123, "count": 6, "type": "SCALAR" },
            { "bufferView": 1, "byteOffset": 18, "componentType": 5123, "count": 12, "type": "SCALAR" }
        ],
        "meshes": [
            { "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1 }] },
            { "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 2 }] },
            { "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 3 }] }
        ],
        "nodes": [
            {
                "name": "abajur",
                "children": [1, 2, 3],
                "extras": { "estados": { "aceso": ["abajur_aceso"], "apagado": ["abajur_apagado"] } }
            },
            { "name": "abajur_base", "mesh": 0 },
            { "name": "abajur_aceso", "mesh": 1, "extras": { "oculto_no_inicio": true } },
            { "name": "abajur_apagado", "mesh": 2 }
        ],
        "scenes": [{ "nodes": [0] }],
        "scene": 0
    }"#;
    write_glb("lamp_kit.glb", json, &bin)
}

/// Mesh 0 is the triangle (0,0,0), (1,0,0), (0,1,0) with normal +Z and the given
/// indices. The first of `nodes` is the only root.
fn triangle_kit(name: &str, nodes: &str, indices: [u16; 3]) -> PathBuf {
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let normals: [f32; 9] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let mut bin: Vec<u8> = bytemuck::cast_slice(&positions).to_vec();
    bin.extend(bytemuck::cast_slice(&normals));
    bin.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
    let json = format!(
        r#"{{
        "asset": {{ "version": "2.0" }},
        "buffers": [{{ "byteLength": 78 }}],
        "bufferViews": [
            {{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }},
            {{ "buffer": 0, "byteOffset": 36, "byteLength": 36 }},
            {{ "buffer": 0, "byteOffset": 72, "byteLength": 6 }}
        ],
        "accessors": [
            {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
            {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
            {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
        ],
        "meshes": [{{ "primitives": [{{ "attributes": {{ "POSITION": 0, "NORMAL": 1 }}, "indices": 2 }}] }}],
        "nodes": {nodes},
        "scenes": [{{ "nodes": [0] }}]
    }}"#
    );
    write_glb(name, &json, &bin)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn triangles(primitives: &[KitPrimitive]) -> usize {
    primitives.iter().map(|p| p.mesh.indices.len() / 3).sum()
}

fn state<'a>(piece: &'a KitPiece, name: &str) -> &'a [KitPrimitive] {
    &piece.states.iter().find(|(s, _)| s == name).unwrap().1
}

fn extent(primitives: &[KitPrimitive], axis: usize) -> (f32, f32) {
    primitives
        .iter()
        .flat_map(|p| &p.mesh.vertices)
        .fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            (lo.min(v.pos[axis]), hi.max(v.pos[axis]))
        })
}

#[test]
fn loads_compressed_fixture() {
    let kit = load_kit(&fixture("kit_min.opt.glb")).unwrap();
    let names: Vec<_> = kit.pieces.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["caixa", "mesa", "lampada"]);
    assert_eq!(kit.triangles(), 36);
    assert!(kit.piece("lampada").unwrap().hidden_at_start);
    assert!(!kit.piece("caixa").unwrap().hidden_at_start);
    assert_eq!(kit.images.len(), 1);
    assert_eq!(kit.images[0].rgba.dimensions(), (4, 4));
    assert_eq!(kit.materials[0].name, "mat0");
    assert_eq!(kit.materials[0].base_color_texture, Some(0));
    assert_eq!(kit.materials[0].uv_scale, [2.0, 2.0]);
}

#[test]
fn child_transform_is_baked() {
    let kit = load_kit(&fixture("kit_min.opt.glb")).unwrap();
    let (bottom, top) = extent(&kit.piece("mesa").unwrap().primitives, 1);
    assert!((bottom - 0.5).abs() < 0.01, "bottom {bottom}");
    assert!((top - 1.5).abs() < 0.01, "top {top}");
}

#[test]
fn quantized_positions_are_dequantized() {
    let kit = load_kit(&fixture("kit_min.opt.glb")).unwrap();
    let caixa = &kit.piece("caixa").unwrap().primitives;
    for axis in 0..3 {
        let (lo, hi) = extent(caixa, axis);
        assert!((hi - lo - 1.0).abs() < 0.01, "axis {axis}: {lo}..{hi}");
    }
}

#[test]
fn compressed_attributes_are_decoded() {
    let kit = load_kit(&fixture("kit_min.opt.glb")).unwrap();
    let vertices = &kit.piece("caixa").unwrap().primitives[0].mesh.vertices;
    for v in vertices {
        let length = v.normal.iter().map(|c| c * c).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 0.01, "normal {:?}", v.normal);
        let axis_aligned = v.normal.iter().filter(|c| c.abs() > 0.99).count();
        assert_eq!(axis_aligned, 1, "normal {:?}", v.normal);
        assert!(
            v.uv.iter().all(|c| (0.0..=1.0).contains(c)),
            "uv {:?}",
            v.uv
        );
        assert!(
            v.color
                .iter()
                .zip([255, 128, 128, 255])
                .all(|(&c, e)| c.abs_diff(e) <= 1)
        );
    }
}

#[test]
fn unknown_required_extension_is_error() {
    let path = write_glb(
        "kit_draco_required.glb",
        r#"{"asset":{"version":"2.0"},"extensionsRequired":["KHR_draco_mesh_compression"]}"#,
        &[],
    );
    let err = load_kit(&path).unwrap_err();
    assert_eq!(err.code(), "E_GLTF_EXTENSION");
    assert!(err.to_string().contains("KHR_draco_mesh_compression"));
}

#[test]
fn states_and_hidden_nodes_follow_the_game() {
    let kit = load_kit(&lamp_kit()).unwrap();
    let lamp = kit.piece("abajur").unwrap();
    assert!(!lamp.hidden_at_start);
    assert_eq!(triangles(&lamp.primitives), 1 + 4);
    let names: Vec<_> = lamp.states.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(names, ["aceso", "apagado"]);
    assert_eq!(triangles(state(lamp, "aceso")), 1 + 2);
    assert_eq!(triangles(state(lamp, "apagado")), 1 + 4);
}

#[test]
fn missing_attributes_get_defaults() {
    let kit = load_kit(&lamp_kit()).unwrap();
    let mesh = &kit.piece("abajur").unwrap().primitives[0].mesh;
    for v in &mesh.vertices {
        assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        assert_eq!(v.uv, [0.0, 0.0]);
        assert_eq!(v.color, [255; 4]);
    }
}

#[test]
fn materials_follow_the_mapping_rules() {
    let json = r#"{
        "asset": { "version": "2.0" },
        "materials": [
            {
                "name": "liso",
                "pbrMetallicRoughness": {
                    "baseColorFactor": [0.5, 0.25, 0.125, 1.0],
                    "metallicFactor": 0.2,
                    "roughnessFactor": 0.3
                }
            },
            { "name": "tela", "extensions": { "KHR_materials_unlit": {} } },
            {
                "name": "brilho",
                "extensions": { "KHR_materials_unlit": {} },
                "extras": { "aditivo": true }
            },
            { "name": "cone_aditivo" },
            { "name": "vidro", "alphaMode": "BLEND", "doubleSided": true },
            {
                "name": "lampada",
                "emissiveFactor": [1.0, 0.5, 0.25],
                "extensions": { "KHR_materials_emissive_strength": { "emissiveStrength": 4.0 } }
            }
        ],
        "scenes": [{ "nodes": [] }]
    }"#;
    let kit = load_kit(&write_glb("materials.glb", json, &[])).unwrap();
    let material = |name: &str| kit.materials.iter().find(|m| m.name == name).unwrap();

    let liso = material("liso");
    assert_eq!(liso.shading, Shading::Lit);
    assert_eq!(liso.alpha, AlphaMode::Opaque);
    assert_eq!(liso.base_color, [0.5, 0.25, 0.125, 1.0]);
    assert_eq!((liso.metallic, liso.roughness), (0.2, 0.3));
    assert_eq!(liso.emissive, [0.0; 3]);
    assert_eq!((liso.drawn, liso.rim), (false, 0.0));
    assert_eq!(material("tela").shading, Shading::Unlit);
    let brilho = material("brilho");
    assert_eq!(
        (brilho.shading, brilho.alpha),
        (Shading::Unlit, AlphaMode::Additive)
    );
    let cone = material("cone_aditivo");
    assert_eq!(
        (cone.shading, cone.alpha),
        (Shading::Lit, AlphaMode::Additive)
    );
    assert_eq!(material("vidro").alpha, AlphaMode::Blend);
    assert!(material("vidro").double_sided && !liso.double_sided);
    assert_eq!(material("lampada").emissive, [4.0, 2.0, 1.0]);
}

#[test]
fn emissive_texture_is_kept() {
    let mut png = Vec::new();
    image::RgbaImage::new(1, 1)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let kit_json = |pbr: &str| {
        format!(
            r#"{{
            "asset": {{ "version": "2.0" }},
            "buffers": [{{ "byteLength": {length} }}],
            "bufferViews": [{{ "buffer": 0, "byteLength": {length} }}],
            "images": [{{ "bufferView": 0, "mimeType": "image/png" }}],
            "textures": [{{ "source": 0 }}],
            "materials": [{{
                "name": "tela",
                "pbrMetallicRoughness": {{ {pbr} }},
                "emissiveTexture": {{
                    "index": 0,
                    "extensions": {{ "KHR_texture_transform": {{ "scale": [2, 2] }} }}
                }},
                "emissiveFactor": [1, 1, 1]
            }}],
            "scenes": [{{ "nodes": [] }}]
        }}"#,
            length = png.len()
        )
    };
    let kit = load_kit(&write_glb("emissive.glb", &kit_json(""), &png)).unwrap();
    let tela = &kit.materials[0];
    assert_eq!(tela.emissive_texture, Some(0));
    assert_eq!(tela.base_color_texture, None);
    assert_eq!(tela.uv_scale, [2.0, 2.0]);

    // Both textures share the material's one texture transform.
    let base = r#""baseColorTexture": { "index": 0 }"#;
    let err = load_kit(&write_glb("emissive_mismatch.glb", &kit_json(base), &png)).unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
}

#[test]
fn mirrored_node_keeps_front_faces() {
    let nodes = r#"[{ "name": "espelho", "children": [1] }, { "mesh": 0, "scale": [-1, 1, 1] }]"#;
    let kit = load_kit(&triangle_kit("mirrored.glb", nodes, [0, 1, 2])).unwrap();
    let mesh = &kit.piece("espelho").unwrap().primitives[0].mesh;
    let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[mesh.indices[k] as usize]);
    assert!(mesh.vertices.iter().any(|v| v.pos == [-1.0, 0.0, 0.0]));
    let (u, v) = (sub(b.pos, a.pos), sub(c.pos, a.pos));
    let winding = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    for vertex in [a, b, c] {
        let agreement: f32 = winding.iter().zip(vertex.normal).map(|(w, n)| w * n).sum();
        assert!(
            agreement > 0.0,
            "winding {winding:?}, normal {:?}",
            vertex.normal
        );
    }
}

#[test]
fn out_of_range_index_is_error() {
    let nodes = r#"[{ "name": "caco", "mesh": 0 }]"#;
    let err = load_kit(&triangle_kit("bad_index.glb", nodes, [0, 1, 5])).unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
    assert!(err.to_string().contains("vertex 5"), "{err}");
}

#[test]
fn huge_accessor_count_is_error() {
    let json = r#"{
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 36 }],
        "bufferViews": [{ "buffer": 0, "byteLength": 36 }],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 4611686018427387904, "type": "VEC3" }
        ],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
        "nodes": [{ "name": "enorme", "mesh": 0 }],
        "scenes": [{ "nodes": [0] }]
    }"#;
    let err = load_kit(&write_glb("huge_count.glb", json, &[0; 36])).unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
}

#[test]
fn node_graph_must_be_trees() {
    let looped = r#"{"asset":{"version":"2.0"},"nodes":[{"name":"a","children":[0,0]}],"scenes":[{"nodes":[0]}]}"#;
    let err = load_kit(&write_glb("looped.glb", looped, &[])).unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");

    // Each node lists the next one twice: 2^64 paths if the walk followed them.
    let nodes: Vec<String> = (0..64)
        .map(|i| format!(r#"{{"children":[{0},{0}]}}"#, i + 1))
        .chain([String::from("{}")])
        .collect();
    let shared = format!(
        r#"{{"asset":{{"version":"2.0"}},"nodes":[{}],"scenes":[{{"nodes":[0]}}]}}"#,
        nodes.join(",")
    );
    let err = load_kit(&write_glb("shared.glb", &shared, &[])).unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
}

#[test]
fn truncated_glb_is_error() {
    let glb = std::fs::read(fixture("kit_min.opt.glb")).unwrap();
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("truncated.glb");
    for length in 0..glb.len() {
        std::fs::write(&path, &glb[..length]).unwrap();
        let err = load_kit(&path).unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA", "cut at {length}: {err}");
    }
}

/// The reference assets live outside the repository, with their names in
/// `$SARIEMA_REFERENCE_ASSETS/reference.json`. This test reads its `kit`:
///
/// ```json
/// { "kit": { "path": "<glb, relative to the directory>", "triangles": 0,
///            "materials": 0, "images": 0, "emissive_material": "<name>",
///            "emissive_texture": 0,
///            "piece": { "name": "<piece>", "triangles": 0,
///                       "state": "<state>", "state_triangles": 0 } } }
/// ```
#[test]
#[ignore = "needs SARIEMA_REFERENCE_ASSETS with a reference.json"]
fn loads_the_reference_kit() {
    let root = PathBuf::from(
        std::env::var_os("SARIEMA_REFERENCE_ASSETS")
            .expect("set SARIEMA_REFERENCE_ASSETS to a directory holding reference.json"),
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("reference.json")).unwrap()).unwrap();
    let want = &manifest["kit"];
    let count = |key: &str| want[key].as_u64().unwrap() as usize;
    let kit = load_kit(&root.join(want["path"].as_str().unwrap())).unwrap();
    assert_eq!(kit.triangles() as usize, count("triangles"));
    assert_eq!(kit.materials.len(), count("materials"));
    assert_eq!(kit.images.len(), count("images"));
    let emissive = kit
        .materials
        .iter()
        .find(|m| m.name == want["emissive_material"].as_str().unwrap());
    assert_eq!(
        emissive.unwrap().emissive_texture.map(|i| i as usize),
        Some(count("emissive_texture"))
    );
    let piece = &want["piece"];
    let p = kit.piece(piece["name"].as_str().unwrap()).unwrap();
    assert_eq!(
        triangles(&p.primitives),
        piece["triangles"].as_u64().unwrap() as usize
    );
    assert_eq!(
        triangles(state(p, piece["state"].as_str().unwrap())),
        piece["state_triangles"].as_u64().unwrap() as usize
    );
}
