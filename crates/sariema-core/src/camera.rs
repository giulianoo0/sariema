// SPDX-License-Identifier: Apache-2.0

use glam::camera::rh::proj::directx;
use glam::{Mat4, Quat, Vec3};
use sariema_format::RoomCamera;

const NEAR: f32 = 0.05;
const FAR: f32 = 100.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// World to view space; the camera looks down −Z.
    pub view: Mat4,
    /// View to clip space, WebGPU convention: NDC depth in [0, 1], Y up.
    pub proj: Mat4,
    pub position: Vec3,
    /// The point the camera looks at.
    pub focus: Vec3,
}

/// The fixed room camera: facing north (−Z), pitched down `pitch_deg`,
/// `distance` metres from `focus`.
pub fn room_camera(cam: &RoomCamera, aspect: f32) -> Camera {
    let pitch = cam.pitch_deg.to_radians();
    let position = Vec3::from(cam.focus) + cam.distance * Vec3::new(0.0, pitch.sin(), pitch.cos());
    Camera {
        // Yaw is 0, so the view is a pure tilt about X; unlike look_at it holds at 90°.
        view: Mat4::from_rotation_x(pitch) * Mat4::from_translation(-position),
        proj: directx::perspective(cam.fov_y_deg.to_radians(), aspect, NEAR, FAR),
        position,
        focus: cam.focus.into(),
    }
}

/// A camera at `position` turned by `rotation` from looking down −Z, with a
/// vertical field of view of `fov_y_deg`; its focus is `focus_distance`
/// metres ahead.
pub fn pose_camera(
    position: Vec3,
    rotation: Quat,
    fov_y_deg: f32,
    aspect: f32,
    focus_distance: f32,
) -> Camera {
    Camera {
        view: Mat4::from_rotation_translation(rotation, position).inverse(),
        proj: directx::perspective(fov_y_deg.to_radians(), aspect, NEAR, FAR),
        position,
        focus: position + rotation * Vec3::NEG_Z * focus_distance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posed_camera_sees_what_the_room_camera_sees_from_its_place() {
        let room = RoomCamera {
            fov_y_deg: 40.0,
            pitch_deg: 50.0,
            distance: 7.0,
            focus: [1.5, 0.5, -2.0],
        };
        let fixed = room_camera(&room, 16.0 / 9.0);
        let tilt = Quat::from_rotation_x(-room.pitch_deg.to_radians());
        let posed = pose_camera(
            fixed.position,
            tilt,
            room.fov_y_deg,
            16.0 / 9.0,
            room.distance,
        );
        assert!(posed.view.abs_diff_eq(fixed.view, 1e-5), "{posed:?}");
        assert_eq!(posed.proj, fixed.proj);
        assert!(posed.focus.abs_diff_eq(fixed.focus, 1e-5), "{posed:?}");
    }
}
