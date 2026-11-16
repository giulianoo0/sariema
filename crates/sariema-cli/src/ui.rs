// SPDX-License-Identifier: Apache-2.0

//! `sariema ui render`: draws a UI description at a screen size and a
//! moment, headless, into a PNG, so a screen can be checked without a device.

use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, value_parser};
use image::{Rgba, RgbaImage, imageops};
use sariema_render::ui::{UiBatch, UiHeadless, UiInstance, UiTextureFormat};
use sariema_ui::{Color, Event, Prop, Quad, SafeArea, Ui, UiDesc, UiError};
use serde_json::{Value, json};

use crate::output::{Done, Failure};

const UI_SCHEMA: &str = include_str!("../../../schemas/ui.schema.json");

const _: () = {
    assert!(size_of::<Quad>() == size_of::<UiInstance>());
    assert!(offset_of!(Quad, rect) == offset_of!(UiInstance, rect));
    assert!(offset_of!(Quad, xform) == offset_of!(UiInstance, xform));
    assert!(offset_of!(Quad, uv) == offset_of!(UiInstance, uv));
    assert!(offset_of!(Quad, fill) == offset_of!(UiInstance, fill));
    assert!(offset_of!(Quad, stroke) == offset_of!(UiInstance, stroke));
    assert!(offset_of!(Quad, radii) == offset_of!(UiInstance, radii));
    assert!(offset_of!(Quad, radii_y) == offset_of!(UiInstance, radii_y));
    assert!(offset_of!(Quad, params) == offset_of!(UiInstance, params));
    assert!(offset_of!(Quad, kind) == offset_of!(UiInstance, kind));
};

#[derive(Args)]
pub struct UiArgs {
    #[command(subcommand)]
    command: UiCommand,
}

