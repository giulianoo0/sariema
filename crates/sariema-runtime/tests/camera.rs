// SPDX-License-Identifier: Apache-2.0

use glam::Vec3;
use sariema_format::script::Ease;
use sariema_runtime::camera::{Camera, CameraPose, CameraSettings, Framing};
use sariema_runtime::rng::Pcg32;
use sariema_runtime::world::{Entity, Kind, World};

const DISTANCE: f32 = 6.0;

fn world_with(n: u32) -> (World, Vec<Entity>) {
    let mut w = World::new(16);
    let es: Vec<Entity> = (0..n)
        .map(|i| Entity {
            index: i,
            generation: 0,
        })
        .collect();
    for &e in &es {
        w.spawn(e, Kind::Actor, 0).unwrap();
    }
    w.resolve();
    (w, es)
}

fn follow(subject: Entity, yaw: f32) -> Framing {
    Framing::Follow {
        subject,
        distance: DISTANCE,
        pitch: 40.0,
        yaw,
        height: 1.0,
        fov: 45.0,
    }
}

fn put(w: &mut World, e: Entity, at: Vec3) {
    w.begin_step();
    w.node_mut(e).unwrap().local.position = at;
    w.resolve();
}

fn focus(pose: &CameraPose) -> Vec3 {
    pose.position + pose.forward() * DISTANCE
}

fn angle(a: &CameraPose, b: &CameraPose) -> f32 {
    a.rotation.angle_between(b.rotation).to_degrees()
}

#[test]
fn follow_never_overshoots_its_bounds() {
    let (mut w, es) = world_with(1);
    let mut cam = Camera::new(CameraSettings::default());
    cam.set_framing(1, follow(es[0], 0.0));
    cam.set_tuning(
        1,
        CameraSettings {
            dead_zone: 0.0,
            frequency: 2.0,
            damping: 0.2,
            look_ahead: 0.5,
            ..CameraSettings::default()
        },
    )
    .unwrap();
    cam.set_bounds(1, Vec3::new(-3.0, -10.0, -3.0), Vec3::new(3.0, 10.0, 3.0))
        .unwrap();
    cam.use_rig(1, 0.0, Ease::Linear).unwrap();
    let mut max_x: f32 = 0.0;
    for step in 0..400 {
        let x = if step < 200 {
            step as f32 * 0.1
        } else {
            20.0 - (step - 200) as f32 * 0.1
        };
        put(&mut w, es[0], Vec3::new(x, 0.0, 0.0));
        cam.begin_step();
        cam.step(&w);
        let f = focus(&cam.pose().unwrap());
        max_x = max_x.max(f.x);
        assert!(
            f.x <= 3.0 + 1e-4 && f.x >= -3.0 - 1e-4,
            "step {step}: focus {f}"
        );
    }
    assert!(max_x > 2.9, "never reached the bound: {max_x}");
}

#[test]
fn the_subject_moves_freely_inside_the_dead_zone() {
    let (mut w, es) = world_with(1);
    let mut cam = Camera::new(CameraSettings::default());
    cam.set_framing(1, follow(es[0], 0.0));
    cam.use_rig(1, 0.0, Ease::Linear).unwrap();
    cam.begin_step();
    cam.step(&w);
    let start = cam.pose().unwrap();
    put(&mut w, es[0], Vec3::new(0.2, 0.0, 0.0));
    for _ in 0..120 {
        cam.begin_step();
        cam.step(&w);
    }
    assert!(cam.pose().unwrap().position.distance(start.position) < 1e-5);
    put(&mut w, es[0], Vec3::new(2.0, 0.0, 0.0));
    for _ in 0..240 {
        cam.begin_step();
        cam.step(&w);
    }
    assert!(
        (focus(&cam.pose().unwrap()).x - 1.75).abs() < 0.01,
        "settles a dead zone behind"
    );
}

