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
    let packs = packs(dir)?;
    if update {
        update_all(dir, &packs)
    } else {
        compare_all(dir, &packs)
    }
}

/// Renders every pack before writing any reference, so a pack that fails
/// leaves all references as they were.
fn update_all(dir: &Path, packs: &[PathBuf]) -> Result<Done, Failure> {
    if packs.is_empty() {
        return Err(Failure::new(
            "E_USAGE",
            format!("{}: no .srma pack to update", dir.display()),
        ));
    }
    let images = packs
        .iter()
        .map(|path| render(path))
        .collect::<Result<Vec<_>, _>>()?;
    for (path, image) in packs.iter().zip(&images) {
        let golden = path.with_extension("golden.png");
        image.save(&golden).map_err(|e| Failure::io(&golden, e))?;
    }
    let updated: Vec<String> = packs.iter().map(|path| file_name(path)).collect();
    Ok(Done {
        json: json!({ "passed": [], "updated": updated, "skipped": [] }),
        text: format!("updated {}", list(&updated)),
    })
}

fn compare_all(dir: &Path, packs: &[PathBuf]) -> Result<Done, Failure> {
    let (mut passed, mut skipped, mut mismatches) = (Vec::new(), Vec::new(), Vec::new());
    for path in packs {
        let name = file_name(path);
        let golden = path.with_extension("golden.png");
        if !golden.exists() {
            skipped.push(name);
            continue;
        }
        let image = render(path)?;
        let reference = image::open(&golden)
            .map_err(|e| Failure::io(&golden, e))?
            .to_rgba8();
        let actual = path.with_extension("actual.png");
        let why = match compare_images(&image, &reference) {
            // A largest delta within GOLDEN_MAX_DELTA gives a ratio of 0, so the ratio decides.
            Some(d) if d.differing_pixels_ratio <= GOLDEN_MAX_RATIO => {
                remove_stale(&actual)?;
                passed.push(name);
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
    if passed.is_empty() {
        return Err(Failure::new(
            "E_USAGE",
            format!(
                "{}: no pack with a reference was found (X.golden.png beside X.srma)",
                dir.display()
            ),
        ));
    }
    Ok(Done {
        json: json!({ "passed": passed, "updated": [], "skipped": skipped }),
        text: format!(
            "{} passed; skipped, without a reference: {}",
            passed.len(),
            list(&skipped)
        ),
    })
}

/// Deletes the render an earlier mismatch left at `path`, if any.
fn remove_stale(path: &Path) -> Result<(), Failure> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(Failure::io(path, e)),
        _ => Ok(()),
    }
}

fn file_name(path: &Path) -> String {
    let name = path.file_name().expect("a read_dir entry has a name");
    name.to_string_lossy().into_owned()
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