#[derive(Subcommand)]
enum UiCommand {
    /// Draw a UI description (schemas/ui.schema.json) into a PNG.
    Render {
        description: PathBuf,
        /// Screen width in pixels.
        #[arg(long, value_parser = value_parser!(u32).range(1..=16384))]
        width: u32,
        #[arg(long, value_parser = value_parser!(u32).range(1..=16384))]
        height: u32,
        /// Safe-area insets in pixels: left,top,right,bottom.
        #[arg(long, value_parser = insets)]
        safe: Option<SafeArea>,
        /// Applied in order before the first step: `node.prop=value`
        /// (x, y, scale, scale_x, scale_y, rotation, opacity, width, height,
        /// progress, reveal, fill, text, visible), `play=animation`, or
        /// `down=`, `move=`, `up=pointer:x,y` in pixels.
        #[arg(long = "state")]
        state: Vec<String>,
        /// Seconds of simulation steps before the frame is drawn.
        #[arg(long, default_value_t = 0.0, value_parser = seconds)]
        time: f32,
        /// Draw this many frames evenly from 0 to --time, side by side.
        #[arg(long, value_parser = value_parser!(u32).range(1..=64))]
        filmstrip: Option<u32>,
        /// Under the UI: a PNG or JPEG scaled to cover the screen, or a #colour.
        #[arg(long)]
        background: Option<String>,
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn insets(s: &str) -> Result<SafeArea, String> {
    let v: Vec<f32> = s
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    match v[..] {
        [left, top, right, bottom] if v.iter().all(|x| x.is_finite() && *x >= 0.0) => {
            Ok(SafeArea {
                left,
                top,
                right,
                bottom,
            })
        }
        _ => Err("expected four non-negative numbers: left,top,right,bottom".into()),
    }
}

fn seconds(s: &str) -> Result<f32, String> {
    match s.parse::<f32>() {
        Ok(v) if v >= 0.0 && v.is_finite() => Ok(v),
        Ok(_) => Err("must be zero or more".into()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn run(args: &UiArgs) -> Result<Done, Failure> {
    match &args.command {
        UiCommand::Render {
            description,
            width,
            height,
            safe,
            state,
            time,
            filmstrip,
            background,
            output,
        } => render(&Render {
            description,
            size: [*width, *height],
            safe: safe.unwrap_or_default(),
            state,
            time: *time,
            frames: filmstrip.unwrap_or(1),
            background: background.as_deref(),
            output,
        }),
    }
}

impl From<UiError> for Failure {
    fn from(e: UiError) -> Failure {
        Failure::new(e.code(), e.to_string())
    }
}

struct Render<'a> {
    description: &'a Path,
    size: [u32; 2],
    safe: SafeArea,
    state: &'a [String],
    time: f32,
    frames: u32,
    background: Option<&'a str>,
    output: &'a Path,
}

fn load(path: &Path) -> Result<Ui, Failure> {
    let text = std::fs::read_to_string(path).map_err(|e| Failure::io(path, e))?;
    let schema_error = |why: String| Failure::new("E_SCHEMA", format!("{}: {why}", path.display()));
    let value: Value = serde_json::from_str(&text).map_err(|e| schema_error(e.to_string()))?;
    let schema: Value = serde_json::from_str(UI_SCHEMA).expect("the embedded schema is JSON");
    jsonschema::validate(&schema, &value).map_err(|e| {
        let at = e.instance_path().to_string();
        schema_error(if at.is_empty() {
            e.to_string()
        } else {
            format!("{at}: {e}")
        })
    })?;
    let desc: UiDesc = serde_json::from_value(value).map_err(|e| schema_error(e.to_string()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    Ok(Ui::from_desc(&desc, |file| {
        let p = base.join(file);
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    })?)
}

fn render(r: &Render) -> Result<Done, Failure> {
    let mut ui = load(r.description)?;
    let [w, h] = r.size;
    ui.set_screen(w as f32, h as f32, r.safe);
    for s in r.state {
        apply(&mut ui, s)?;
    }
    let background = r.background.map(|b| background(b, w, h)).transpose()?;
    let steps = (r.time / ui.settings().step_s).round() as u32;
    let mut gpu = UiHeadless::new(w, h, 1024)?;
    let mut strip = RgbaImage::new(w * r.frames, h);
    let mut done = 0u32;
    let mut events = Vec::new();
    let mut last = (0, 0, 0);
    for frame in 0..r.frames {
        let target = if r.frames == 1 {
            steps
        } else {
            (steps as f32 * frame as f32 / (r.frames - 1) as f32).round() as u32
        };
        while done < target {
            done += 1;
            let happened = ui.step().to_vec();
            events.extend(happened.iter().map(|e| event_json(&ui, done, e)));
        }
        if ui.take_atlas_changed() {
            let (side, texels) = ui.atlas();
            gpu.pass().set_texture(
                sariema_ui::ATLAS_TEXTURE,
                side,
                side,
                UiTextureFormat::R8,
                texels,
            );
        }
        for (i, image) in ui.images().iter().enumerate() {
            let id = 1 + i as u32;
            gpu.pass().set_texture(
                id,
                image.width,
                image.height,
                UiTextureFormat::Rgba8,
                &image.rgba,
            );
        }
        let list = ui.draw(1.0);
        let batches: Vec<UiBatch> = list
            .batches
            .iter()
            .map(|b| UiBatch {
                texture: b.texture,
                scissor: b.scissor,
                start: b.start,
                count: b.count,
            })
            .collect();
        let instances: &[UiInstance] = bytemuck::cast_slice(&list.quads);
        let quads = instances.len();
        let mut image = gpu.render(instances, &batches);
        last = (quads, batches.len(), gpu.pass().draw_calls());
        composite(&mut image, background.as_ref());
        imageops::replace(&mut strip, &image, i64::from(frame * w), 0);
    }
    strip.save(r.output).map_err(|e| Failure::io(r.output, e))?;
    let missing: Vec<Value> = ui
        .names()
        .filter_map(|(name, id)| ui.missing_glyphs(id).filter(|&n| n > 0).map(|n| (name, n)))
        .map(|(name, n)| json!({ "node": name, "characters": n }))
        .collect();
    let (quads, batches, draw_calls) = last;
    let text = format!(
        "drew {} frame{} of {w} × {h} into {}: {quads} quads in {draw_calls} draw call{}{}",
        r.frames,
        if r.frames == 1 { "" } else { "s" },
        r.output.display(),
        if draw_calls == 1 { "" } else { "s" },
        if missing.is_empty() {
            String::new()
        } else {
            format!("; {} text nodes lack glyphs", missing.len())
        },
    );
    Ok(Done {
        json: json!({
            "output": r.output.display().to_string(),
            "width": strip.width(),
            "height": strip.height(),
            "frames": r.frames,
            "steps": done,
            "nodes": ui.len(),
            "quads": quads,
            "batches": batches,
            "draw_calls": draw_calls,
            "missing_glyphs": missing,
            "events": events,
        }),
        text,
    })
}

fn usage(s: &str, why: &str) -> Failure {
    Failure::new("E_USAGE", format!("--state {s:?}: {why}"))
}

fn apply(ui: &mut Ui, s: &str) -> Result<(), Failure> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| usage(s, "expected key=value"))?;
    match key {
        "play" => return Ok(ui.play(value)?),
        "down" | "move" | "up" => {
            let (pointer, at) = value
                .split_once(':')
                .ok_or_else(|| usage(s, "expected pointer:x,y"))?;
            let pointer: u64 = pointer
                .parse()
                .map_err(|_| usage(s, "the pointer is a number"))?;
            let (x, y) = at
                .split_once(',')
                .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)))
                .ok_or_else(|| usage(s, "expected pointer:x,y"))?;
            match key {
                "down" => ui.pointer_down(pointer, x, y),
                "move" => ui.pointer_move(pointer, x, y),
                _ => ui.pointer_up(pointer, x, y),
            }
            return Ok(());
        }
        _ => {}
    }
    let (name, field) = key
        .rsplit_once('.')
        .ok_or_else(|| usage(s, "expected node.prop=value, play=, down=, move= or up="))?;
    let node = ui
        .node(name)
        .ok_or_else(|| Failure::new("E_UI_UNKNOWN", format!("--state {s:?}: no node {name:?}")))?;
    let number = || {
        value
            .parse::<f32>()
            .map_err(|_| usage(s, "expected a number"))
    };
    let props: &[Prop] = match field {
        "text" => return Ok(ui.set_text(node, value)?),
        "visible" => {
            let v = value
                .parse::<bool>()
                .map_err(|_| usage(s, "expected true or false"))?;
            return Ok(ui.set_visible(node, v)?);
        }
        "fill" => {
            let c = Color::parse(value).map_err(|e| usage(s, &e))?;
            return Ok(ui.set_fill(node, c)?);
        }
        "x" => &[Prop::X],
        "y" => &[Prop::Y],
        "scale" => &[Prop::ScaleX, Prop::ScaleY],
        "scale_x" => &[Prop::ScaleX],
        "scale_y" => &[Prop::ScaleY],
        "rotation" => &[Prop::Rotation],
        "opacity" => &[Prop::Opacity],
        "width" => &[Prop::Width],
        "height" => &[Prop::Height],
        "progress" => &[Prop::Progress],
        "reveal" => &[Prop::Reveal],
        _ => return Err(usage(s, &format!("no property {field:?}"))),
    };
    let v = number()?;
    for &p in props {
        ui.set(node, p, v)?;
    }
    Ok(())
}

fn event_json(ui: &Ui, step: u32, e: &Event) -> Value {
    let name = |n| ui.name(n).map_or(Value::Null, |s| Value::String(s.into()));
    let (kind, node, extra) = match *e {
        Event::Press {
            node,
            pointer,
            x,
            y,
        } => ("press", node, json!({ "pointer": pointer, "x": x, "y": y })),
        Event::Drag {
            node,
            pointer,
            x,
            y,
        } => ("drag", node, json!({ "pointer": pointer, "x": x, "y": y })),
        Event::Release {
            node,
            pointer,
            x,
            y,
        } => (
            "release",
            node,
            json!({ "pointer": pointer, "x": x, "y": y }),
        ),
        Event::Click { node } => ("click", node, json!({})),
        Event::Stick { node, x, y } => ("stick", node, json!({ "x": x, "y": y })),
        Event::Tap { node } => ("tap", node, json!({})),
        Event::Held { node } => ("held", node, json!({})),
        Event::Focus { node } => ("focus", node, json!({})),
    };
    let mut v = json!({ "step": step, "event": kind, "node": name(node) });
    v.as_object_mut()
        .expect("an object")
        .extend(extra.as_object().expect("an object").clone());
    v
}

/// Straight-alpha background pixels of the screen's size.
fn background(spec: &str, w: u32, h: u32) -> Result<RgbaImage, Failure> {
    if spec.starts_with('#') {
        let c = Color::parse(spec)
            .map_err(|e| Failure::new("E_USAGE", format!("--background: {e}")))?;
        let px = c.0.map(|v| (v * 255.0).round() as u8);
        return Ok(RgbaImage::from_pixel(w, h, Rgba(px)));
    }
    let path = Path::new(spec);
    let img = image::open(path)
        .map_err(|e| Failure::io(path, e))?
        .to_rgba8();
    let (iw, ih) = img.dimensions();
    let k = (w as f32 / iw as f32).max(h as f32 / ih as f32);
    let (sw, sh) = (
        (iw as f32 * k).round() as u32,
        (ih as f32 * k).round() as u32,
    );
    let scaled = imageops::resize(&img, sw.max(w), sh.max(h), imageops::FilterType::Triangle);
    let (x, y) = ((scaled.width() - w) / 2, (scaled.height() - h) / 2);
    Ok(imageops::crop_imm(&scaled, x, y, w, h).to_image())
}

/// Turns the premultiplied render into straight alpha over `background`.
fn composite(image: &mut RgbaImage, background: Option<&RgbaImage>) {
    for (x, y, px) in image.enumerate_pixels_mut() {
        let [r, g, b, a] = px.0.map(f32::from);
        let out = match background {
            Some(bg) => {
                let [br, bg_, bb, ba] = bg.get_pixel(x, y).0.map(f32::from);
                let k = 1.0 - a / 255.0;
                let oa = a + ba * k;
                if oa <= 0.0 {
                    [0.0; 4]
                } else {
                    // Background straight → premultiplied, composited, back.
                    let mix = |c: f32, bc: f32| (c + bc * ba / 255.0 * k) * 255.0 / oa;
                    [mix(r, br), mix(g, bg_), mix(b, bb), oa]
                }
            }
            None if a > 0.0 => [r * 255.0 / a, g * 255.0 / a, b * 255.0 / a, a],
            None => [0.0; 4],
        };
        px.0 = out.map(|v| v.round().clamp(0.0, 255.0) as u8);
    }
}