#[test]
fn a_blend_between_rigs_is_continuous_and_a_cut_is_the_only_jump() {
    let (mut w, es) = world_with(2);
    put(&mut w, es[1], Vec3::new(8.0, 0.0, -5.0));
    let mut cam = Camera::new(CameraSettings::default());
    cam.set_framing(1, follow(es[0], 0.0));
    cam.set_framing(2, follow(es[1], 120.0));
    cam.set_framing(
        3,
        Framing::Fixed(CameraPose::looking(
            Vec3::new(0.0, 20.0, 20.0),
            Vec3::ZERO,
            30.0,
        )),
    );
    cam.use_rig(1, 0.0, Ease::Linear).unwrap();
    let mut last: Option<CameraPose> = None;
    let mut cuts = Vec::new();
    for step in 0..400u32 {
        cam.begin_step();
        match step {
            50 => cam.use_rig(2, 1.0, Ease::InOutCubic).unwrap(),
            80 => cam.use_rig(1, 0.8, Ease::OutQuad).unwrap(),
            200 => cam.use_rig(3, 0.0, Ease::Linear).unwrap(),
            260 => cam.use_rig(2, 1.5, Ease::Smoothstep).unwrap(),
            _ => {}
        }
        let t = step as f32 / 60.0;
        put(&mut w, es[0], Vec3::new(t.sin() * 2.0, 0.0, t * 0.5));
        cam.step(&w);
        let pose = cam.pose().unwrap();
        if cam.cut() {
            cuts.push(step);
            if let Some(before) = last {
                assert!(
                    pose.position.distance(before.position) > 5.0,
                    "a cut that did not move"
                );
            }
        } else if let Some(before) = last {
            // The rigs' poses are up to 18 m apart and in-out cubic peaks at
            // slope 3, so a 1 s blend moves at most 0.9 m in a step.
            let moved = pose.position.distance(before.position);
            assert!(moved < 1.0, "step {step}: jumped {moved} m");
            assert!(
                angle(&pose, &before) < 10.0,
                "step {step}: turned {}°",
                angle(&pose, &before)
            );
            // Curves with zero slope at 0 start a blend from rest.
            if [50, 260].contains(&step) {
                assert!(
                    moved < 0.1,
                    "step {step}: a blend started with a jump of {moved} m"
                );
            }
        }
        last = Some(pose);
    }
    assert_eq!(cuts, [0, 200]);
}

#[test]
fn framing_two_subjects_backs_off_as_they_part() {
    let (mut w, es) = world_with(2);
    let mut cam = Camera::new(CameraSettings::default());
    let frame = Framing::Frame {
        a: es[0],
        b: es[1],
        pitch: 30.0,
        yaw: 0.0,
        height: 1.0,
        fov: 40.0,
        margin: 1.0,
    };
    cam.set_framing(1, frame);
    cam.use_rig(1, 0.0, Ease::Linear).unwrap();
    put(&mut w, es[1], Vec3::new(2.0, 0.0, 0.0));
    let mut distances = Vec::new();
    for sep in [2.0f32, 8.0] {
        put(&mut w, es[1], Vec3::new(sep, 0.0, 0.0));
        for _ in 0..300 {
            cam.begin_step();
            cam.step(&w);
        }
        let pose = cam.pose().unwrap();
        let mid = Vec3::new(sep / 2.0, 1.0, 0.0);
        distances.push(pose.position.distance(mid));
        let half_fov = (pose
            .forward()
            .angle_between(Vec3::new(sep, 0.0, 0.0) - pose.position))
        .to_degrees();
        assert!(half_fov < 20.0, "subject b outside the view at {sep} m");
    }
    assert!(distances[1] > distances[0] * 2.0, "{distances:?}");
}

#[test]
fn shake_is_seeded_and_dies_out() {
    let run = |seed: u64| {
        let (w, es) = world_with(1);
        let mut cam = Camera::new(CameraSettings::default());
        cam.set_framing(1, follow(es[0], 0.0));
        cam.use_rig(1, 0.0, Ease::Linear).unwrap();
        cam.shake(0.3, 8.0, 0.5, &mut Pcg32::new(seed, 9));
        (0..60)
            .map(|_| {
                cam.begin_step();
                cam.step(&w);
                cam.pose().unwrap().position
            })
            .collect::<Vec<_>>()
    };
    let (a, b, c) = (run(1), run(1), run(2));
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a[59], a[40], "still after the shake");
}

#[test]
fn reframing_the_rig_in_use_blends_from_the_shown_pose() {
    let (w, es) = world_with(1);
    let mut cam = Camera::new(CameraSettings::default());
    cam.set_framing(1, follow(es[0], 0.0));
    cam.use_rig(1, 0.0, Ease::Linear).unwrap();
    let fixed = Framing::Fixed(CameraPose::looking(
        Vec3::new(0.0, 20.0, 20.0),
        Vec3::ZERO,
        30.0,
    ));
    let mut last: Option<CameraPose> = None;
    for step in 0..300u32 {
        cam.begin_step();
        match step {
            60 => cam.set_framing(1, follow(es[0], 90.0)),
            90 => cam.set_framing(1, fixed),
            200 => cam.set_framing(1, fixed),
            _ => {}
        }
        cam.step(&w);
        let pose = cam.pose().unwrap();
        if let Some(before) = last {
            assert!(!cam.cut(), "step {step}: reframing cut");
            assert!(
                angle(&pose, &before) < 10.0,
                "step {step}: turned {}° at once",
                angle(&pose, &before)
            );
            // The framings are up to 26 m apart; a 0.5 s in-out cubic blend
            // moves at most 2.6 m in a step.
            let moved = pose.position.distance(before.position);
            assert!(moved < 3.0, "step {step}: jumped {moved} m");
        }
        last = Some(pose);
    }
    let end = cam.pose().unwrap();
    let Framing::Fixed(target) = fixed else {
        unreachable!()
    };
    assert!(end.position.distance(target.position) < 1e-3, "it arrives");
    assert!(!cam.blending(), "the same framing again starts no blend");
}
