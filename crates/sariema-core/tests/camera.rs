// SPDX-License-Identifier: Apache-2.0

use glam::{Vec3, vec3};
use sariema_core::{Camera, room_camera};
use sariema_format::RoomCamera;

const ROOM: RoomCamera = RoomCamera {
    fov_y_deg: 28.12,
    pitch_deg: 45.0,
    distance: 25.0,
    focus: [3.0, 0.0, -4.0],
};

fn camera() -> Camera {
    room_camera(&ROOM, 16.0 / 9.0)
}

fn ndc(c: &Camera, world: Vec3) -> Vec3 {
    (c.proj * c.view).project_point3(world)
}

#[test]
fn sits_behind_and_above_focus() {
    let c = camera();
    assert!((c.position - vec3(3.0, 17.6777, 13.6777)).length() < 1e-3);
}

#[test]
fn focus_projects_to_screen_centre() {
    let p = ndc(&camera(), Vec3::from(ROOM.focus));
    assert!(p.x.abs() < 1e-5 && p.y.abs() < 1e-5, "ndc {p}");
    assert!(p.z > 0.0 && p.z < 1.0, "ndc {p}");
}

#[test]
fn north_is_up_on_screen() {
    let c = camera();
    let focus = Vec3::from(ROOM.focus);
    assert!(ndc(&c, focus + Vec3::NEG_Z).y > 0.0);
    assert!(ndc(&c, focus + Vec3::X).x > 0.0);
}

#[test]
fn vertical_fov_is_respected() {
    let pitch = ROOM.pitch_deg.to_radians();
    let up = vec3(0.0, pitch.cos(), -pitch.sin());
    let top = Vec3::from(ROOM.focus) + ROOM.distance * 14.06_f32.to_radians().tan() * up;
    let p = ndc(&camera(), top);
    assert!((p.y - 1.0).abs() < 1e-4, "ndc {p}");
}

#[test]
fn depth_spans_zero_at_near_to_one_at_far() {
    let c = camera();
    assert!(c.proj.project_point3(vec3(0.0, 0.0, -0.05)).z.abs() < 1e-6);
    assert!((c.proj.project_point3(vec3(0.0, 0.0, -100.0)).z - 1.0).abs() < 1e-6);
}
