// SPDX-License-Identifier: Apache-2.0

use sariema_format::demo::demo_room;
use sariema_format::{CharacterLight, Fog, FormatError, decode, encode, eval_sh9, hemisphere_sh9};

#[test]
fn roundtrip_demo() {
    let r = demo_room();
    assert_eq!(decode(&encode(&r)).unwrap(), r);
}

#[test]
fn rejects_bad_magic() {
    let mut b = encode(&demo_room());
    b[0] = b'X';
    assert!(matches!(decode(&b), Err(FormatError::BadMagic)));
}

#[test]
fn rejects_other_version() {
    let mut b = encode(&demo_room());
    b[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert!(matches!(decode(&b), Err(FormatError::Version { found: 1 })));
}

#[test]
fn truncated_never_panics() {
    let b = encode(&demo_room());
    for n in 0..b.len() {
        assert!(decode(&b[..n]).is_err());
    }
}

#[test]
fn huge_length_prefix_is_error_not_alloc() {
    let mut b = encode(&demo_room())[..8].to_vec();
    b.extend([0xFF; 16]);
    assert!(decode(&b).is_err());
}

#[test]
fn dangling_material_index_is_invalid() {
    let mut r = demo_room();
    r.draws[0].material = 999;
    assert!(matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))));
}

#[test]
fn dangling_emissive_texture_is_invalid() {
    let mut r = demo_room();
    r.materials[0].emissive_texture = Some(1);
    assert!(matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))));
}

#[test]
fn many_lights_are_valid() {
    let mut r = demo_room();
    r.lights = vec![r.lights[1].clone(); 200];
    assert_eq!(decode(&encode(&r)).unwrap(), r);
}

#[test]
fn demo_bounds_enclose_each_draw() {
    let r = demo_room();
    let floor = &r.draws[0].bounds;
    assert_eq!((floor.min, floor.max), ([-5.0, 0.0, -5.0], [5.0, 0.0, 5.0]));
    // box_red: a unit cube standing at (-2.5, 0, -1).
    let red = &r.draws[1].bounds;
    assert_eq!((red.min, red.max), ([-3.0, 0.0, -1.5], [-2.0, 1.0, -0.5]));
}

#[test]
fn bad_bounds_are_invalid() {
    let mut r = demo_room();
    r.draws[0].bounds.min[1] = 1.0;
    assert!(matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))));
    let mut r = demo_room();
    r.draws[0].bounds.max[0] = f32::NAN;
    assert!(matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))));
}

#[test]
fn bad_importance_is_invalid() {
    for importance in [-0.5, f32::NAN, f32::INFINITY] {
        let mut r = demo_room();
        r.lights[0].importance = importance;
        assert!(
            matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))),
            "{importance}"
        );
    }
}

#[test]
fn bad_ambient_occlusion_is_invalid() {
    for strength in [-0.1, 1.5, f32::NAN, f32::INFINITY] {
        let mut r = demo_room();
        r.look.ao_strength = strength;
        assert!(
            matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))),
            "strength {strength}"
        );
    }
    for radius in [0.0, -0.5, f32::NAN, f32::INFINITY] {
        let mut r = demo_room();
        r.look.ao_radius_m = radius;
        assert!(
            matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))),
            "radius {radius}"
        );
    }
}

#[test]
fn ambient_occlusion_limits_are_valid() {
    for (strength, radius) in [(0.0, 0.01), (1.0, 50.0)] {
        let mut r = demo_room();
        r.look.ao_strength = strength;
        r.look.ao_radius_m = radius;
        assert_eq!(decode(&encode(&r)).unwrap(), r);
    }
}

#[test]
fn hemisphere_sh_matches_endpoints() {
    let sh = hemisphere_sh9([0.2, 0.4, 0.8], [0.1, 0.1, 0.0], 0.5);
    let up = eval_sh9(&sh, [0.0, 1.0, 0.0]);
    let down = eval_sh9(&sh, [0.0, -1.0, 0.0]);
    let side = eval_sh9(&sh, [1.0, 0.0, 0.0]);
    for (a, e) in up.iter().zip([0.1, 0.2, 0.4]) {
        assert!((a - e).abs() < 1e-5);
    }
    for (a, e) in down.iter().zip([0.05, 0.05, 0.0]) {
        assert!((a - e).abs() < 1e-5);
    }
    for (a, e) in side.iter().zip([0.075, 0.125, 0.2]) {
        assert!((a - e).abs() < 1e-5);
    }
}

#[test]
fn a_character_light_round_trips() {
    let mut r = demo_room();
    r.character = CharacterLight {
        direction: [-0.2, 0.5, 1.0],
        color: [1.0, 0.9, 0.8],
        intensity: 0.7,
        wrap: 0.3,
        rim: 0.4,
        rim_power: 2.0,
        min_brightness: 0.2,
        room_tint: 0.25,
    };
    assert_eq!(decode(&encode(&r)).unwrap(), r);
    assert_eq!(demo_room().character, CharacterLight::default());
}

#[test]
fn a_bad_character_light_is_invalid() {
    let bad = [
        CharacterLight {
            direction: [0.0; 3],
            ..CharacterLight::default()
        },
        CharacterLight {
            intensity: -1.0,
            ..CharacterLight::default()
        },
        CharacterLight {
            room_tint: 1.5,
            ..CharacterLight::default()
        },
        CharacterLight {
            rim: f32::NAN,
            ..CharacterLight::default()
        },
    ];
    for character in bad {
        let mut r = demo_room();
        r.character = character.clone();
        assert!(
            matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))),
            "{character:?}"
        );
    }
}

#[test]
fn fog_round_trips_and_is_off_by_default() {
    let mut r = demo_room();
    assert_eq!(r.fog, Fog::default());
    assert_eq!(r.fog.density, 0.0);
    assert!(r.fog.end <= r.fog.start, "no ramp by default");
    r.fog = Fog {
        color: [0.4, 0.45, 0.5],
        density: 0.08,
        start: 2.0,
        end: 30.0,
    };
    assert_eq!(decode(&encode(&r)).unwrap(), r);
}

#[test]
fn a_bad_fog_is_invalid() {
    let bad = [
        Fog {
            density: -0.1,
            ..Fog::default()
        },
        Fog {
            start: -1.0,
            ..Fog::default()
        },
        Fog {
            color: [f32::NAN, 0.0, 0.0],
            ..Fog::default()
        },
        Fog {
            end: f32::INFINITY,
            ..Fog::default()
        },
    ];
    for fog in bad {
        let mut r = demo_room();
        r.fog = fog;
        assert!(
            matches!(decode(&encode(&r)), Err(FormatError::Invalid(_))),
            "{fog:?}"
        );
    }
}
