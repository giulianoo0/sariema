// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeSet, HashMap};

use sariema_format::script::cutscene::{Block, Cutscene, Place, Role};
use sariema_format::script::{
    BATCH_VERSION, COMMANDS, Command, ENUMS, EVENTS, Entity, Event, GAME_VERSION, GameAssets,
    GameFormatError, GamePack, GameRoom, Hud, Message, MessageSpec, Sink, WireError, decode_game,
    encode_game, name_id, put_scene_value, records, scene_values,
};

fn batch<M>(messages: impl IntoIterator<Item = M>, write: impl Fn(&M, &mut Vec<u8>)) -> Vec<u8> {
    let mut out = Vec::new();
    for m in messages {
        write(&m, &mut out);
    }
    out
}

fn decode_all<'a, M: Message<'a>>(
    bytes: &'a [u8],
) -> Result<Vec<M>, sariema_format::script::BatchError> {
    records::<M>(bytes).collect()
}

#[test]
fn every_command_round_trips() {
    let samples: Vec<Command> = Command::samples().collect();
    assert_eq!(samples.len(), COMMANDS.len());
    for sample in &samples {
        let one = batch([*sample], |m, out| m.write(out));
        assert_eq!(one.len(), sample.size(), "{sample:?}");
        assert_eq!(decode_all::<Command>(&one).unwrap(), vec![*sample]);
    }
    let all = batch(samples.iter().copied(), |m, out| m.write(out));
    assert_eq!(decode_all::<Command>(&all).unwrap(), samples);
}

#[test]
fn every_event_round_trips() {
    let samples: Vec<Event> = Event::samples().collect();
    assert_eq!(samples.len(), EVENTS.len());
    let all = batch(samples.iter().copied(), |m, out| m.write(out));
    assert_eq!(decode_all::<Event>(&all).unwrap(), samples);
    for sample in &samples {
        let mut one = Vec::new();
        sample.write(&mut one);
        assert_eq!(one.len(), sample.size());
    }
}

#[test]
fn specs_match_the_decoder() {
    for (specs, opcodes) in [
        (
            COMMANDS,
            Command::samples()
                .map(|c| (c.opcode(), c.name()))
                .collect::<Vec<_>>(),
        ),
        (
            EVENTS,
            Event::samples().map(|e| (e.opcode(), e.name())).collect(),
        ),
    ] {
        let listed: Vec<(u16, &str)> = specs.iter().map(|s| (s.opcode, s.name)).collect();
        assert_eq!(listed, opcodes);
        let unique: BTreeSet<u16> = listed.iter().map(|(op, _)| *op).collect();
        assert_eq!(unique.len(), listed.len(), "opcodes repeat");
        let names: BTreeSet<&str> = listed.iter().map(|(_, n)| *n).collect();
        assert_eq!(names.len(), listed.len(), "names repeat");
        for spec in specs {
            assert!(!spec.doc.is_empty() || spec.fields.is_empty() || !spec.name.is_empty());
            for field in spec.fields {
                let known = [
                    "u8", "u16", "u32", "u64", "i32", "f32", "bool", "Entity", "Vec3", "Str",
                    "Bytes", "Points",
                ];
                assert!(
                    known.contains(&field.ty) || ENUMS.iter().any(|e| e.name == field.ty),
                    "{}.{} has type {}",
                    spec.name,
                    field.name,
                    field.ty
                );
            }
        }
    }
}

#[test]
fn every_prefix_of_a_batch_is_a_stable_error_or_a_shorter_batch() {
    let samples: Vec<Command> = Command::samples().collect();
    let all = batch(samples.iter().copied(), |m, out| m.write(out));
    let mut ends = vec![0];
    for s in &samples {
        ends.push(ends.last().unwrap() + s.size());
    }
    for cut in 0..all.len() {
        let result = decode_all::<Command>(&all[..cut]);
        if ends.contains(&cut) {
            assert_eq!(
                result.unwrap().len(),
                ends.iter().position(|&e| e == cut).unwrap()
            );
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.error, WireError::Truncated, "cut at {cut}");
            assert_eq!(error.code(), "E_BATCH_TRUNCATED");
            assert_eq!(
                error.offset,
                *ends.iter().filter(|&&e| e < cut).max().unwrap()
            );
        }
    }
}

