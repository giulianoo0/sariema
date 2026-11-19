// SPDX-License-Identifier: Apache-2.0

use sariema_format::Aabb;
use sariema_format::collision::{
    ActorBody, Collider, CollisionError, CollisionPack, Contact, HeadSphere, Hinge, Shape, Spot,
    Traversal, Trigger, decode, encode,
};

const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

fn pack() -> CollisionPack {
    CollisionPack {
        room: "hall".into(),
        colliders: vec![
            Collider {
                name: "hall.floor".into(),
                shape: Shape::Mesh {
                    vertices: vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, 4.0]],
                    triangles: vec![[0, 2, 1]],
                },
                enabled: true,
            },
            Collider {
                name: "hall.crate".into(),
                shape: Shape::Box {
                    center: [1.0, 0.5, 1.0],
                    half_extents: [0.5; 3],
                    rotation: IDENTITY,
                },
                enabled: false,
            },
            Collider {
                name: "root".into(),
                shape: Shape::Capsule {
                    a: [0.0, 0.0, 0.0],
                    b: [1.0, 1.0, 0.0],
                    radius: 0.2,
                },
                enabled: true,
            },
            Collider {
                name: "stone".into(),
                shape: Shape::Hull {
                    points: vec![
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                    ],
                },
                enabled: true,
            },
        ],
        triggers: vec![Trigger {
            name: "door".into(),
            shape: Shape::Sphere {
                center: [2.0, 1.0, 2.0],
                radius: 0.5,
            },
        }],
        actors: vec![ActorBody {
            name: "hero".into(),
            radius: 0.3,
            height: 1.0,
            head: Some(HeadSphere {
                radius: 0.34,
                height: 1.1,
                forward: 0.0,
            }),
            visual_bounds: Some(Aabb {
                min: [-0.35, 0.0, -0.3],
                max: [0.35, 1.45, 0.3],
            }),
        }],
        spots: vec![
            Spot {
                name: "desk".into(),
                actor: 0,
                position: [1.0, 0.0, 3.0],
                yaw: 1.5,
            },
            Spot {
                name: "door".into(),
                actor: 0,
                position: [3.0, 0.0, 1.0],
                yaw: 0.0,
            },
        ],
        routes: vec![[0, 1]],
        traversals: vec![Traversal {
            name: "window".into(),
            from: 1,
            points: vec![[3.0, 1.0, 0.4], [3.0, 0.0, -0.4]],
            yaw: 3.1,
            contacts: vec![Contact {
                name: "foot".into(),
                point: [3.0, 0.9, 0.0],
            }],
            hold_end: false,
            ease: 0,
        }],
        nav_bounds: Some(Aabb {
            min: [-1.0, -1.0, -1.0],
            max: [5.0, 3.0, 5.0],
        }),
        hinges: vec![Hinge {
            collider: 1,
            pivot: [0.5, 0.0, 0.5],
            axis: [0.0, 1.0, 0.0],
            angle_deg: 30.0,
        }],
    }
}

fn invalid(p: &CollisionPack) -> String {
    match decode(&encode(p)) {
        Err(CollisionError::Invalid(why)) => why,
        other => panic!("expected an invalid pack, got {other:?}"),
    }
}

#[test]
fn a_pack_survives_the_round_trip() {
    let p = pack();
    assert_eq!(decode(&encode(&p)).unwrap(), p);
    assert_eq!(p.triangles(), 1 + 12 + 8);
    assert_eq!(p.spot("door").unwrap().position, [3.0, 0.0, 1.0]);
    assert_eq!(p.traversals[0].end(), [3.0, 0.0, -0.4]);
}

