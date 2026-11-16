// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use common::write_glb;
use sariema_format::{
    AlphaMode, Draw, LightKind, Look, Mobility, RoomPack, Shading, TextureFormat, decode, encode,
    eval_sh9,
};
use sariema_import::{ImportError, RoomSource, build_room};
use serde_json::{Value, json};

mod common;

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(fixtures().join(name)).unwrap()).unwrap()
}

fn sala_min() -> Value {
    fixture("sala_min.json")
}

/// `sala` with the top-level keys of `changes` replaced.
fn with(mut sala: Value, changes: Value) -> Value {
    for (key, value) in changes.as_object().unwrap() {
        sala[key] = value.clone();
    }
    sala
}

/// `sala_min` with `key` pointing at a copy of the fixture `file` changed by `edit`.
fn with_edited(key: &str, file: &str, tag: &str, edit: impl FnOnce(&mut Value)) -> Value {
    let mut value = fixture(file);
    edit(&mut value);
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{tag}_{file}"));
    std::fs::write(&path, value.to_string()).unwrap();
    with(sala_min(), json!({ key: path }))
}

fn instance(piece: &str, rz: f32, rot: Option<[f32; 3]>, scale: f32) -> Value {
    json!({
        "peca": piece, "x": 0.0, "y": 0.0, "z": 0.0, "rz": rz, "rot": rot,
        "estado": null, "giros": null, "escala": scale
    })
}

fn montagem_with(piece: &str, room: &str) -> Value {
    with_edited("montagem", "montagem_min.json", piece, |m| {
        let instances = m["comodos"][room]["instancias"].as_array_mut().unwrap();
        instances.push(instance(piece, 0.0, None, 1.0));
    })
}

/// `pos` is in the room's Blender coordinates: x east, y north, z up.
fn point(name: &str, pos: [f32; 3], intensity: f32, range: f32) -> Value {
    json!({ "nome": name, "tipo": "ponto", "cor": "#FFFFFF", "int": intensity, "pos": pos, "dist": range })
}

/// `sala_min` with room `a` lit by `n` point lights.
fn luz_with_n(n: usize) -> Value {
    let lights = (0..n)
        .map(|i| point(&format!("l{i}"), [1.5, 1.5, 3.0], 1.0, 10.0))
        .collect();
    with_edited("luz", "luz_min.json", &format!("n{n}"), |l| {
        l["comodos"]["a"]["luzes"] = Value::Array(lights);
    })
}

fn build_in(sala: &Value, root: &Path) -> Result<RoomPack, ImportError> {
    build_room(&RoomSource::from_json(&sala.to_string())?, root)
}

fn build(sala: Value) -> RoomPack {
    build_in(&sala, &fixtures()).unwrap()
}

fn build_err(sala: Value) -> ImportError {
    build_in(&sala, &fixtures()).unwrap_err()
}

fn names(r: &RoomPack) -> Vec<&str> {
    r.draws.iter().filter_map(|d| d.name.as_deref()).collect()
}

fn draw<'a>(r: &'a RoomPack, name: &str) -> &'a Draw {
    r.draws
        .iter()
        .find(|d| d.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no draw {name} in {:?}", names(r)))
}

fn triangles(r: &RoomPack, draw: &Draw) -> usize {
    r.meshes[draw.mesh as usize].indices.len() / 3
}

fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
    assert!(
        actual.len() == expected.len()
            && actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() <= tolerance),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn merges_static_pieces_by_material() {
    let r = build(sala_min());
    assert_eq!(names(&r).iter().filter(|&&n| n == "a/mat0").count(), 1);
    assert_eq!(r.stats.triangles, 36);
    assert_eq!(draw(&r, "a/mat0").transform, IDENTITY);
}

