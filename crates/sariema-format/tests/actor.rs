// SPDX-License-Identifier: Apache-2.0

use sariema_format::actor::demo::{demo_arm, demo_face};
use sariema_format::actor::{ActorPack, ActorPackError, Values, decode, encode};

fn invalid(actor: &ActorPack) -> String {
    match decode(&encode(actor)) {
        Err(ActorPackError::Invalid(why)) => why,
        other => panic!("expected an invalid pack, got {other:?}"),
    }
}

#[test]
fn demo_actors_round_trip() {
    for actor in [demo_arm(), demo_face()] {
        assert_eq!(decode(&encode(&actor)).unwrap(), actor);
    }
}

#[test]
fn header_errors_have_room_pack_codes() {
    let mut b = encode(&demo_arm());
    b[3] = b'A';
    let bad = decode(&b).unwrap_err();
    assert_eq!(
        (bad.clone(), bad.code()),
        (ActorPackError::BadMagic, "E_PACK_BAD_MAGIC")
    );
    let mut b = encode(&demo_arm());
    b[4..8].copy_from_slice(&7u32.to_le_bytes());
    assert_eq!(
        decode(&b).unwrap_err(),
        ActorPackError::Version { found: 7 }
    );
    let room = sariema_format::encode(&sariema_format::demo::demo_room());
    assert_eq!(decode(&room).unwrap_err(), ActorPackError::BadMagic);
}

#[test]
fn truncated_never_panics() {
    let b = encode(&demo_face());
    for n in 0..b.len() {
        assert!(decode(&b[..n]).is_err(), "{n} bytes");
    }
}

#[test]
fn a_parent_after_its_child_is_invalid() {
    let mut a = demo_arm();
    a.joints[1].parent = Some(2);
    assert!(invalid(&a).contains("joint 1"));
}

#[test]
fn weights_that_do_not_sum_to_one_are_invalid() {
    let mut a = demo_arm();
    a.meshes[0].vertices[3].weights = [100, 0, 0, 0];
    assert!(invalid(&a).contains("weights"));
}

#[test]
fn a_vertex_bound_to_a_missing_joint_is_invalid() {
    let mut a = demo_arm();
    a.meshes[1].vertices[0].joints[0] = 3;
    assert!(invalid(&a).contains("joint 3"));
}

#[test]
fn an_unused_joint_slot_may_hold_anything() {
    let mut a = demo_arm();
    a.meshes[1].vertices[0].joints[3] = 900;
    assert_eq!(decode(&encode(&a)).unwrap(), a);
}

#[test]
fn keys_out_of_order_or_past_the_end_are_invalid() {
    let mut a = demo_arm();
    a.clips[0].channels[0].times.swap(0, 1);
    assert!(invalid(&a).contains("bend"));
    let mut a = demo_arm();
    a.clips[0].channels[0].times[2] = 1.5;
    assert!(invalid(&a).contains("bend"));
}

#[test]
fn rotation_keys_must_be_quaternions() {
    let mut a = demo_arm();
    a.clips[0].channels[0].values = Values::Vec3(vec![[0.0; 3]; 3]);
    assert!(invalid(&a).contains("Rotation"));
}

#[test]
fn two_channels_for_one_property_are_invalid() {
    let mut a = demo_arm();
    let twin = a.clips[0].channels[0].clone();
    a.clips[0].channels.push(twin);
    assert!(invalid(&a).contains("two"));
}

#[test]
fn face_cells_past_the_grid_are_invalid() {
    let mut a = demo_face();
    a.faces[0].frames[1].cell = 4;
    assert!(invalid(&a).contains("eyes"));
    let mut a = demo_face();
    a.clips[0].faces[0].cells[1] = 9;
    assert!(invalid(&a).contains("face track"));
}

#[test]
fn an_actor_without_joints_is_invalid() {
    let mut a = demo_face();
    a.joints.clear();
    a.clips.clear();
    a.meshes.clear();
    a.parts.clear();
    assert!(invalid(&a).contains("no joints"));
}

#[test]
fn quantized_rotations_round_trip() {
    let mut a = demo_arm();
    a.clips[0].channels[0].values = Values::QuatSnorm16(vec![[0, 0, 0, i16::MAX]; 3]);
    assert_eq!(decode(&encode(&a)).unwrap(), a);
}