#[test]
fn corrupt_records_are_stable_errors() {
    let entity = Entity {
        index: 1,
        generation: 1,
    };
    let mut good = Vec::new();
    Command::SetVisible {
        entity,
        visible: true,
    }
    .write(&mut good);

    let mut unknown = good.clone();
    0xFFFFu16
        .to_le_bytes()
        .iter()
        .for_each(|b| unknown.push(*b));
    let error = decode_all::<Command>(&unknown).unwrap_err();
    assert_eq!(
        (error.code(), error.record, error.offset),
        ("E_BATCH_UNKNOWN", 1, good.len())
    );

    let mut bad_bool = good.clone();
    *bad_bool.last_mut().unwrap() = 2;
    assert_eq!(
        decode_all::<Command>(&bad_bool).unwrap_err().code(),
        "E_BATCH_INVALID"
    );

    let mut nan = Vec::new();
    Command::SetScale { entity, scale: 1.0 }.write(&mut nan);
    let n = nan.len();
    nan[n - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        decode_all::<Command>(&nan).unwrap_err().error,
        WireError::Invalid("f32")
    );

    let mut text = Vec::new();
    Command::UiText {
        node: 3,
        text: "ab",
    }
    .write(&mut text);
    let n = text.len();
    text[n - 1] = 0xFF;
    assert_eq!(
        decode_all::<Command>(&text).unwrap_err().error,
        WireError::Invalid("Str")
    );

    let mut huge = Vec::new();
    Command::UiText { node: 3, text: "" }.write(&mut huge);
    let n = huge.len();
    huge[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode_all::<Command>(&huge).unwrap_err().error,
        WireError::Truncated
    );

    let mut bad_enum = Vec::new();
    Command::Control {
        by: sariema_format::script::Controller::Player,
    }
    .write(&mut bad_enum);
    *bad_enum.last_mut().unwrap() = 9;
    assert_eq!(
        decode_all::<Command>(&bad_enum).unwrap_err().error,
        WireError::Invalid("Controller")
    );
}

#[test]
fn records_stop_after_the_first_error() {
    let mut bytes = vec![0xEE, 0xEE];
    Command::AdvanceLine {}.write(&mut bytes);
    let all: Vec<_> = records::<Command>(&bytes).collect();
    assert_eq!(all.len(), 1);
    assert!(all[0].is_err());
}

struct Count(usize);

impl Sink for Count {
    fn put(&mut self, bytes: &[u8]) {
        self.0 += bytes.len();
    }
}

#[test]
fn fields_visits_every_field_of_the_spec_in_order() {
    let listed = |spec: &MessageSpec| -> Vec<&str> { spec.fields.iter().map(|f| f.name).collect() };
    for (c, spec) in Command::samples().zip(COMMANDS) {
        let mut visited = Vec::new();
        c.fields(|name, _| visited.push(name));
        assert_eq!(visited, listed(spec), "{}", spec.name);
    }
    for (e, spec) in Event::samples().zip(EVENTS) {
        let mut visited = Vec::new();
        e.fields(|name, _| visited.push(name));
        assert_eq!(visited, listed(spec), "{}", spec.name);
    }
}

#[test]
fn size_matches_what_write_writes_for_every_sample() {
    for c in Command::samples() {
        let mut n = Count(0);
        c.write(&mut n);
        assert_eq!(n.0, c.size());
    }
}

fn game() -> GamePack {
    GamePack {
        name: "hello".into(),
        batch_version: BATCH_VERSION,
        wasm: b"\0asm\x01\0\0\0".to_vec(),
        cutscenes: vec![Cutscene {
            name: "meet".into(),
            skippable: true,
            roles: vec![Role {
                name: "hero".into(),
                start: Some(Place {
                    position: [0.0, 0.0, -2.0],
                    yaw: 0.0,
                }),
                actor: name_id("hero"),
            }],
            body: vec![Block::Wait(0.5), Block::Show { role: 0 }],
        }],
        names: vec!["meet".into(), "hero".into()],
        settings: "{}".into(),
        assets: GameAssets {
            rooms: vec![
                GameRoom {
                    name: "porch".into(),
                    room: Some(b"SRMA room".to_vec()),
                    neighbours: vec![1],
                    ambience: Some("crickets".into()),
                    ..GameRoom::default()
                },
                GameRoom {
                    name: "hall".into(),
                    room: Some(b"SRMA hall".to_vec()),
                    neighbours: vec![0],
                    ..GameRoom::default()
                },
            ],
            actors: vec![("hero".into(), b"SRMK hero".to_vec())],
            sounds: None,
            hud: Some(Hud {
                description: "{}".into(),
                files: vec![("font.ttf".into(), vec![0, 1, 2, 255])],
            }),
        },
    }
}

#[test]
fn game_pack_round_trips() {
    let g = game();
    let bytes = encode_game(&g);
    assert_eq!(&bytes[..4], b"SRMG");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        GAME_VERSION
    );
    assert_eq!(decode_game(&bytes).unwrap(), g);
}

#[test]
fn a_game_packs_rooms_must_be_distinct_with_neighbours_in_range() {
    let invalid = |g: &GamePack| match decode_game(&encode_game(g)) {
        Err(GameFormatError::Invalid(why)) => why,
        other => panic!("expected an invalid pack, got {other:?}"),
    };
    let mut g = game();
    g.assets.rooms[1].name = "porch".into();
    assert!(invalid(&g).contains("rooms \"porch\" and \"porch\""));
    let mut g = game();
    g.assets.rooms[0].neighbours = vec![2];
    assert!(invalid(&g).contains("room porch has neighbour 2 of 2 rooms"));
    let mut g = game();
    g.assets.rooms[1].neighbours = vec![1];
    assert!(invalid(&g).contains("room hall has neighbour 1"));
    let mut g = game();
    g.assets.rooms[1].collision = Some(b"SRMC".to_vec());
    assert!(invalid(&g).contains("room hall has a collision pack and room porch has none"));
}