#[test]
fn foreign_old_and_cut_bytes_are_errors_with_their_codes() {
    let bytes = encode(&pack());
    let mut room = bytes.clone();
    room[3] = b'A';
    assert_eq!(decode(&room), Err(CollisionError::BadMagic));
    let mut old = bytes.clone();
    old[4..8].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(decode(&old), Err(CollisionError::Version { found: 0 }));
    let text = decode(&old).unwrap_err().to_string();
    assert!(text.contains("rebuild it with this engine's"), "{text}");
    old[4..8].copy_from_slice(&99u32.to_le_bytes());
    let text = decode(&old).unwrap_err().to_string();
    assert!(text.contains("newer engine"), "{text}");
    for n in 0..bytes.len() {
        let err = decode(&bytes[..n]).unwrap_err();
        assert!(
            matches!(err, CollisionError::Truncated | CollisionError::BadMagic),
            "{n}: {err:?}"
        );
    }
    let mut long = bytes;
    long.push(0);
    assert!(matches!(decode(&long), Err(CollisionError::Invalid(_))));
    assert_eq!(CollisionError::Truncated.code(), "E_PACK_TRUNCATED");
    assert_eq!(CollisionError::Invalid(String::new()).code(), "E_SCHEMA");
}

#[test]
fn a_pack_with_bad_numbers_or_indices_is_rejected() {
    let mut p = pack();
    p.colliders[0].shape = Shape::Mesh {
        vertices: vec![[0.0; 3]],
        triangles: vec![[0, 0, 3]],
    };
    assert!(invalid(&p).contains("hall.floor"));

    let mut p = pack();
    p.colliders[1].shape = Shape::Box {
        center: [0.0; 3],
        half_extents: [1.0; 3],
        rotation: [2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
    };
    assert!(invalid(&p).contains("orthonormal"));

    let mut p = pack();
    p.actors[0].height = 0.5;
    assert!(invalid(&p).contains("hero"));

    let mut p = pack();
    p.spots[1].actor = 1;
    assert!(invalid(&p).contains("spot door"));

    let mut p = pack();
    p.routes.push([0, 2]);
    assert!(invalid(&p).contains("route"));

    let mut p = pack();
    p.spots[1].name = "desk".into();
    assert!(invalid(&p).contains("two spots"));

    let mut p = pack();
    p.triggers[0].shape = Shape::Sphere {
        center: [f32::NAN, 0.0, 0.0],
        radius: 1.0,
    };
    assert!(invalid(&p).contains("trigger door"));

    let mut p = pack();
    p.traversals[0].from = 2;
    assert!(invalid(&p).contains("traversal window"));

    let mut p = pack();
    p.traversals[0].points.clear();
    assert!(invalid(&p).contains("traversal window"));

    let mut p = pack();
    p.traversals[0].contacts[0].point[1] = f32::INFINITY;
    assert!(invalid(&p).contains("traversal window"));

    let mut p = pack();
    let again = p.traversals[0].contacts[0].clone();
    p.traversals[0].contacts.push(again);
    assert!(invalid(&p).contains("two contacts"));

    let mut p = pack();
    let again = p.traversals[0].clone();
    p.traversals.push(again);
    assert!(invalid(&p).contains("two traversals"));

    let mut p = pack();
    p.nav_bounds.as_mut().unwrap().min[0] = 9.0;
    assert!(invalid(&p).contains("nav_bounds"));
}

#[test]
fn a_hinge_turns_a_collider_of_the_pack_about_a_vertical_axis_once() {
    let mut p = pack();
    p.hinges[0].collider = 4;
    assert!(invalid(&p).contains("hinge of collider 4"));

    let mut p = pack();
    p.hinges[0].axis = [1.0, 0.0, 0.0];
    assert!(invalid(&p).contains("vertical"));

    let mut p = pack();
    p.hinges[0].angle_deg = f32::NAN;
    assert!(invalid(&p).contains("hinge of collider 1"));

    let mut p = pack();
    let again = p.hinges[0].clone();
    p.hinges.push(again);
    assert!(invalid(&p).contains("two hinges"));

    let mut p = pack();
    p.hinges[0].axis = [0.0, -1.0, 0.0];
    assert_eq!(decode(&encode(&p)).unwrap(), p);
}