#[test]
fn draw_bounds_enclose_vertices_after_transform() {
    let r = build(with(sala_min(), json!({ "loose": ["mesa"] })));
    let merged = draw(&r, "a/mat0");
    let bounds = &merged.bounds;
    for v in &r.meshes[merged.mesh as usize].vertices {
        assert!((0..3).all(|i| bounds.min[i] <= v.pos[i] && v.pos[i] <= bounds.max[i]));
    }
    // The two boxes: the first spans 0.25..1.25 east, the second is turned 45°.
    let half_diagonal = 0.5 * 2f32.sqrt();
    assert_close(&bounds.min, &[0.25, 0.0, -0.75 - half_diagonal], 0.01);
    assert_close(
        &bounds.max,
        &[2.25 + half_diagonal, 1.0, -0.75 + half_diagonal],
        0.01,
    );
    // The mesa's top is a 1 m box raised 1 m, centred on its placement.
    let mesa = &draw(&r, "a.mesa").bounds;
    assert_close(&mesa.min, &[1.0, 0.5, -2.75], 0.01);
    assert_close(&mesa.max, &[2.0, 1.5, -1.75], 0.01);
}

#[test]
fn loose_pieces_keep_name_and_transform() {
    let r = build(with(sala_min(), json!({ "loose": ["mesa"] })));
    let mesa = draw(&r, "a.mesa");
    // In the montagem the mesa stands at x 1.5, y 2.25, z 0 from room a's corner.
    assert_eq!(mesa.transform[12..15], [1.5, 0.0, -2.25]);
    assert!(mesa.visible);
    assert_eq!(triangles(&r, draw(&r, "a/mat0")), 24);
}

#[test]
fn second_loose_copy_gets_suffix() {
    let r = build(with(sala_min(), json!({ "loose": ["caixa"] })));
    let copies: Vec<_> = names(&r)
        .into_iter()
        .filter(|n| n.starts_with("a.caixa"))
        .collect();
    assert_eq!(copies, ["a.caixa", "a.caixa.2"]);
}

#[test]
fn rotations_follow_blender_axes() {
    let sala = with_edited("montagem", "montagem_min.json", "rotations", |m| {
        m["comodos"]["a"]["instancias"] = json!([
            instance("caixa", 90.0, None, 1.0),
            instance("caixa", 0.0, Some([90.0, 0.0, 90.0]), 2.0),
        ]);
    });
    let r = build(with(sala, json!({ "loose": ["caixa"] })));
    // rz 90 turns east to north, which is −Z in the engine.
    assert_close(
        &draw(&r, "a.caixa").transform[0..3],
        &[0.0, 0.0, -1.0],
        1e-6,
    );
    // rot is three.js's Euler XYZ (Rx·Ry·Rz), as casa.ts reads it: [90, 0, 90]
    // tips east up and up to the south (+Z). escala is 2.
    let turned = &draw(&r, "a.caixa.2").transform;
    assert_close(&turned[0..3], &[0.0, 2.0, 0.0], 1e-6);
    assert_close(&turned[4..7], &[0.0, 0.0, 2.0], 1e-6);
}

#[test]
fn only_shown_rooms_enter() {
    let r = build(with(
        sala_min(),
        json!({ "show": ["a"], "loose": ["lampada"] }),
    ));
    assert!(names(&r).contains(&"a/mat0"));
    assert!(
        !names(&r).iter().any(|n| n.starts_with('b')),
        "{:?}",
        names(&r)
    );
}

#[test]
fn hidden_piece_is_invisible() {
    let r = build(with(
        sala_min(),
        json!({ "show": ["b"], "loose": ["lampada"] }),
    ));
    assert!(!draw(&r, "b.lampada").visible);
    assert_eq!((r.stats.triangles, r.stats.draws), (0, 0));
}

#[test]
fn listed_pieces_start_hidden() {
    let r = build(with(
        sala_min(),
        json!({ "hidden": ["mesa"], "loose": ["mesa"] }),
    ));
    assert!(!draw(&r, "a.mesa").visible);
    assert_eq!(triangles(&r, draw(&r, "a/mat0")), 24);

    // Merged pieces have no draw of their own to show later, so they are left out.
    let merged = build(with(sala_min(), json!({ "hidden": ["caixa"] })));
    assert_eq!(merged.stats.triangles, 12);
}

#[test]
fn hiding_an_unplaced_piece_names_it_and_the_room() {
    // lampada is in the kit but only room b places it.
    for piece in ["mesa_typo", "lampada"] {
        let e = build_err(with(
            sala_min(),
            json!({ "show": ["a"], "hidden": [piece] }),
        ));
        assert_eq!(e.code(), "E_ROOM_PIECE_MISSING");
        let message = e.to_string();
        assert!(
            message.contains(piece) && message.contains("sala_min"),
            "{message}"
        );
    }
}

