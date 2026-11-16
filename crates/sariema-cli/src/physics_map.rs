// SPDX-License-Identifier: Apache-2.0

//! The `physics-map` command: a top-down picture of a collision pack, north
//! up, for checking a bake without a screen.
//!
//! Colliders are grey, lighter the higher their top, and shaded by its slope;
//! disabled ones are striped orange. Green covers the cells the actor can
//! stand on, yellow the triggers, blue discs with a white facing line the
//! spots, magenta the path. Faint lines mark every metre.

use std::path::{Path, PathBuf};

use clap::Args;
use glam::{Vec2, Vec3};
use image::{Rgb, RgbImage};
use sariema_format::collision::{CollisionPack, Shape};
use sariema_physics::{ActorId, ColliderId, Intent, Settings, World, spot_actor, yaw_matrix};
use serde_json::{Value, json};

use crate::collision;
use crate::output::{Done, Failure};

#[derive(Args)]
pub struct MapArgs {
    /// Collision pack, conventionally `.srmc`.
    pack: PathBuf,
    /// PNG to write.
    #[arg(short, long)]
    output: PathBuf,
    /// Draw the path between two spots, as `<from>:<to>`, for the first spot's actor.
    #[arg(long)]
    path: Option<String>,
    /// Actor whose walkable cells to draw, when no path names one [default: the first].
    #[arg(long)]
    actor: Option<String>,
    /// [default: 40]
    #[arg(long)]
    pixels_per_metre: Option<f32>,
    /// Longest side in pixels; a larger room is drawn at fewer pixels per metre [default: 8192].
    #[arg(long)]
    max_side: Option<u32>,
}

pub fn run(args: &MapArgs) -> Result<Done, Failure> {
    let defaults = MapSettings::default();
    let settings = MapSettings {
        pixels_per_metre: args.pixels_per_metre.unwrap_or(defaults.pixels_per_metre),
        max_side: args.max_side.unwrap_or(defaults.max_side),
        ..defaults
    };
    if !(settings.pixels_per_metre > 0.0
        && settings.pixels_per_metre.is_finite()
        && settings.max_side > 0)
    {
        return Err(Failure::new(
            "E_USAGE",
            "--pixels-per-metre and --max-side must be positive",
        ));
    }
    physics_map(
        &args.pack,
        &args.output,
        args.path.as_deref(),
        args.actor.as_deref(),
        &settings,
    )
}

#[derive(Clone, Copy, Debug)]
pub struct MapSettings {
    pub pixels_per_metre: f32,
    /// Metres of border around the colliders.
    pub margin: f32,
    /// Longest side in pixels; a larger room is drawn at fewer pixels per metre.
    pub max_side: u32,
}

impl Default for MapSettings {
    fn default() -> MapSettings {
        MapSettings {
            pixels_per_metre: 40.0,
            margin: 0.5,
            max_side: 8192,
        }
    }
}

const BACKGROUND: [u8; 3] = [20, 24, 32];
const DISABLED: [u8; 3] = [230, 140, 40];
const WALKABLE: [u8; 3] = [60, 200, 90];
const TRIGGER: [u8; 3] = [240, 210, 60];
const SPOT: [u8; 3] = [70, 140, 255];
const FACING: [u8; 3] = [255, 255, 255];
const PATH: [u8; 3] = [230, 60, 220];

/// Pixel geometry: the world (x, z) of the top-left corner and the scale.
struct Frame {
    origin: Vec2,
    ppm: f32,
    width: u32,
    height: u32,
}

impl Frame {
    fn pixel(&self, x: f32, z: f32) -> (i64, i64) {
        (
            ((x - self.origin.x) * self.ppm).floor() as i64,
            ((z - self.origin.y) * self.ppm).floor() as i64,
        )
    }

    fn centre(&self, px: u32, py: u32) -> Vec2 {
        self.origin + (Vec2::new(px as f32, py as f32) + 0.5) / self.ppm
    }
}

fn blend(image: &mut RgbImage, px: i64, py: i64, colour: [u8; 3], amount: f32) {
    if px < 0 || py < 0 || px >= image.width() as i64 || py >= image.height() as i64 {
        return;
    }
    let pixel = image.get_pixel_mut(px as u32, py as u32);
    for k in 0..3 {
        pixel.0[k] = (pixel.0[k] as f32 * (1.0 - amount) + colour[k] as f32 * amount).round() as u8;
    }
}

