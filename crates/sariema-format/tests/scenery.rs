// SPDX-License-Identifier: Apache-2.0

use sariema_format::demo::demo_room;
use sariema_format::scenery::{Channel, LightField, RoomIndex, Target, sun_angles, sun_direction};
use sariema_format::{
    Fog, FormatError, Group, Hinge, Node, Piece, PieceState, decode, encode, eval_sh9,
    hemisphere_sh9,
};

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn scenery_room() -> sariema_format::RoomPack {
    let mut room = demo_room();
    room.groups = vec![
        Group {
            name: "west".into(),
            on: true,
            draws: vec![1, 2],
            lights: vec![0],
        },
        Group {
            name: "east".into(),
            on: false,
            draws: vec![3],
            lights: vec![1],
        },
    ];
    room.nodes = vec![Node {
        name: "leaf".into(),
        pivot: IDENTITY,
        draws: vec![2],
        visible: true,
        hinge: Some(Hinge {
            axis: [0.0, 1.0, 0.0],
            angle_deg: 15.0,
        }),
    }];
    room.pieces = vec![Piece {
        name: "screen".into(),
        states: vec![
            PieceState {
                name: String::new(),
                draws: vec![4],
            },
            PieceState {
                name: "awake".into(),
                draws: vec![5],
            },
        ],
        state: 0,
    }];
    room
}

fn invalid(room: &sariema_format::RoomPack) -> String {
    match decode(&encode(room)) {
        Err(FormatError::Invalid(why)) => why,
        other => panic!("expected an invalid pack, got {other:?}"),
    }
}

#[test]
fn groups_nodes_and_pieces_round_trip() {
    let room = scenery_room();
    assert_eq!(decode(&encode(&room)).unwrap(), room);
}

#[test]
fn a_draw_in_two_groups_is_invalid() {
    let mut room = scenery_room();
    room.groups[1].draws.push(1);
    assert!(invalid(&room).contains("draw 1 is in both group west and group east"));
}

#[test]
fn a_light_in_two_groups_is_invalid() {
    let mut room = scenery_room();
    room.groups[1].lights.push(0);
    assert!(invalid(&room).contains("light 0 is in both"));
}

#[test]
fn dangling_indices_are_invalid() {
    let mut room = scenery_room();
    room.groups[0].draws.push(99);
    assert!(invalid(&room).contains("holds draw 99"));
    let mut room = scenery_room();
    room.groups[0].lights.push(7);
    assert!(invalid(&room).contains("holds light 7"));
    let mut room = scenery_room();
    room.pieces[0].state = 2;
    assert!(invalid(&room).contains("starts in state 2"));
}

#[test]
fn names_repeat_nowhere() {
    let mut room = scenery_room();
    room.groups[1].name = "west".into();
    assert!(invalid(&room).contains("two groups are named \"west\""));
    let mut room = scenery_room();
    room.pieces[0].states[1].name = String::new();
    assert!(invalid(&room).contains("two state of piece screens"));
}

#[test]
fn a_hinge_axis_must_be_a_unit_vector() {
    let mut room = scenery_room();
    room.nodes[0].hinge.as_mut().unwrap().axis = [0.0, 2.0, 0.0];
    assert!(invalid(&room).contains("unit length"));
}

#[test]
fn the_index_names_every_switch_and_parameter() {
    let room = scenery_room();
    let index = RoomIndex::of(&room);
    assert_eq!(
        index.groups,
        vec![("west".to_string(), true), ("east".to_string(), false)]
    );
    assert_eq!(index.nodes[0].name, "leaf");
    assert_eq!(index.pieces[0].states, vec!["".to_string(), "awake".into()]);
    let lights: Vec<&str> = index.lights.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(lights, ["spot", "point"]);
    assert_eq!(index.lights[1].position, room.lights[1].position);
    let param = |name: &str| {
        index
            .params
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no parameter {name}"))
    };
    assert_eq!(param("look.exposure").value, room.look.exposure);
    assert_eq!(param("hinge.leaf").value, 15.0);
    assert_eq!(param("hinge.leaf").target, Target::Hinge(0));
    assert_eq!(
        param("light.point.intensity").target,
        Target::Light(1, LightField::Intensity)
    );
    assert_eq!(param("light.point.intensity").value, 4.0);
    assert_eq!(param("ambient.intensity").value, 1.0);
    assert_eq!(param("fog.density").value, 0.0);
    let mut names: Vec<&str> = index.params.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), index.params.len(), "parameter names repeat");
}

