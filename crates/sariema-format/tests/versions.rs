// SPDX-License-Identifier: Apache-2.0

use sariema_format::demo::demo_room;
use sariema_format::scenery::{CutawayIndex, Param, Target};
use sariema_format::script::cutscene::{Block, Cutscene, PathSpan, Place, Role, Then};
use sariema_format::script::{
    GAME_MAGIC, GameFormatError, GamePack, GameRoom, decode_game, encode_game,
};
use sariema_format::stream::RoomDecoder;
use sariema_format::{
    CharacterLight, CutAnchor, CutLine, CutStyle, Cutaway, Fog, MAGIC, OLDEST_VERSION, RoomPack,
    VERSION, decode,
};
use serde::Serialize;

/// `room` as a pack of `version` writes it: its fields up to the last that
/// version has, in order.
fn written_by(version: u32, room: &RoomPack) -> Vec<u8> {
    let head = (
        &room.name,
        &room.meshes,
        &room.materials,
        &room.textures,
        &room.draws,
        &room.lights,
        &room.ambient,
        &room.sun,
        &room.background,
    );
    let body = (&room.look, &room.camera, &room.stats, &room.groups);
    let scenery = (&room.nodes, &room.pieces);
    let mut out = [&MAGIC[..], &version.to_le_bytes()].concat();
    let mut put = |value: &dyn erased::Put| value.put(&mut out);
    put(&head);
    put(&body);
    put(&scenery);
    if version >= 7 {
        put(&room.character);
    }
    if version >= 8 {
        put(&room.fog);
    }
    if version == 9 {
        let point = |a: &Option<CutAnchor>| a.map(|a| a.at);
        let cutaways: Vec<_> = (room.cutaways.iter())
            .map(|c| {
                let anchors: Vec<_> = c.anchors.iter().map(point).collect();
                let rule = (c.radius, c.soft, c.cap, c.cut_shadows);
                (&c.name, &c.draws, c.height, &c.line, rule, anchors, c.style)
            })
            .collect();
        put(&cutaways);
    }
    if version >= 10 {
        put(&room.cutaways);
    }
    out
}

mod erased {
    pub trait Put {
        fn put(&self, out: &mut Vec<u8>);
    }

    impl<T: serde::Serialize> Put for T {
        fn put(&self, out: &mut Vec<u8>) {
            let bytes = postcard::to_allocvec(self).unwrap();
            out.extend_from_slice(&bytes);
        }
    }
}

/// The demo room as a version before the character light, fog and cutaways
/// held: none of them, and the fog taking the background's bottom.
fn as_of(version: u32) -> RoomPack {
    let mut room = demo_room();
    room.cutaways.clear();
    if version < 8 {
        room.fog = Fog {
            color: room.background.bottom,
            ..Fog::default()
        };
    }
    if version < 7 {
        room.character = CharacterLight::default();
    }
    room
}

#[test]
fn a_room_pack_of_every_version_since_the_oldest_still_loads() {
    assert_eq!(
        written_by(VERSION, &demo_room()),
        sariema_format::encode(&demo_room())
    );
    for version in OLDEST_VERSION..VERSION {
        let room = as_of(version);
        let bytes = written_by(version, &room);
        assert_eq!(decode(&bytes).unwrap(), room, "version {version}");
        assert_eq!(
            RoomDecoder::decode(&bytes[..]).unwrap(),
            room,
            "version {version}"
        );
        let mut decoder = RoomDecoder::new(&bytes[..]);
        let shared = loop {
            if let Some(room) = decoder.step(256).unwrap() {
                break room;
            }
        };
        assert_eq!(shared, room, "version {version} a share at a time");
        let torn = &bytes[..bytes.len() - 1];
        assert!(decode(torn).is_err() && RoomDecoder::decode(torn).is_err());
    }
}

#[test]
fn a_cutaway_of_version_9_keeps_judging_a_draw_on_a_wall_whole_at_its_point() {
    let mut room = demo_room();
    room.cutaways.push(Cutaway {
        name: "walls".into(),
        draws: vec![1, 2],
        height: 0.4,
        line: CutLine::default(),
        radius: 0.0,
        soft: 0.1,
        cap: Some([0.1, 0.1, 0.1]),
        cut_shadows: true,
        anchors: vec![
            None,
            Some(CutAnchor {
                at: [1.0, 0.5],
                along: [0.0; 2],
            }),
        ],
        style: CutStyle::Cut,
    });
    let bytes = written_by(9, &room);
    assert_eq!(decode(&bytes).unwrap(), room);
    assert_eq!(RoomDecoder::decode(&bytes[..]).unwrap(), room);
}

