// SPDX-License-Identifier: MIT OR Apache-2.0

use glam::camera::rh::proj::directx;
use glam::{Mat4, Vec3};
use sariema_format::RoomCamera;

pub const NEAR: f32 = 0.05;
pub const FAR: f32 = 100.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// World to view space; the camera looks down −Z.
    pub view: Mat4,
    /// View to clip space, WebGPU convention: NDC depth in [0, 1], Y up.
    pub proj: Mat4,
    pub position: Vec3,
    pub near: f32,
    pub far: f32,
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
        near: NEAR,
        far: FAR,
    }
}