/// `path` is `<from spot>:<to spot>`; the actor is the path's, else `actor`,
/// else the pack's first.
pub fn physics_map(
    pack_path: &Path,
    output: &Path,
    path: Option<&str>,
    actor: Option<&str>,
    settings: &MapSettings,
) -> Result<Done, Failure> {
    let pack = collision::read(pack_path)?;
    let route = path.map(|p| route(&pack, p)).transpose()?;
    let body = match (route, actor) {
        (Some((from, _)), _) => Some(pack.spots[from].actor as usize),
        (None, Some(name)) => Some(
            pack.actors
                .iter()
                .position(|a| a.name == name)
                .ok_or_else(|| Failure::new("E_USAGE", format!("the pack has no actor {name}")))?,
        ),
        (None, None) => (!pack.actors.is_empty()).then_some(0),
    };
    let mut world = World::from_pack(Settings::default(), &pack)
        .map_err(|e| Failure::new("E_SCHEMA", format!("{}: {e}", pack_path.display())))?;

    let bounds = world.static_bounds();
    let lo = Vec2::new(bounds.min.x, bounds.min.z) - settings.margin;
    let hi = Vec2::new(bounds.max.x, bounds.max.z) + settings.margin;
    let origin = lo.floor();
    let size = hi.ceil() - origin;
    let ppm = settings
        .pixels_per_metre
        .min(settings.max_side as f32 / size.max_element());
    let frame = Frame {
        origin,
        ppm,
        width: (size.x * ppm).ceil() as u32,
        height: (size.y * ppm).ceil() as u32,
    };

    let walker = body.map(|b| {
        let spot = pack.spots.iter().find(|s| s.actor as usize == b);
        let desc = match spot {
            Some(spot) => spot_actor(&pack, spot),
            None => sariema_physics::ActorDesc {
                collider: sariema_physics::actor_collider(&pack.actors[b]),
                ..Default::default()
            },
        };
        world.add_actor(desc)
    });
    let walker: Option<ActorId> = walker
        .transpose()
        .map_err(|e| Failure::new("E_SCHEMA", format!("{}: {e}", pack_path.display())))?;
    let mut waypoints = Vec::new();
    let found = match (route, walker) {
        (Some((from, to)), Some(id)) => world.find_path(
            id,
            pack.spots[from].position.into(),
            pack.spots[to].position.into(),
            &mut waypoints,
        ),
        _ => false,
    };
    let cells: Vec<Vec3> = walker.map_or_else(Vec::new, |id| world.walkable_cells(id).collect());
    let cell = world.settings().nav.cell_size;
    for i in 0..world.collider_count() {
        world.apply(Intent::SetColliderEnabled {
            collider: ColliderId(i as u32),
            enabled: true,
        });
    }

    let mut image = RgbImage::from_pixel(frame.width, frame.height, Rgb(BACKGROUND));
    draw_colliders(
        &mut image,
        &frame,
        &world,
        &pack,
        bounds.min.y,
        bounds.max.y,
    );
    draw_grid(&mut image, &frame);
    for c in &cells {
        let (x0, y0) = frame.pixel(c.x - cell * 0.5, c.z - cell * 0.5);
        let (x1, y1) = frame.pixel(c.x + cell * 0.5, c.z + cell * 0.5);
        for py in y0..y1.max(y0 + 1) {
            for px in x0..x1.max(x0 + 1) {
                blend(&mut image, px, py, WALKABLE, 0.45);
            }
        }
    }
    for t in &pack.triggers {
        draw_footprint(&mut image, &frame, &t.shape);
    }
    let mut spots = Vec::new();
    for s in &pack.spots {
        let radius = pack.actors[s.actor as usize].radius;
        let at = Vec3::from(s.position);
        let (px, py) = frame.pixel(at.x, at.z);
        let reach = (radius * ppm).ceil() as i64;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if ((dx * dx + dy * dy) as f32) <= (radius * ppm) * (radius * ppm) {
                    blend(&mut image, px + dx, py + dy, SPOT, 0.85);
                }
            }
        }
        let tip = at + yaw_matrix(s.yaw) * Vec3::Z * radius * 1.5;
        line(&mut image, &frame, at, tip, FACING, 0);
        spots.push(json!({ "name": s.name, "actor": pack.actors[s.actor as usize].name, "pixel": [px, py] }));
    }
    for pair in waypoints.windows(2) {
        line(&mut image, &frame, pair[0], pair[1], PATH, 1);
    }
    image.save(output).map_err(|e| Failure::io(output, e))?;

    let path_report = route.map(|(from, to)| {
        json!({
            "from": pack.spots[from].name,
            "to": pack.spots[to].name,
            "found": found,
            "points": waypoints.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
        })
    });
    Ok(Done {
        json: json!({
            "output": output.display().to_string(),
            "width": frame.width,
            "height": frame.height,
            "pixels_per_metre": ppm,
            "origin": [origin.x, origin.y],
            "actor": body.map(|b| pack.actors[b].name.clone()),
            "spots": spots,
            "path": path_report.clone().unwrap_or(Value::Null),
        }),
        text: format!(
            "drew {} into {}: {}×{} px, {ppm} px/m, top-left at x {} z {}{}",
            pack.room,
            output.display(),
            frame.width,
            frame.height,
            origin.x,
            origin.y,
            match path_report {
                Some(p) if found => format!(
                    ", path of {} points",
                    p["points"].as_array().map_or(0, Vec::len)
                ),
                Some(_) => ", no path".to_owned(),
                None => String::new(),
            }
        ),
    })
}

