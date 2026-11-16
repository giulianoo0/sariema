// SPDX-License-Identifier: Apache-2.0

//! The page's options for the engine, as the JSON object it passes.

use sariema_render::{Quality, RenderPath};
use serde::Deserialize;

use crate::clock::StepSettings;
use crate::feed::FeedSettings;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// WebGPU where the browser has it, else WebGL2.
    #[default]
    Auto,
    WebGpu,
    WebGl2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathChoice {
    #[default]
    Auto,
    Full,
    Downlevel,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebSettings {
    pub backend: Backend,
    /// The renderer's quality tier. Default medium.
    pub quality: Tier,
    /// Default auto: WebGPU takes the full path and WebGL2 the downlevel one.
    pub path: PathChoice,
    /// Samples per pixel instead of the tier's; the device may hold fewer.
    pub msaa: Option<u32>,
    /// Physical pixels per CSS pixel at most; a phone's 3 is cut to this.
    /// Default 1.5, what the three.js game draws at.
    pub max_pixel_ratio: f32,
    pub step: StepSettings,
    pub audio: FeedSettings,
    /// Quads the UI holds before its buffer grows. Default 1024.
    pub ui_quads: u32,
    /// Sample compressed textures where the GPU can. False loads the RGBA8
    /// packs on every device, as one without ASTC does. Default true.
    pub compressed_textures: bool,
    /// Lights and the sun cast the shadows their room gives them. Default true.
    pub shadows: bool,
    /// Screen-space ambient occlusion where the tier has it. Default true.
    pub ambient_occlusion: bool,
    /// Texels per row of the downlevel path's light, cluster and shadow
    /// tables instead of the tier's (1024).
    pub data_texture_width: Option<u32>,
    /// Rows of one actor's uniform record on the downlevel path instead of
    /// the tier's (1024, 16 KiB); the device's uniform limit caps it.
    pub actor_uniform_rows: Option<u32>,
}

impl Default for WebSettings {
    fn default() -> WebSettings {
        WebSettings {
            backend: Backend::Auto,
            quality: Tier::Medium,
            path: PathChoice::Auto,
            msaa: None,
            max_pixel_ratio: 1.5,
            step: StepSettings::default(),
            audio: FeedSettings::default(),
            ui_quads: 1024,
            compressed_textures: true,
            shadows: true,
            ambient_occlusion: true,
            data_texture_width: None,
            actor_uniform_rows: None,
        }
    }
}

impl WebSettings {
    pub fn from_json(json: &str) -> Result<WebSettings, String> {
        if json.trim().is_empty() {
            return Ok(WebSettings::default());
        }
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    pub fn render_quality(&self) -> Quality {
        let tier = match self.quality {
            Tier::Low => Quality::low(),
            Tier::Medium => Quality::medium(),
            Tier::High => Quality::high(),
        };
        Quality {
            msaa: self.msaa.unwrap_or(tier.msaa),
            ao_samples: if self.ambient_occlusion {
                tier.ao_samples
            } else {
                0
            },
            path: match self.path {
                PathChoice::Auto => RenderPath::Auto,
                PathChoice::Full => RenderPath::Full,
                PathChoice::Downlevel => RenderPath::Downlevel,
            },
            data_texture_width: self.data_texture_width.unwrap_or(tier.data_texture_width),
            actor_uniform_rows: self.actor_uniform_rows.unwrap_or(tier.actor_uniform_rows),
            ..tier
        }
    }

    /// The canvas's size in physical pixels for a `css` size at `ratio`.
    pub fn physical_size(&self, css: (f32, f32), ratio: f32) -> (u32, u32) {
        let ratio = ratio.clamp(0.25, self.max_pixel_ratio.max(0.25));
        let side = |v: f32| ((v * ratio).round() as u32).max(1);
        (side(css.0), side(css.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_options_take_every_default() {
        assert_eq!(WebSettings::from_json("").unwrap(), WebSettings::default());
        assert_eq!(
            WebSettings::from_json("{}").unwrap(),
            WebSettings::default()
        );
    }

    #[test]
    fn options_override_by_name_and_unknown_ones_fail() {
        let s = WebSettings::from_json(
            r#"{"backend":"webgl2","quality":"high","msaa":1,"step":{"step_s":0.01},"compressed_textures":false}"#,
        )
        .unwrap();
        assert_eq!(s.backend, Backend::WebGl2);
        assert_eq!(s.render_quality().msaa, 1);
        assert_eq!(s.step.step_s, 0.01);
        assert_eq!(s.step.max_steps_per_frame, 4);
        assert!(!s.compressed_textures);
        let wrong = WebSettings::from_json(r#"{"quallity":"high"}"#).unwrap_err();
        assert!(wrong.contains("quallity"), "{wrong}");
    }

    #[test]
    fn the_downlevel_tables_are_set_by_option() {
        let s = WebSettings::from_json(r#"{"data_texture_width":2048,"actor_uniform_rows":512}"#)
            .unwrap();
        let q = s.render_quality();
        assert_eq!(q.data_texture_width, 2048);
        assert_eq!(q.actor_uniform_rows, 512);
        let tier = WebSettings::default().render_quality();
        assert_eq!(
            tier.data_texture_width,
            Quality::medium().data_texture_width
        );
        assert_eq!(
            tier.actor_uniform_rows,
            Quality::medium().actor_uniform_rows
        );
    }

    #[test]
    fn the_pixel_ratio_is_capped() {
        let s = WebSettings::default();
        assert_eq!(s.physical_size((390.0, 844.0), 3.0), (585, 1266));
        assert_eq!(s.physical_size((800.0, 600.0), 1.0), (800, 600));
        assert_eq!(s.physical_size((0.0, 0.0), 1.0), (1, 1));
    }
}