#[test]
fn a_room_pack_older_than_the_oldest_or_newer_is_refused() {
    for version in [OLDEST_VERSION - 1, VERSION + 1] {
        let bytes = written_by(version, &as_of(OLDEST_VERSION));
        let error = decode(&bytes).unwrap_err();
        assert_eq!(error.code(), "E_PACK_VERSION");
        assert_eq!(RoomDecoder::decode(&bytes[..]).unwrap_err(), error);
    }
}

#[derive(Serialize)]
struct RoomIndex3<'a> {
    groups: &'a [(String, bool)],
    nodes: &'a [sariema_format::scenery::NodeIndex],
    pieces: &'a [sariema_format::scenery::PieceIndex],
    lights: &'a [sariema_format::scenery::LightIndex],
    params: &'a [Param],
}

#[derive(Serialize)]
struct GameRoom3<'a> {
    name: &'a str,
    room: &'a Option<Vec<u8>>,
    collision: &'a Option<Vec<u8>>,
    index: RoomIndex3<'a>,
    neighbours: &'a [u32],
    beside: &'a [u32],
    ambience: &'a Option<String>,
}

/// `game` as version 3 writes it: its room index ends at the parameters.
fn written_by_3(game: &GamePack) -> Vec<u8> {
    let rooms: Vec<GameRoom3> = (game.assets.rooms.iter())
        .map(|r| GameRoom3 {
            name: &r.name,
            room: &r.room,
            collision: &r.collision,
            index: RoomIndex3 {
                groups: &r.index.groups,
                nodes: &r.index.nodes,
                pieces: &r.index.pieces,
                lights: &r.index.lights,
                params: &r.index.params,
            },
            neighbours: &r.neighbours,
            beside: &r.beside,
            ambience: &r.ambience,
        })
        .collect();
    let a = &game.assets;
    let body = (
        &game.name,
        game.batch_version,
        &game.wasm,
        &game.cutscenes,
        &game.names,
        &game.settings,
        (&rooms, &a.actors, &a.sounds, &a.hud),
    );
    let mut out = [&GAME_MAGIC[..], &3u32.to_le_bytes()].concat();
    out.extend(postcard::to_allocvec(&body).unwrap());
    out
}

fn game() -> GamePack {
    let room = as_of(OLDEST_VERSION);
    GamePack {
        name: "old".into(),
        batch_version: sariema_format::script::BATCH_VERSION,
        wasm: b"\0asm\x01\0\0\0".to_vec(),
        cutscenes: Vec::new(),
        names: Vec::new(),
        settings: "{}".into(),
        assets: sariema_format::script::GameAssets {
            rooms: vec![GameRoom::of("porch", &room)],
            ..Default::default()
        },
    }
}

#[test]
fn a_game_pack_of_version_3_still_loads_with_rooms_without_cutaways() {
    let game = game();
    assert!(!game.assets.rooms[0].index.params.is_empty());
    assert_eq!(decode_game(&written_by_3(&game)).unwrap(), game);
    let mut version = written_by_3(&game);
    version[4] = 2;
    assert_eq!(
        decode_game(&version).unwrap_err(),
        GameFormatError::Version { found: 2 }
    );
}

#[test]
fn a_room_index_with_a_cutaway_lacking_its_height_or_out_of_range_is_refused() {
    let invalid = |g: &GamePack| match decode_game(&encode_game(g)) {
        Err(e @ GameFormatError::Invalid(_)) => {
            assert_eq!(e.code(), "E_SCHEMA");
            e.to_string()
        }
        other => panic!("expected an invalid pack, got {other:?}"),
    };
    let cutaway = CutawayIndex {
        name: "front".into(),
        line: CutLine::default(),
    };
    let height = |c: u32| Param {
        name: "cutaway.front.height".into(),
        value: 1.0,
        target: Target::CutawayHeight(c),
    };

    let mut g = game();
    g.assets.rooms[0].index.cutaways.push(cutaway.clone());
    assert!(invalid(&g).contains("cutaway front has 0 height parameters"));
    g.assets.rooms[0].index.params.push(height(0));
    assert_eq!(decode_game(&encode_game(&g)).unwrap(), g);
    g.assets.rooms[0].index.params.push(height(0));
    assert!(invalid(&g).contains("cutaway front has 2 height parameters"));

    let mut g = game();
    g.assets.rooms[0].index.params.push(height(1));
    assert!(invalid(&g).contains("names cutaway 1 of 0"));

    let mut g = game();
    let index = &mut g.assets.rooms[0].index;
    index.cutaways.push(cutaway);
    index.params.push(height(0));
    index.cutaways[0].line.offset = f32::NAN;
    assert!(invalid(&g).contains("not finite"));

    let mut g = game();
    let lights = g.assets.rooms[0].index.lights.len() as u32;
    g.assets.rooms[0].index.params.push(Param {
        name: "light.ghost.intensity".into(),
        value: 1.0,
        target: Target::Light(lights, sariema_format::scenery::LightField::Intensity),
    });
    assert!(invalid(&g).contains(&format!("names light {lights}")));
}

