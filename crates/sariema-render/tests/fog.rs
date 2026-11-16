// SPDX-License-Identifier: Apache-2.0

//! Fog: the colour of what is drawn goes towards the fog's with its distance
//! from the camera, glows fade instead of taking its colour, the ambient
//! occlusion fades with the surface it darkens, and the background stays,
//! on both render paths.

mod common;

use common::downlevel;
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{AlphaMode, Fog, RoomPack, Shading};
use sariema_render::{Headless, Lighting, Quality, compare_images};

fn paths() -> [Quality; 2] {
    [Quality::high(), downlevel()]
}

const W: u32 = 320;
const H: u32 = 180;

fn camera_of(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

/// `room` drawn with `fog`, and with the draw named `hidden` hidden.
fn render(room: &RoomPack, fog: Option<Fog>, hidden: Option<&str>, quality: &Quality) -> RgbaImage {
    let mut h = Headless::new(W, H, quality.clone()).unwrap();
    h.renderer().load_room(room).unwrap();
    if let Some(fog) = fog {
        h.renderer().set_lighting(Some(Lighting {
            fog,
            ..Lighting::of(room)
        }));
    }
    if let Some(name) = hidden {
        assert!(h.renderer().set_draw_visible(name, false), "no draw {name}");
    }
    h.render(&camera_of(room), 0.0)
}

/// Fog that covers everything a millimetre from the camera.
fn thick(color: [f32; 3]) -> Fog {
    Fog {
        color,
        density: 0.0,
        start: 0.0,
        end: 0.001,
    }
}

#[test]
fn fog_that_begins_beyond_everything_changes_no_pixel() {
    let room = demo_room();
    let far = Fog {
        color: [0.0, 1.0, 0.0],
        density: 3.0,
        start: 500.0,
        end: 600.0,
    };
    for q in &paths() {
        let clear = render(&room, None, None, q);
        assert_eq!(render(&room, Some(far), None, q), clear, "{:?}", q.path);
        assert_eq!(render(&room, Some(Fog::default()), None, q), clear);
    }
}

#[test]
fn thick_fog_hides_every_kind_of_surface_and_its_occlusion() {
    let fog = Some(thick([0.15, 0.3, 0.2]));
    let modes = [AlphaMode::Opaque, AlphaMode::Blend, AlphaMode::Additive];
    for (q, alpha) in paths().iter().flat_map(|q| modes.map(|a| (q, a))) {
        let mut room = demo_room();
        let red = (room.draws.iter())
            .find(|d| d.name.as_deref() == Some("box_red"))
            .unwrap()
            .material as usize;
        room.materials[red].alpha = alpha;
        room.materials[red].base_color[3] = 0.5;
        room.materials[red].emissive = [2.0, 0.5, 0.2];
        let shown = render(&room, fog, None, q);
        let hidden = render(&room, fog, Some("box_red"), q);
        let diff = compare_images(&shown, &hidden).unwrap();
        assert!(
            diff.max_channel_delta <= 2,
            "the red box as {alpha:?} shows through thick fog on {:?}: {diff:?}",
            q.path
        );
    }
}

#[test]
fn fog_thickens_with_distance_and_leaves_the_background() {
    // A grey floor that no light changes, so a row's colour tells its fog
    // alone; the camera looks down at 45°, so lower rows are nearer.
    let mut room = demo_room();
    room.draws.retain(|d| d.name.as_deref() == Some("floor"));
    let floor = room.draws[0].material as usize;
    room.materials[floor].shading = Shading::Unlit;
    room.materials[floor].base_color_texture = None;
    room.materials[floor].base_color = [0.5, 0.5, 0.5, 1.0];
    let green = Fog {
        color: [0.0, 0.6, 0.0],
        density: 0.15,
        start: 2.0,
        end: 0.0,
    };
    // The middle fifth of a row, which the floor fills at every height.
    let greener = |image: &RgbaImage, y: u32| {
        let middle = 2 * W / 5..3 * W / 5;
        let sum: f32 = (middle.clone())
            .map(|x| image.get_pixel(x, y))
            .map(|p| p[1] as f32 - p[0] as f32)
            .sum();
        sum / middle.len() as f32
    };
    let empty = RoomPack {
        draws: Vec::new(),
        ..demo_room()
    };
    for q in &paths() {
        let clear = render(&room, None, None, q);
        let fogged = render(&room, Some(green), None, q);
        let rows = [H - 4, 3 * H / 4, H / 2].map(|y| greener(&fogged, y) - greener(&clear, y));
        assert!(
            rows[0] > 20.0 && rows.windows(2).all(|w| w[1] > w[0] + 5.0),
            "nearer rows take less fog on {:?}: {rows:?}",
            q.path
        );
        assert_eq!(
            render(&empty, Some(thick(green.color)), None, q),
            render(&empty, None, None, q),
            "the background takes no fog on {:?}",
            q.path
        );
    }
}