#[test]
fn game_pack_header_errors() {
    let bytes = encode_game(&game());
    assert_eq!(decode_game(b"SRMA").unwrap_err().code(), "E_PACK_BAD_MAGIC");
    assert_eq!(decode_game(b"SRM").unwrap_err(), GameFormatError::Truncated);
    let mut version = bytes.clone();
    version[4] = 99;
    assert_eq!(decode_game(&version).unwrap_err().code(), "E_PACK_VERSION");
    for cut in 8..bytes.len() {
        assert_eq!(
            decode_game(&bytes[..cut]).unwrap_err().code(),
            "E_PACK_TRUNCATED",
            "{cut}"
        );
    }
    let mut extra = bytes;
    extra.push(0);
    assert_eq!(decode_game(&extra).unwrap_err().code(), "E_SCHEMA");
}

#[test]
fn batch_version_mismatch_is_refused_at_load() {
    let mut g = game();
    g.batch_version = BATCH_VERSION + 1;
    let error = decode_game(&encode_game(&g)).unwrap_err();
    assert_eq!(error.code(), "E_BATCH_VERSION");
    assert!(error.to_string().contains(&(BATCH_VERSION + 1).to_string()));
}

#[test]
fn cutscene_with_a_role_out_of_range_is_invalid() {
    let mut g = game();
    g.cutscenes[0].body.push(Block::Hide { role: 4 });
    let error = decode_game(&encode_game(&g)).unwrap_err();
    assert_eq!(error.code(), "E_SCHEMA");
    assert!(error.to_string().contains("cutscene meet"), "{error}");

    let mut g = game();
    g.cutscenes[0].body.push(Block::Fade {
        to: 2.0,
        duration: 1.0,
        color: [0.0; 3],
    });
    assert_eq!(
        decode_game(&encode_game(&g)).unwrap_err().code(),
        "E_SCHEMA"
    );
}

#[test]
fn names_with_the_same_id_are_refused() {
    let mut seen: HashMap<u32, String> = HashMap::new();
    let (a, b) = (0u32..)
        .find_map(|i| {
            let name = format!("n{i}");
            seen.insert(name_id(&name), name.clone())
                .map(|other| (other, name))
        })
        .unwrap();
    let mut g = game();
    g.names = vec![a, b];
    let error = decode_game(&encode_game(&g)).unwrap_err();
    assert_eq!(error.code(), "E_SCHEMA");
    assert!(error.to_string().contains("same id"), "{error}");
}

#[test]
fn scene_values_read_back_what_was_put_and_refuse_a_torn_list() {
    let mut bytes = Vec::new();
    put_scene_value(&mut bytes, name_id("look.exposure"), 2.5);
    put_scene_value(&mut bytes, name_id("fog.density"), 0.04);
    assert_eq!(bytes.len(), 16);
    let read: Vec<(u32, f32)> = scene_values(&bytes).unwrap().collect();
    assert_eq!(
        read,
        [
            (name_id("look.exposure"), 2.5),
            (name_id("fog.density"), 0.04)
        ]
    );
    assert_eq!(scene_values(&[]).unwrap().count(), 0);
    assert!(scene_values(&bytes[..15]).is_none());
    let command = Command::SceneTween {
        tween: 3,
        values: &bytes,
        duration: 1.5,
        ease: sariema_format::script::Ease::InOutSine,
    };
    let encoded = batch([command], |c, out| c.write(out));
    let decoded = decode_all::<Command>(&encoded).unwrap();
    assert_eq!(decoded, [command]);
}

#[test]
fn a_scene_block_needs_finite_values_a_duration_and_a_known_ease() {
    let scene = |values: Vec<(u32, f32)>, duration: f32, ease: u8| {
        let mut g = game();
        g.cutscenes[0].body.push(Block::Scene {
            values,
            duration,
            ease,
        });
        decode_game(&encode_game(&g))
    };
    let exposure = name_id("look.exposure");
    let fine = scene(vec![(exposure, 0.5)], 2.0, 0).unwrap();
    assert_eq!(
        fine.cutscenes[0].body.last(),
        Some(&Block::Scene {
            values: vec![(exposure, 0.5)],
            duration: 2.0,
            ease: 0
        })
    );
    for (values, duration, ease) in [
        (vec![(exposure, f32::NAN)], 1.0, 0),
        (vec![(exposure, 1.0)], -1.0, 0),
        (vec![(exposure, 1.0)], 1.0, 200),
    ] {
        let error = scene(values, duration, ease).unwrap_err();
        assert_eq!(error.code(), "E_SCHEMA", "{error}");
        assert!(error.to_string().contains("cutscene meet"), "{error}");
    }
}