/// A kit with one piece, `vela`, hidden at start: a 1-triangle `vela_base` and
/// a 1-triangle `vela_chama` that only its state `acesa` shows.
fn candle_kit() -> PathBuf {
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let json = r#"{
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 36 }],
        "bufferViews": [{ "buffer": 0, "byteLength": 36 }],
        "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }],
        "materials": [{ "name": "cera" }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "material": 0 }] }],
        "nodes": [
            {
                "name": "vela",
                "children": [1, 2],
                "extras": { "oculto_no_inicio": true, "estados": { "acesa": ["vela_chama"] } }
            },
            { "name": "vela_base", "mesh": 0 },
            { "name": "vela_chama", "mesh": 0, "extras": { "oculto_no_inicio": true } }
        ],
        "scenes": [{ "nodes": [0] }]
    }"#;
    write_glb("candle_kit.glb", json, bytemuck::cast_slice(&positions))
}

/// `sala_min` over [`candle_kit`], with room `a` holding a `vela` as it starts
/// and a `vela` in its state `acesa`.
fn candles(loose: &[&str]) -> Value {
    let sala = with_edited("montagem", "montagem_min.json", "candles", |m| {
        let mut lit = instance("vela", 0.0, None, 1.0);
        lit["estado"] = json!("acesa");
        m["comodos"]["a"]["instancias"] = json!([instance("vela", 0.0, None, 1.0), lit]);
    });
    with(
        sala,
        json!({ "kit": candle_kit(), "show": ["a"], "loose": loose }),
    )
}

#[test]
fn hidden_piece_shows_in_its_state() {
    // casa.ts hides a node flagged oculto_no_inicio only while no estado is set.
    let r = build(candles(&["vela"]));
    assert!(!draw(&r, "a.vela").visible);
    let lit: Vec<_> = r
        .draws
        .iter()
        .filter(|d| d.name.as_deref() == Some("a.vela.2"))
        .collect();
    assert_eq!(lit.len(), 2);
    assert!(lit.iter().all(|d| d.visible));

    let merged = build(candles(&[]));
    assert_eq!(triangles(&merged, draw(&merged, "a/cera")), 2);
}

#[test]
fn zero_or_missing_escala_is_one() {
    let sala = with_edited("montagem", "montagem_min.json", "escala", |m| {
        let mut missing = instance("caixa", 0.0, None, 1.0);
        missing.as_object_mut().unwrap().remove("escala");
        m["comodos"]["a"]["instancias"] = json!([instance("caixa", 0.0, None, 0.0), missing]);
    });
    let r = build(with(sala, json!({ "loose": ["caixa"] })));
    for name in ["a.caixa", "a.caixa.2"] {
        assert_eq!(draw(&r, name).transform, IDENTITY, "{name}");
    }
}

#[test]
fn negative_escala_names_piece_and_room() {
    let sala = with_edited("montagem", "montagem_min.json", "negative", |m| {
        m["comodos"]["a"]["instancias"] = json!([instance("caixa", 0.0, None, -1.0)]);
    });
    let e = build_err(sala);
    assert_eq!(e.code(), "E_SCHEMA");
    let message = e.to_string();
    assert!(
        message.contains("caixa") && message.contains("room a"),
        "{message}"
    );
}

#[test]
fn missing_piece_names_piece_and_room() {
    let e = build_err(montagem_with("fantasma", "a"));
    assert_eq!(e.code(), "E_ROOM_PIECE_MISSING");
    let message = e.to_string();
    assert!(
        message.contains("fantasma") && message.contains("room a"),
        "{message}"
    );
}

#[test]
fn unknown_state_is_error() {
    let sala = with_edited("montagem", "montagem_min.json", "state", |m| {
        let mut caixa = instance("caixa", 0.0, None, 1.0);
        caixa["estado"] = json!("aceso");
        m["comodos"]["a"]["instancias"] = json!([caixa]);
    });
    let e = build_err(sala);
    assert_eq!(e.code(), "E_SCHEMA");
    assert!(e.to_string().contains("no state aceso"), "{e}");
}

