// SPDX-License-Identifier: Apache-2.0

use glam::Vec3;
use sariema_runtime::world::{Entity, Kind, Reject, World};

fn e(index: u32, generation: u32) -> Entity {
    Entity { index, generation }
}

#[test]
fn stale_handles_are_rejected_after_despawn_and_reuse() {
    let mut w = World::new(16);
    let first = e(3, 0);
    w.spawn(first, Kind::Actor, 7).unwrap();
    assert!(w.is_alive(first));
    w.despawn(first).unwrap();
    assert_eq!(w.node_mut(first).err(), Some(Reject::Stale));
    assert_eq!(w.despawn(first), Err(Reject::Stale));

    assert_eq!(
        w.spawn(first, Kind::Actor, 7),
        Err(Reject::InUse),
        "a generation is never reused"
    );
    let second = e(3, 1);
    w.spawn(second, Kind::Empty, 0).unwrap();
    assert!(!w.is_alive(first));
    assert_eq!(w.node_mut(first).err(), Some(Reject::Stale));
    assert_eq!(w.node(second).unwrap().kind, Kind::Empty);
    assert_eq!(
        w.spawn(e(3, 9), Kind::Empty, 0),
        Err(Reject::InUse),
        "slot is live"
    );
    assert_eq!(w.len(), 1);
}

#[test]
fn slots_past_the_limit_are_refused() {
    let mut w = World::new(4);
    assert_eq!(w.spawn(e(4, 0), Kind::Empty, 0), Err(Reject::Limit));
    assert_eq!(w.spawn(Entity::NONE, Kind::Empty, 0), Err(Reject::Limit));
    w.spawn(e(3, 0), Kind::Empty, 0).unwrap();
}

#[test]
fn children_follow_their_parent_and_keep_their_place_when_it_goes() {
    let mut w = World::new(8);
    let (parent, child) = (e(0, 0), e(1, 0));
    w.spawn(parent, Kind::Empty, 0).unwrap();
    w.spawn(child, Kind::Empty, 0).unwrap();
    w.node_mut(parent).unwrap().local.position = Vec3::new(1.0, 0.0, 0.0);
    w.node_mut(parent).unwrap().local.yaw = 90.0;
    w.node_mut(child).unwrap().local.position = Vec3::new(0.0, 0.0, -2.0);
    w.set_parent(child, parent).unwrap();
    w.resolve();
    let at = w.pose(child).unwrap().position;
    assert!(at.distance(Vec3::new(-1.0, 0.0, 0.0)) < 1e-5, "{at}");

    assert_eq!(w.set_parent(parent, child), Err(Reject::Cycle));
    assert_eq!(w.set_parent(parent, parent), Err(Reject::Cycle));

    w.despawn(parent).unwrap();
    w.resolve();
    let node = w.node(child).unwrap();
    assert!(node.parent.is_none());
    assert!(w.pose(child).unwrap().position.distance(at) < 1e-5);
    assert!((node.local.yaw - 90.0).abs() < 1e-3);
}

#[test]
fn rendering_interpolates_between_steps_except_after_a_snap() {
    let mut w = World::new(8);
    let a = e(0, 0);
    w.spawn(a, Kind::Empty, 0).unwrap();
    w.resolve();
    w.begin_step();
    w.node_mut(a).unwrap().local.position.x = 2.0;
    w.resolve();
    assert_eq!(w.interpolated(a, 0.25).unwrap().position.x, 0.5);

    w.begin_step();
    w.node_mut(a).unwrap().local.position.x = 10.0;
    w.snap(a);
    w.resolve();
    assert_eq!(w.interpolated(a, 0.25).unwrap().position.x, 10.0);
}

#[test]
fn entities_iterate_in_slot_order() {
    let mut w = World::new(8);
    for i in [5, 1, 3] {
        w.spawn(e(i, 0), Kind::Empty, 0).unwrap();
    }
    let order: Vec<u32> = w.entities().map(|(e, _)| e.index).collect();
    assert_eq!(order, [1, 3, 5]);
}