#[test]
fn sun_angles_and_direction_invert_each_other() {
    for direction in [[0.0, 1.0, 0.0001], [0.3, 0.8, -0.5], [-0.6, 0.2, 0.77]] {
        let length = direction.iter().map(|v: &f32| v * v).sum::<f32>().sqrt();
        let unit = direction.map(|v| v / length);
        let (azimuth, elevation) = sun_angles(direction);
        let back = sun_direction(azimuth, elevation);
        for k in 0..3 {
            assert!((back[k] - unit[k]).abs() < 1e-5, "{back:?} vs {unit:?}");
        }
    }
}

#[test]
fn the_index_holds_the_rooms_fog_and_its_hemisphere() {
    let mut room = scenery_room();
    room.fog = Fog {
        color: [0.3, 0.35, 0.4],
        density: 0.05,
        start: 1.5,
        end: 25.0,
    };
    let (sky, ground) = ([0.6, 0.7, 0.9], [0.2, 0.15, 0.1]);
    room.ambient.sh9 = hemisphere_sh9(sky, ground, 0.8);
    room.ambient.sh9[4] = [0.05, 0.0, 0.02];
    let index = RoomIndex::of(&room);
    let param = |name: &str| {
        let p = (index.params.iter())
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no parameter {name}"));
        (p.value, p.target)
    };
    assert_eq!(param("fog.density"), (0.05, Target::FogDensity));
    assert_eq!(param("fog.start"), (1.5, Target::FogStart));
    assert_eq!(param("fog.end"), (25.0, Target::FogEnd));
    assert_eq!(param("fog.color.g"), (0.35, Target::FogColor(Channel::G)));
    assert_eq!(param("hemisphere.intensity"), (1.0, Target::Hemisphere));
    for (k, (n, c)) in [("r", Channel::R), ("g", Channel::G), ("b", Channel::B)]
        .into_iter()
        .enumerate()
    {
        let (value, target) = param(&format!("hemisphere.sky.{n}"));
        assert_eq!(target, Target::HemisphereSky(c));
        assert!((value - 0.8 * sky[k]).abs() < 1e-6, "{value}");
        let (value, target) = param(&format!("hemisphere.ground.{n}"));
        assert_eq!(target, Target::HemisphereGround(c));
        assert!((value - 0.8 * ground[k]).abs() < 1e-6, "{value}");
    }
    // What the hemisphere reads is what the ambient gives straight up and down.
    let up = eval_sh9(&room.ambient.sh9, [0.0, 1.0, 0.0]);
    let down = eval_sh9(&room.ambient.sh9, [0.0, -1.0, 0.0]);
    let band2 = |n: [f32; 3]| {
        let mut only = [[0.0; 3]; 9];
        only[4..].copy_from_slice(&room.ambient.sh9[4..]);
        eval_sh9(&only, n)
    };
    for k in 0..3 {
        let sky_read = param(["hemisphere.sky.r", "hemisphere.sky.g", "hemisphere.sky.b"][k]).0;
        let ground_read = param(
            [
                "hemisphere.ground.r",
                "hemisphere.ground.g",
                "hemisphere.ground.b",
            ][k],
        )
        .0;
        let up_k = up[k] - band2([0.0, 1.0, 0.0])[k];
        let down_k = down[k] - band2([0.0, -1.0, 0.0])[k];
        assert!((sky_read - up_k).abs() < 1e-5, "{sky_read} vs {up_k}");
        assert!(
            (ground_read - down_k).abs() < 1e-5,
            "{ground_read} vs {down_k}"
        );
    }
}
