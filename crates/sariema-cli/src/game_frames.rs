// SPDX-License-Identifier: Apache-2.0

//! Frames of a headless game run, drawn as the window player draws them:
//! `--screenshot` alone keeps the last step's, with `--filmstrip N` it
//! keeps N steps spread over the run, or over `--span` seconds from
//! `--time`, tiled.

use std::path::PathBuf;

use image::RgbaImage;
use sariema_core::STEP_SECONDS;
use sariema_engine::Engine;
use sariema_format::script::GameAssets;
use sariema_render::Headless;
use sariema_runtime::sim::Logic;
use sariema_ui::SafeArea;
use serde_json::{Value, json};

use crate::RunArgs;
use crate::filmstrip;
use crate::output::Failure;
use sariema_view::GameView;

pub struct Frames {
    headless: Headless,
    view: GameView,
    /// The steps whose frame is kept, in order.
    at: Vec<u64>,
    images: Vec<RgbaImage>,
    aspect: f32,
    columns: u32,
    path: PathBuf,
}

impl Frames {
    /// None unless the run asked for a screenshot.
    pub fn new<L: Logic>(
        args: &RunArgs,
        assets: Option<&GameAssets>,
        engine: &mut Engine<L>,
        steps: u64,
    ) -> Result<Option<Frames>, Failure> {
        let (Some(path), Some(assets)) = (&args.screenshot, assets) else {
            return Ok(None);
        };
        let count = u64::from(args.stage.filmstrip.unwrap_or(1)).min(steps);
        let at = match args.stage.span {
            Some(span) => {
                let first = ((args.stage.time / STEP_SECONDS).round() as u64).min(steps - 1);
                let last = (((args.stage.time + span) / STEP_SECONDS).round() as u64)
                    .clamp(first, steps - 1);
                let gaps = (count - 1).max(1);
                (0..count)
                    .map(|i| first + i * (last - first) / gaps)
                    .collect()
            }
            None => (1..=count).map(|i| i * steps / count - 1).collect(),
        };
        let mut headless = Headless::new(args.width, args.height, args.quality.settings())?;
        let view = GameView::load(headless.renderer(), assets, engine)?;
        engine.resize(args.width as f32, args.height as f32, SafeArea::default());
        Ok(Some(Frames {
            headless,
            view,
            at,
            images: Vec::new(),
            aspect: args.width as f32 / args.height as f32,
            columns: args.stage.columns,
            path: path.clone(),
        }))
    }

    /// Draws the frame that shows step `step` when it is one kept.
    pub fn after_step<L: Logic>(
        &mut self,
        step: u64,
        engine: &mut Engine<L>,
    ) -> Result<(), Failure> {
        let (view, aspect) = (&mut self.view, self.aspect);
        if self.at.get(self.images.len()) != Some(&step) {
            // A host draws a frame a step; the rooms load and show between kept frames too.
            return Ok(self
                .headless
                .paint(|renderer, _| view.update_rooms(renderer, engine, 1.0))?);
        }
        self.headless
            .paint(|renderer, target| view.draw(renderer, engine, 1.0, aspect, target))?;
        self.images.push(self.headless.image());
        Ok(())
    }

    /// Saves the frames kept, tiled, and describes them.
    pub fn finish(mut self) -> Result<Value, Failure> {
        let image = filmstrip::tile(&self.images, self.columns);
        image
            .save(&self.path)
            .map_err(|e| Failure::io(&self.path, e))?;
        Ok(json!({
            "screenshot": self.path.display().to_string(),
            "steps": self.at,
            "columns": self.columns.min(self.images.len() as u32),
            "frame_stats": crate::run::frame_stats_json(&self.headless.renderer().last_frame_stats()),
        }))
    }
}