#[test]
fn cutscene_blocks_keep_their_wire_numbers() {
    let number = |block: Block| postcard::to_allocvec(&block).unwrap()[0];
    let shake = Block::CameraShake {
        amplitude: 0.1,
        frequency: 10.0,
        duration: 0.5,
    };
    let hinge = Block::Hinge {
        hinge: 7,
        role: 0,
        clip: 9,
        span: [0.25, 0.5],
        to: 90.0,
        ease: 0,
    };
    let path = Block::Path {
        role: 0,
        clip: 9,
        mode: 2,
        blend: 0.25,
        from: 0.0,
        hold: 0.0,
        spans: Vec::new(),
        then: Vec::new(),
    };
    let scene = Block::Scene {
        values: Vec::new(),
        duration: 1.0,
        ease: 0,
    };
    assert_eq!(number(Block::Sequence(Vec::new())), 0);
    assert_eq!(number(shake), 23);
    assert_eq!(number(hinge), 24);
    assert_eq!(number(path), 25);
    assert_eq!(number(scene), 26);
}

/// A cutscene with a `hinge` step and a `path` step, byte by byte as
/// game pack 3 stores it.
fn hinge_and_path_as_written_by_3() -> Vec<u8> {
    let f = |v: f32| v.to_le_bytes();
    let mut c = Vec::new();
    c.extend([4, b'd', b'o', b'o', b'r', 1]);
    c.extend([1, 4, b'h', b'e', b'r', b'o', 1]);
    c.extend([f(0.0), f(0.0), f(0.0), f(0.0)].concat());
    c.push(0);
    c.push(2);
    c.extend([24, 7, 0, 9]);
    c.extend([f(0.25), f(0.5), f(90.0)].concat());
    c.push(0);
    c.extend([25, 0, 9, 2]);
    c.extend([f(0.25), f(0.0), f(0.5)].concat());
    c.push(1);
    c.extend([f(0.25), f(0.5), f(1.0)].concat());
    c.push(1);
    c.extend([f(0.0), f(1.0), f(0.0)].concat());
    c.extend([0, 0]);
    c.extend(f(0.5));
    c.extend([1, 11, 1]);
    c.extend([f(0.25), f(1.0), f(0.0)].concat());
    c
}

#[test]
fn a_game_pack_of_version_3_reads_its_hinge_and_path_steps() {
    let game = game();
    let mut written = written_by_3(&game);
    let mut body = written.split_off(GAME_MAGIC.len() + 4);
    let head = postcard::to_allocvec(&(&game.name, game.batch_version, &game.wasm)).unwrap();
    assert_eq!(body[..head.len()], head[..]);
    assert_eq!(body[head.len()], 0, "no cutscenes");
    let rest = body.split_off(head.len() + 1);
    written.extend(head);
    written.push(1);
    written.extend(hinge_and_path_as_written_by_3());
    written.extend(rest);

    let read = decode_game(&written).unwrap();
    let expected = Cutscene {
        name: "door".into(),
        skippable: true,
        roles: vec![Role {
            name: "hero".into(),
            start: Some(Place {
                position: [0.0; 3],
                yaw: 0.0,
            }),
            actor: 0,
        }],
        body: vec![
            Block::Hinge {
                hinge: 7,
                role: 0,
                clip: 9,
                span: [0.25, 0.5],
                to: 90.0,
                ease: 0,
            },
            Block::Path {
                role: 0,
                clip: 9,
                mode: 2,
                blend: 0.25,
                from: 0.0,
                hold: 0.5,
                spans: vec![PathSpan {
                    from: 0.25,
                    until: 0.5,
                    rate: 1.0,
                    to: Some([0.0, 1.0, 0.0]),
                    yaw: None,
                    ease: 0,
                    arc: 0.5,
                }],
                then: vec![Then {
                    clip: 11,
                    mode: 1,
                    blend: 0.25,
                    speed: 1.0,
                    after: 0.0,
                }],
            },
        ],
    };
    assert_eq!(read.cutscenes, vec![expected]);
    assert_eq!(read.assets, game.assets);
}
