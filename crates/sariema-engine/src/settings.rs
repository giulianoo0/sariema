// SPDX-License-Identifier: Apache-2.0

//! Every number the glue between the modules uses, with its default. Each
//! module's own settings stay in its crate.

use sariema_anim::AnimSettings;
use sariema_audio::MixerSettings;
use sariema_physics::Settings as PhysicsSettings;
use sariema_ui::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct EngineSettings {
    /// The collision module's settings. The engine sets its step to the
    /// runtime's and caps its turn speed at `motion.turn_rate`.
    pub physics: PhysicsSettings,
    pub anim: AnimSettings,
    /// Seconds a `knockback` impulse pushes for; its m/s times this is the
    /// distance, given out fastest first.
    pub knockback_s: f32,
    /// How long a traversal's contact holds, as a fraction of the traversal
    /// on each side of the moment the actor's feet are nearest to it: the
    /// body eases onto the contact and off it over that fraction.
    pub contact_blend: f32,
    pub audio: SoundSettings,
    /// The mixer's settings under the game's sound bank, which brings its
    /// own buses; a host playing to a device sets its sample rate.
    pub mixer: MixerSettings,
    pub hud: HudSettings,
}

impl Default for EngineSettings {
    fn default() -> EngineSettings {
        EngineSettings {
            physics: PhysicsSettings::default(),
            anim: AnimSettings::default(),
            knockback_s: 0.25,
            contact_blend: 0.25,
            audio: SoundSettings::default(),
            mixer: MixerSettings::default(),
            hud: HudSettings::default(),
        }
    }
}

/// How a sound tied to an entity is heard from the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundSettings {
    /// Metres within which a sound plays at its full gain.
    pub reference_distance: f32,
    /// How fast the gain falls beyond that: gain = reference /
    /// (reference + rolloff × (distance − reference)), the inverse-distance
    /// model of OpenAL and Web Audio.
    pub rolloff: f32,
    /// Sounds followed at once; the oldest is let go past this.
    pub emitters: usize,
    /// A followed sound's gain or pan changing by less than this is not sent.
    pub min_change: f32,
    /// Streamed sounds decode on a thread of their own, for a host playing
    /// to a device. Off, [`crate::Engine::feed_audio`] decodes them between
    /// steps and the mixed sound is the same every run. Ignored on wasm32,
    /// which has no threads.
    pub stream_thread: bool,
    /// Streamed sounds are decoded whole when the bank loads, so playing
    /// them never decodes: no allocation anywhere after loading, at the cost
    /// of their samples in memory (f32 stereo at the mixer's rate, 23 MB a
    /// minute at 48 kHz).
    pub predecode_streams: bool,
    /// Requests a step can leave to the next feed (a sound to decode or
    /// open as a stream, any music request); one past it is refused with
    /// `E_AUDIO_QUEUE_FULL`.
    pub pending: usize,
}

impl Default for SoundSettings {
    fn default() -> SoundSettings {
        SoundSettings {
            reference_distance: 2.0,
            rolloff: 1.0,
            emitters: 32,
            min_change: 1e-3,
            stream_thread: false,
            predecode_streams: false,
            pending: 16,
        }
    }
}

/// The layer the engine draws over the HUD: dialogue, cinema bars, fade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudSettings {
    /// Height of each cinema bar at `letterbox` 1, as a fraction of the screen.
    pub bar_fraction: f32,
    /// Size of a dialogue line's text, in UI pixels.
    pub line_size: f32,
    /// Space between a dialogue line and the bottom bar, or the bottom of
    /// the screen without bars, in UI pixels.
    pub line_margin: f32,
    /// Space either side of a dialogue line, as a fraction of the screen's width.
    pub line_inset: f32,
    /// Space around a line's text, vertical then horizontal, in UI pixels.
    pub line_padding: [f32; 2],
    pub line_radius: f32,
    pub line_fill: Color,
    pub line_color: Color,
    /// Bytes of text one `say` line or `ui_text` may hold; a longer one is
    /// refused with `E_UI_TEXT_TOO_LONG`, since shaping it runs in the step.
    pub max_text_bytes: usize,
    /// Screen size until the host gives one.
    pub width: f32,
    pub height: f32,
    /// Hides the game's HUD while a cutscene holds control and until its
    /// cinema bars are gone; a node of its root in the group
    /// [`KEEP_IN_CUTSCENES`](crate::KEEP_IN_CUTSCENES) stays.
    pub hide_in_cutscenes: bool,
}

impl Default for HudSettings {
    fn default() -> HudSettings {
        HudSettings {
            bar_fraction: 0.12,
            line_size: 22.0,
            line_margin: 48.0,
            line_inset: 0.12,
            line_padding: [10.0, 18.0],
            line_radius: 12.0,
            line_fill: Color([0.0, 0.0, 0.0, 0.7]),
            line_color: Color::WHITE,
            max_text_bytes: 4096,
            width: 1280.0,
            height: 720.0,
            hide_in_cutscenes: true,
        }
    }
}
