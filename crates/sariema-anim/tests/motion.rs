// SPDX-License-Identifier: Apache-2.0

use sariema_anim::{Rig, ground_speed};
use sariema_format::Aabb;
use sariema_format::actor::demo::demo_arm;
use sariema_format::actor::{Channel, Clip, Interpolation, Joint, Property, Transform, Values};

fn foot(name: &str) -> Joint {
    Joint {
        name: name.into(),
        parent: Some(0),
        rest: Transform::IDENTITY,
        inverse_bind: glam::Mat4::IDENTITY.to_cols_array(),
    }
}

/// Two feet under joint 0 over a 1 s cycle: each is planted (y = 0) for half
/// of it, sliding back `stride` metres along −Z, and lifted 0.1 m while it
/// swings forward.
fn walker(stride: f32) -> Rig {
    let mut pack = demo_arm();
    pack.joints.truncate(1);
    pack.joints.extend([foot("foot_L"), foot("foot_R")]);
    pack.sockets.clear();
    pack.meshes
        .iter_mut()
        .flat_map(|m| &mut m.vertices)
        .for_each(|v| v.joints = [0; 4]);
    let h = stride / 2.0;
    let keys = |z: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|k| [0.0, y[k], z[k]]).to_vec();
    let channel = |joint, values| Channel {
        joint,
        property: Property::Translation,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 0.5, 1.0],
        values: Values::Vec3(values),
    };
    pack.clips = vec![Clip {
        name: "walk".into(),
        duration: 1.0,
        looping: true,
        channels: vec![
            channel(1, keys([h, -h, h], [0.0, 0.0, 0.1])),
            channel(2, keys([-h, h, -h], [0.1, 0.1, 0.0])),
        ],
        markers: Vec::new(),
        faces: Vec::new(),
        bounds: Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        },
        ground_speed: None,
    }];
    Rig::new(&pack).unwrap()
}

#[test]
fn ground_speed_is_the_planted_foot_speed() {
    let rig = walker(0.75);
    let feet = [rig.joint("foot_L").unwrap(), rig.joint("foot_R").unwrap()];
    let walk = rig.clip("walk").unwrap();
    let speed = ground_speed(&rig, walk, &feet, 120.0).unwrap();
    assert!((speed - 1.5).abs() < 0.05, "{speed} m/s");
}

#[test]
fn ground_speed_needs_feet_and_time() {
    let rig = walker(0.75);
    let walk = rig.clip("walk").unwrap();
    assert_eq!(ground_speed(&rig, walk, &[], 120.0), None);
    let feet = [rig.joint("foot_L").unwrap()];
    assert_eq!(ground_speed(&rig, walk, &feet, 0.0), None);
}