#[test]
fn textures_carry_mip_chains() {
    let r = build(with(sala_min(), json!({ "textures": "rgba8" })));
    let texture = &r.textures[0];
    assert_eq!(texture.format, TextureFormat::Rgba8Srgb);
    assert_eq!((texture.width, texture.height), (4, 4));
    let sizes: Vec<_> = texture.mips.iter().map(Vec::len).collect();
    assert_eq!(sizes, [64, 16, 4]);
    assert_eq!(r.stats.texture_bytes, 84);
}

#[test]
fn texture_mode_selects_format() {
    let rgba8 = build(with(sala_min(), json!({ "textures": "rgba8" })));
    let astc = build(with(sala_min(), json!({ "textures": "astc" })));
    assert_eq!(astc.textures[0].format, TextureFormat::Astc4x4Srgb);
    assert!(
        astc.stats.texture_bytes < rgba8.stats.texture_bytes,
        "{} vs {}",
        astc.stats.texture_bytes,
        rgba8.stats.texture_bytes
    );
}

#[test]
fn textures_default_to_astc() {
    let r = build(sala_min());
    assert_eq!(r.textures[0].format, TextureFormat::Astc4x4Srgb);
}

#[test]
fn material_lists_mark_materials() {
    let r = build(with(
        sala_min(),
        json!({
            "drawn_materials": ["mat0"],
            "rim_materials": [["mat0", 0.4]],
            "additive_materials": ["mat0"]
        }),
    ));
    let mat0 = &r.materials[0];
    assert!(mat0.drawn);
    assert_eq!(mat0.rim, 0.4);
    // Blending is changed; the lighting the kit declares is kept.
    assert_eq!(
        (mat0.shading, mat0.alpha),
        (Shading::Lit, AlphaMode::Additive)
    );
    assert!(r.draws.iter().all(|d| !d.casts_shadow));
}

#[test]
fn many_lights_build_and_round_trip() {
    let r = build(luz_with_n(200));
    assert_eq!(r.lights.len(), 200);
    assert_eq!(decode(&encode(&r)).unwrap(), r);
}

#[test]
fn spots_cast_shadows_and_points_do_not() {
    let r = build(sala_min());
    let casters: Vec<_> = r
        .lights
        .iter()
        .map(|l| (l.name.as_str(), l.casts_shadow))
        .collect();
    assert_eq!(casters, [("a.foco", true), ("a.abajur", false)]);
}

#[test]
fn listed_lights_cast_no_shadow() {
    let r = build(with(sala_min(), json!({ "shadowless_lights": ["a.foco"] })));
    assert!(r.lights.iter().all(|l| !l.casts_shadow));
}

#[test]
fn unknown_shadowless_light_names_it_and_the_room() {
    let e = build_err(with(sala_min(), json!({ "shadowless_lights": ["a.fogo"] })));
    assert_eq!(e.code(), "E_SCHEMA");
    let message = e.to_string();
    assert!(
        message.contains("a.fogo") && message.contains("sala_min"),
        "{message}"
    );
}

#[test]
fn lights_are_placed_like_pieces() {
    let sala = with_edited("luz", "luz_min.json", "placed", |l| {
        l["comodos"]["b"]["luzes"] = json!([point("vela", [0.5, 0.25, 1.0], 2.0, 3.0)]);
    });
    let r = build(sala);
    let light = |name: &str| r.lights.iter().find(|l| l.name == name).unwrap();

    let foco = light("a.foco");
    assert_eq!(foco.position, [1.5, 2.4, -0.2]);
    assert_eq!(
        foco.kind,
        LightKind::Spot {
            target: [1.5, 0.0, -1.5],
            cone_deg: 60.0
        }
    );
    assert_eq!((foco.intensity, foco.range), (40.0, 8.0));
    assert_eq!(foco.flicker, None);
    assert_eq!(
        light("a.abajur").flicker.as_deref(),
        Some("0.85-1.0 at ~8 Hz")
    );
    // Room b's corner is 2 cells (3 m) east of room a's.
    let vela = light("b.vela");
    assert_eq!(vela.kind, LightKind::Point);
    assert_eq!(vela.position, [3.5, 1.0, -0.25]);
    // Flicker changes intensity only, so the flickering abajur is static too.
    for l in &r.lights {
        assert_eq!(
            (l.mobility, l.importance),
            (Mobility::Static, 1.0),
            "{}",
            l.name
        );
    }
}

