// SPDX-License-Identifier: Apache-2.0

//! Frustum tests against the planes of a projection, after Gribb and
//! Hartmann's "Fast Extraction of Viewing Frustum Planes".

use glam::{Mat4, Vec3, Vec4};

/// The six planes (normal xyz, offset w) of `m`, a projection or a
/// view-projection, in the space `m` maps from, with the inside where
/// `p · (x, 1) >= 0`. NDC depth runs 0..1, so the near plane is the third row
/// alone.
pub(crate) fn planes(m: Mat4) -> [Vec4; 6] {
    let row = |i| m.row(i);
    [
        row(3) + row(0),
        row(3) - row(0),
        row(3) + row(1),
        row(3) - row(1),
        row(2),
        row(3) - row(2),
    ]
    .map(|p| p / p.truncate().length())
}

pub(crate) fn sphere_visible(planes: &[Vec4; 6], center: Vec3, radius: f32) -> bool {
    planes.iter().all(|p| p.dot(center.extend(1.0)) >= -radius)
}

/// Whether the box `min..max` may be inside: false only when it lies wholly
/// outside one plane. The corner furthest along each plane's normal decides.
pub(crate) fn box_visible(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|p| {
        let normal = p.truncate();
        let corner = Vec3::select(normal.cmpge(Vec3::ZERO), max, min);
        normal.dot(corner) + p.w >= 0.0
    })
}

#[cfg(test)]
mod tests {
    use glam::camera::rh::proj::directx::perspective;
    use glam::camera::rh::view::look_to_mat4;
    use glam::vec3;

    use super::*;

    fn view_proj() -> Mat4 {
        let view = look_to_mat4(vec3(0.0, 1.0, 5.0), Vec3::NEG_Z, Vec3::Y);
        perspective(60f32.to_radians(), 16.0 / 9.0, 0.1, 50.0) * view
    }

    #[test]
    fn boxes_in_front_are_kept_and_boxes_behind_or_aside_are_culled() {
        let planes = planes(view_proj());
        let unit = |x: f32, y: f32, z: f32| {
            let c = vec3(x, y, z);
            box_visible(&planes, c - 0.5, c + 0.5)
        };
        assert!(unit(0.0, 1.0, 0.0));
        assert!(!unit(0.0, 1.0, 10.0), "behind the eye");
        assert!(!unit(40.0, 1.0, 0.0), "far to the right");
        assert!(!unit(0.0, 1.0, -60.0), "past the far plane");
        assert!(unit(0.0, 1.0, 4.95), "straddling the near plane");
    }

    #[test]
    fn a_box_across_a_frustum_corner_is_kept() {
        let planes = planes(view_proj());
        assert!(box_visible(
            &planes,
            vec3(-100.0, -100.0, -10.0),
            vec3(100.0, 100.0, -9.0)
        ));
    }
}