fn route(pack: &CollisionPack, path: &str) -> Result<(usize, usize), Failure> {
    let (from, to) = path.split_once(':').ok_or_else(|| {
        Failure::new(
            "E_USAGE",
            format!("--path {path}: expected <from spot>:<to spot>"),
        )
    })?;
    let index = |name: &str| {
        pack.spots
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| {
                Failure::new(
                    "E_USAGE",
                    format!("--path {path}: the pack has no spot {name}"),
                )
            })
    };
    Ok((index(from)?, index(to)?))
}

/// The top surface under every pixel, by a ray cast straight down.
fn draw_colliders(
    image: &mut RgbImage,
    frame: &Frame,
    world: &World,
    pack: &CollisionPack,
    bottom: f32,
    top: f32,
) {
    let light = Vec3::new(-0.4, 1.0, -0.3).normalize();
    let span = (top - bottom).max(f32::EPSILON);
    for py in 0..frame.height {
        for px in 0..frame.width {
            let c = frame.centre(px, py);
            let Some(hit) = world.raycast(Vec3::new(c.x, top + 1.0, c.y), Vec3::NEG_Y, span + 2.0)
            else {
                continue;
            };
            let height = ((hit.point.y - bottom) / span).clamp(0.0, 1.0);
            let shade = 0.55 + 0.45 * hit.normal.dot(light).max(0.0);
            let grey = ((70.0 + 160.0 * height) * shade) as u8;
            image.put_pixel(px, py, Rgb([grey; 3]));
            if !pack.colliders[hit.collider.0 as usize].enabled && (px + py) / 4 % 2 == 0 {
                blend(image, px as i64, py as i64, DISABLED, 0.7);
            }
        }
    }
}

fn draw_grid(image: &mut RgbImage, frame: &Frame) {
    let metres = |pixels: u32| (pixels as f32 / frame.ppm).ceil() as i64;
    for m in 0..=metres(frame.width) {
        let (px, _) = frame.pixel(frame.origin.x + m as f32, 0.0);
        for py in 0..frame.height {
            blend(image, px, py as i64, [0; 3], 0.18);
        }
    }
    for m in 0..=metres(frame.height) {
        let (_, py) = frame.pixel(0.0, frame.origin.y + m as f32);
        for px in 0..frame.width {
            blend(image, px as i64, py, [0; 3], 0.18);
        }
    }
}

/// Fills the trigger's outline seen from above and draws its edge.
fn draw_footprint(image: &mut RgbImage, frame: &Frame, shape: &Shape) {
    let inside = |p: Vec2| -> bool {
        match shape {
            Shape::Box {
                center,
                half_extents,
                rotation,
            } => {
                let m = glam::Mat3::from_cols_array(rotation);
                let local = m.transpose() * (Vec3::new(p.x, center[1], p.y) - Vec3::from(*center));
                local.x.abs() <= half_extents[0] && local.z.abs() <= half_extents[2]
            }
            Shape::Sphere { center, radius } => {
                p.distance(Vec2::new(center[0], center[2])) <= *radius
            }
            Shape::Capsule { a, b, radius } => {
                let (a, b) = (Vec2::new(a[0], a[2]), Vec2::new(b[0], b[2]));
                let ab = b - a;
                let t = if ab.length_squared() > 0.0 {
                    ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                p.distance(a + ab * t) <= *radius
            }
            Shape::Hull { points }
            | Shape::Mesh {
                vertices: points, ..
            } => {
                let (lo, hi) = points
                    .iter()
                    .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), q| {
                        (lo.min(Vec2::new(q[0], q[2])), hi.max(Vec2::new(q[0], q[2])))
                    });
                p.cmpge(lo).all() && p.cmple(hi).all()
            }
        }
    };
    let step = 1.0 / frame.ppm;
    for py in 0..frame.height {
        for px in 0..frame.width {
            let c = frame.centre(px, py);
            if !inside(c) {
                continue;
            }
            let edge = [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y]
                .iter()
                .any(|&d| !inside(c + d * step));
            blend(
                image,
                px as i64,
                py as i64,
                TRIGGER,
                if edge { 0.95 } else { 0.25 },
            );
        }
    }
}

/// A line `thickness` pixels either side of `a`–`b` seen from above.
fn line(image: &mut RgbImage, frame: &Frame, a: Vec3, b: Vec3, colour: [u8; 3], thickness: i64) {
    let (x0, y0) = frame.pixel(a.x, a.z);
    let (x1, y1) = frame.pixel(b.x, b.z);
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for i in 0..=steps {
        let x = x0 + (x1 - x0) * i / steps;
        let y = y0 + (y1 - y0) * i / steps;
        for dy in -thickness..=thickness {
            for dx in -thickness..=thickness {
                blend(image, x + dx, y + dy, colour, 1.0);
            }
        }
    }
}
