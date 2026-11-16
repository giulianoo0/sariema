// SPDX-License-Identifier: Apache-2.0

// Each test crate that declares `mod common` uses only some of these.
#![allow(dead_code)]

use image::RgbaImage;
use sariema_render::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Quality, RenderPath, compare_images};

/// Where the downlevel path's renders are kept, for comparing them by eye.
pub fn downlevel_dir() -> String {
    let dir = concat!(env!("CARGO_TARGET_TMPDIR"), "/downlevel");
    std::fs::create_dir_all(dir).unwrap();
    dir.to_string()
}

/// Saves `img` among the downlevel path's renders and checks it against the golden `name`.
pub fn matches_golden_on_the_downlevel_path(img: &RgbaImage, name: &str, golden: &str) {
    img.save(format!("{}/{name}.png", downlevel_dir())).unwrap();
    let golden = image::open(golden).unwrap().to_rgba8();
    let d = compare_images(img, &golden).unwrap();
    assert!(
        d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO,
        "{name}: {d:?}"
    );
}

pub fn downlevel() -> Quality {
    Quality {
        path: RenderPath::Downlevel,
        ..Quality::high()
    }
}
