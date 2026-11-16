// SPDX-License-Identifier: Apache-2.0

//! The `test` command: each `X.srma` of a directory against the
//! `X.golden.png` beside it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use sariema_core::room_camera;
use sariema_render::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, compare_images, render_to_image};
use serde_json::json;

use crate::output::{Done, Failure, list};
use crate::pack;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;

/// With `update`, rewrites every reference instead of comparing.
pub fn test(dir: &Path, update: bool) -> Result<Done, Failure> {
    let (mut passed, mut updated, mut unreferenced) = (Vec::new(), Vec::new(), Vec::new());
    let mut mismatches = Vec::new();
    for path in packs(dir)? {
        let name = path.file_name().expect("read_dir entry").to_string_lossy();
        let golden = path.with_extension("golden.png");
        if !update && !golden.exists() {
            unreferenced.push(name.into_owned());
            continue;
        }
        let image = render(&path)?;
        if update {
            image.save(&golden).map_err(|e| Failure::io(&golden, e))?;
            updated.push(name.into_owned());
            continue;
        }
        let reference = image::open(&golden)
            .map_err(|e| Failure::io(&golden, e))?
            .to_rgba8();
        let why = match compare_images(&image, &reference) {
            Some(d)
                if d.max_channel_delta <= GOLDEN_MAX_DELTA
                    || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO =>
            {
                passed.push(name.into_owned());
                continue;
            }
            Some(d) => format!(
                "{:.2}% of pixels differ by more than {GOLDEN_MAX_DELTA}, at most by {}",
                d.differing_pixels_ratio * 100.0,
                d.max_channel_delta
            ),
            None => format!(
                "the reference is {}×{}, not {WIDTH}×{HEIGHT}",
                reference.width(),
                reference.height()
            ),
        };
        let actual = path.with_extension("actual.png");
        image.save(&actual).map_err(|e| Failure::io(&actual, e))?;
        mismatches.push(format!(
            "{name} differs from {}: {why}; the render is in {}",
            golden.display(),
            actual.display()
        ));
    }
    if !mismatches.is_empty() {
        return Err(Failure::new("E_GOLDEN_MISMATCH", mismatches.join("; ")));
    }
    Ok(Done {
        json: json!({
            "passed": passed,
            "updated": updated,
            "no_reference": unreferenced,
        }),
        text: format!(
            "{} passed, {} updated; without a reference: {}",
            passed.len(),
            updated.len(),
            list(&unreferenced)
        ),
    })
}

/// The directory's `.srma` files, sorted.
fn packs(dir: &Path) -> Result<Vec<PathBuf>, Failure> {
    let mut packs = fs::read_dir(dir)
        .and_then(|entries| {
            entries
                .map(|e| e.map(|e| e.path()))
                .collect::<io::Result<Vec<_>>>()
        })
        .map_err(|e| Failure::io(dir, e))?;
    packs.retain(|p| p.extension().is_some_and(|e| e == "srma"));
    packs.sort();
    Ok(packs)
}

fn render(path: &Path) -> Result<RgbaImage, Failure> {
    let room = pack::read(path)?;
    let camera = room_camera(&room.camera, WIDTH as f32 / HEIGHT as f32);
    Ok(render_to_image(&room, &camera, WIDTH, HEIGHT, 0.0)?)
}