#[test]
fn hex_colour_is_linearised() {
    let r = build(sala_min());
    let abajur = r.lights.iter().find(|l| l.name == "a.abajur").unwrap();
    assert_close(&abajur.color, &[0.2158; 3], 0.001);
}

#[test]
fn first_shown_room_gives_ambient_and_sun() {
    let a = build(sala_min());
    let sun = a.sun.unwrap();
    assert_eq!((sun.intensity, sun.casts_shadow), (0.55, false));
    assert_eq!(sun.mobility, Mobility::Static);

    let b = build(with(sala_min(), json!({ "show": ["b", "a"] })));
    let sun = b.sun.as_ref().unwrap();
    assert_eq!((sun.intensity, sun.casts_shadow), (1.5, true));
    let length = sun.direction.iter().map(|c| c * c).sum::<f32>().sqrt();
    assert!((length - 1.0).abs() < 1e-6);
    // Room b's sky is #CFE0F0, whose red is 0.624 in linear light, at intensity 0.75.
    let up = eval_sh9(&b.ambient.sh9, [0.0, 1.0, 0.0]);
    assert!((up[0] - 0.624 * 0.75).abs() < 0.001, "{up:?}");
}

#[test]
fn extra_hemisphere_adds_to_the_ambient() {
    let extra = json!({ "sky": [0.5, 0.25, 0.5], "ground": [0.1, 0.1, 0.0], "intensity": 2.0 });
    let plain = build(sala_min());
    let filled = build(with(sala_min(), json!({ "extra_hemisphere": extra })));
    for (normal, added) in [
        ([0.0, 1.0, 0.0], [1.0, 0.5, 1.0]),
        ([0.0, -1.0, 0.0], [0.2, 0.2, 0.0]),
    ] {
        let before = eval_sh9(&plain.ambient.sh9, normal);
        let expected: Vec<f32> = before.iter().zip(added).map(|(b, a)| b + a).collect();
        assert_close(&eval_sh9(&filled.ambient.sh9, normal), &expected, 1e-5);
    }
}

#[test]
fn first_room_without_luz_gets_the_game_fallback() {
    let sala = with_edited("luz", "luz_min.json", "fallback", |l| {
        l["comodos"].as_object_mut().unwrap().remove("b");
    });
    let r = build(with(sala, json!({ "show": ["b", "a"] })));
    // casa.ts: a white directional light of 1.4 from (-0.5, 1, 0.4) and a
    // #CFE0F0 / #8A7F70 hemisphere at 0.9.
    let sun = r.sun.as_ref().unwrap();
    assert_eq!((sun.color, sun.intensity), ([1.0; 3], 1.4));
    let length = (0.25f32 + 1.0 + 0.16).sqrt();
    assert_close(
        &sun.direction,
        &[-0.5 / length, 1.0 / length, 0.4 / length],
        1e-6,
    );
    let up = eval_sh9(&r.ambient.sh9, [0.0, 1.0, 0.0]);
    assert!((up[0] - 0.624 * 0.9).abs() < 0.001, "{up:?}");
    // Room a still brings its own two lights.
    assert_eq!(r.lights.len(), 2);
}

#[test]
fn zero_sun_direction_is_error() {
    let sala = with_edited("luz", "luz_min.json", "no_sun", |l| {
        l["comodos"]["a"]["sol_dir_three"] = json!([0.0, 0.0, 0.0]);
    });
    let e = build_err(sala);
    assert_eq!(e.code(), "E_SCHEMA");
    assert!(e.to_string().contains("sol_dir_three"), "{e}");
}

#[test]
fn schema_rejects_unknown_field() {
    let e = build_err(with(sala_min(), json!({ "extra": 1 })));
    assert_eq!(e.code(), "E_SCHEMA");
}

#[test]
fn schema_rejects_malformed_room_options() {
    let sky = [0.5, 0.5, 0.5];
    for (bad, at) in [
        (
            json!({ "extra_hemisphere": { "sky": sky, "ground": sky, "intensity": -1.0 } }),
            "/extra_hemisphere/intensity",
        ),
        (
            json!({ "extra_hemisphere": { "sky": sky, "intensity": 1.0 } }),
            "/extra_hemisphere",
        ),
        (json!({ "hidden": "mesa" }), "/hidden"),
        (
            json!({ "shadowless_lights": "a.foco" }),
            "/shadowless_lights",
        ),
    ] {
        let e = build_err(with(sala_min(), bad));
        assert_eq!(e.code(), "E_SCHEMA");
        assert!(e.to_string().contains(at), "{e}");
    }
}

