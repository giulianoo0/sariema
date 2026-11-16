// SPDX-License-Identifier: Apache-2.0

//! Cutaways: the draws that step aside for the camera, and the line that
//! follows the hero across the floor.

use sariema_format::demo::demo_room;
use sariema_format::scenery::{RoomIndex, Target};
use sariema_format::{Area, CutLine, CutStyle, Cutaway, RoomPack, decode, encode};

fn draw_named(room: &RoomPack, name: &str) -> u32 {
    let at = room
        .draws
        .iter()
        .position(|d| d.name.as_deref() == Some(name));
    at.unwrap() as u32
}

fn walls(room: &RoomPack) -> Cutaway {
    Cutaway {
        name: "walls".into(),
        draws: vec![draw_named(room, "box_red"), draw_named(room, "box_green")],
        height: 0.4,
        line: CutLine {
            areas: vec![Area {
                min: [-2.0, -2.0],
                max: [2.0, 1.0],
            }],
            offset: -0.1,
            follow: 0.2,
        },
        radius: 0.0,
        soft: 0.25,
        cap: Some([0.2, 0.21, 0.23]),
        cut_shadows: true,
        anchors: vec![None, Some([1.0, 0.5])],
        style: CutStyle::Cut,
    }
}

#[test]
fn cutaways_round_trip_and_a_room_has_none_by_default() {
    let mut room = demo_room();
    assert!(room.cutaways.is_empty());
    room.cutaways.push(walls(&room));
    room.cutaways[0].style = CutStyle::Fade;
    assert_eq!(decode(&encode(&room)).unwrap(), room);
}

#[test]
fn a_bad_cutaway_is_invalid() {
    let room = demo_room();
    let good = walls(&room);
    let mut twice = walls(&room);
    twice.name = "again".into();
    let cases: Vec<(&str, Vec<Cutaway>)> = vec![
        (
            "a draw out of range",
            vec![Cutaway {
                draws: vec![room.draws.len() as u32, 0],
                ..good.clone()
            }],
        ),
        ("a draw in two cutaways", vec![good.clone(), twice]),
        (
            "two cutaways of one name",
            vec![
                good.clone(),
                Cutaway {
                    draws: Vec::new(),
                    anchors: Vec::new(),
                    ..good.clone()
                },
            ],
        ),
        (
            "a negative radius",
            vec![Cutaway {
                radius: -1.0,
                ..good.clone()
            }],
        ),
        (
            "a negative soft",
            vec![Cutaway {
                soft: -0.1,
                ..good.clone()
            }],
        ),
        (
            "a height not finite",
            vec![Cutaway {
                height: f32::NAN,
                ..good.clone()
            }],
        ),
        (
            "a negative follow",
            vec![Cutaway {
                line: CutLine {
                    follow: -1.0,
                    ..good.line.clone()
                },
                ..good.clone()
            }],
        ),
        (
            "an area turned inside out",
            vec![Cutaway {
                line: CutLine {
                    areas: vec![Area {
                        min: [1.0, 0.0],
                        max: [0.0, 1.0],
                    }],
                    ..good.line.clone()
                },
                ..good.clone()
            }],
        ),
        (
            "an anchor short of the draws",
            vec![Cutaway {
                anchors: vec![None],
                ..good.clone()
            }],
        ),
        (
            "an anchor not finite",
            vec![Cutaway {
                anchors: vec![None, Some([f32::NAN, 0.0])],
                ..good.clone()
            }],
        ),
        (
            "a cap not finite",
            vec![Cutaway {
                cap: Some([f32::INFINITY, 0.0, 0.0]),
                ..good.clone()
            }],
        ),
    ];
    for (what, cutaways) in cases {
        let bad = RoomPack {
            cutaways,
            ..demo_room()
        };
        let err = decode(&encode(&bad)).unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA", "{what}: {err}");
    }
}

/// Across the floor towards a camera turned `yaw` degrees about +Y from
/// looking down −Z.
fn toward(yaw: f32) -> [f32; 2] {
    let yaw = yaw.to_radians();
    [yaw.sin(), yaw.cos()]
}

#[test]
fn the_line_sits_at_the_near_edge_of_the_area_the_hero_stands_in() {
    let line = CutLine {
        areas: vec![
            Area {
                min: [0.0, 0.0],
                max: [4.0, 3.0],
            },
            Area {
                min: [0.0, 3.0],
                max: [4.0, 7.0],
            },
            // A nook inside the second: the smaller area wins.
            Area {
                min: [3.0, 5.0],
                max: [4.0, 6.0],
            },
        ],
        offset: -0.1,
        follow: 0.0,
    };
    let near = |hero, yaw| line.at(hero, toward(yaw));
    assert_eq!(near([1.0, 1.0], 0.0), Some(3.0 - 0.1));
    assert_eq!(near([1.0, 4.0], 0.0), Some(7.0 - 0.1));
    assert_eq!(near([3.5, 5.5], 0.0), Some(6.0 - 0.1));
    // Seen from +X, the near edge is the area's largest x.
    let seen_from_x = near([1.0, 1.0], 90.0).unwrap();
    assert!((seen_from_x - (4.0 - 0.1)).abs() < 1e-5, "{seen_from_x}");
    assert_eq!(near([9.0, 9.0], 0.0), None, "in no area, the line stays");
}

#[test]
fn without_areas_the_line_sits_by_the_hero() {
    let line = CutLine {
        areas: Vec::new(),
        offset: -0.5,
        follow: 0.0,
    };
    assert_eq!(line.at([2.0, 5.0], toward(0.0)), Some(5.0 - 0.5));
    let behind = line.at([2.0, 5.0], toward(180.0)).unwrap();
    assert!((behind - (-5.0 - 0.5)).abs() < 1e-5, "{behind}");
}

#[test]
fn the_index_holds_each_cutaways_line_and_its_height_as_a_parameter() {
    let mut room = demo_room();
    room.cutaways.push(walls(&room));
    let index = RoomIndex::of(&room);
    assert_eq!(index.cutaways.len(), 1);
    assert_eq!(index.cutaways[0].name, "walls");
    assert_eq!(index.cutaways[0].line, walls(&room).line);
    let height = (index.params.iter())
        .find(|p| p.name == "cutaway.walls.height")
        .unwrap();
    assert_eq!(height.value, 0.4);
    assert_eq!(height.target, Target::CutawayHeight(0));
}