#[test]
fn ambient_occlusion_is_optional_in_the_look() {
    let r = build(sala_min());
    assert_eq!(r.look.ao_strength, Look::DEFAULT_AO_STRENGTH);
    assert_eq!(r.look.ao_radius_m, Look::DEFAULT_AO_RADIUS_M);
    let mut sala = sala_min();
    sala["look"]["ao_strength"] = json!(0.0);
    sala["look"]["ao_radius_m"] = json!(0.25);
    let r = build(sala);
    assert_eq!((r.look.ao_strength, r.look.ao_radius_m), (0.0, 0.25));
}

#[test]
fn schema_rejects_ambient_occlusion_out_of_range() {
    for (key, value) in [
        ("ao_strength", 1.2),
        ("ao_strength", -0.1),
        ("ao_radius_m", 0.0),
    ] {
        let mut sala = sala_min();
        sala["look"][key] = json!(value);
        let e = build_err(sala);
        assert_eq!(e.code(), "E_SCHEMA");
        assert!(e.to_string().contains(&format!("/look/{key}")), "{e}");
    }
}

#[test]
fn result_survives_format_roundtrip() {
    let r = build(sala_min());
    assert_eq!(decode(&encode(&r)).unwrap(), r);
}

#[test]
#[ignore = "needs the reference assets in SARIEMA_REFERENCE_ASSETS"]
fn builds_real_quintal() {
    let root = std::env::var_os("SARIEMA_REFERENCE_ASSETS").expect(
        "set SARIEMA_REFERENCE_ASSETS to the reference assets directory, the one holding casa/",
    );
    let sala = with(
        sala_min(),
        json!({
            "kit": "casa/kit_casa_pain.opt.glb",
            "montagem": "casa/montagem.json",
            "luz": "casa/luz.json",
            "show": ["quintal"]
        }),
    );
    let r = build_in(&sala, Path::new(&root)).unwrap();
    // montagem.json declares 41 instances and 13 218 triangles in the quintal.
    assert_eq!(r.stats.triangles, 13_218);
    assert_eq!(r.lights.len(), 2);
}

#[test]
#[ignore = "needs the reference assets in SARIEMA_REFERENCE_ASSETS"]
fn real_cozinha_shows_instance_states() {
    let root = std::env::var_os("SARIEMA_REFERENCE_ASSETS").expect(
        "set SARIEMA_REFERENCE_ASSETS to the reference assets directory, the one holding casa/",
    );
    let sala = with(
        sala_min(),
        json!({
            "kit": "casa/kit_casa_pain.opt.glb",
            "montagem": "casa/montagem.json",
            "luz": "casa/luz.json",
            "show": ["cozinha"]
        }),
    );
    let r = build_in(&sala, Path::new(&root)).unwrap();
    // montagem.json declares 10 302. Its geladeira_real is `aberta` and its
    // pacote_batata `caido`; as they start, the cozinha would have 10 148.
    assert_eq!(r.stats.triangles, 10_302);
}

#[test]
#[ignore = "needs the reference assets in SARIEMA_REFERENCE_ASSETS"]
fn builds_whole_tree_house() {
    let root = std::env::var_os("SARIEMA_REFERENCE_ASSETS").expect(
        "set SARIEMA_REFERENCE_ASSETS to the reference assets directory, the one holding arvore/",
    );
    let show = [
        "quarto_janela",
        "tunel_entrada",
        "salao",
        "tunel_1",
        "tunel_2",
        "cozinha",
        "sala_estar",
        "tunel_alto",
        "sala_mila",
    ];
    let sala = with(
        sala_min(),
        json!({
            "kit": "arvore/kit_casa_arvore.opt.glb",
            "montagem": "arvore/montagem.json",
            "luz": "arvore/luz.json",
            "show": show
        }),
    );
    let r = build_in(&sala, Path::new(&root)).unwrap();
    // montagem.json declares 46 013 triangles over the nine rooms, and luz.json 21 lights.
    assert_eq!(r.stats.triangles, 46_013);
    assert_eq!(r.lights.len(), 21);
}
