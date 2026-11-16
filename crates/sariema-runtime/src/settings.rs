// SPDX-License-Identifier: Apache-2.0

//! Every number that bounds or tunes the runtime, with its default. A game
//! overrides any of them in its project's `settings`; `sariema api` lists them.

use sariema_format::script::Ease;
use sariema_vm::VmSettings;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::camera::CameraSettings;
use crate::input::InputSettings;
use crate::motion::MotionSettings;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    pub max_steps_per_frame: u32,
    pub max_entities: u32,
    pub spring_rest: f32,
    pub min_time_scale: f32,
    pub motion: MotionSettings,
    pub camera: CameraSettings,
    pub input: InputSettings,
    pub cutscene: CutsceneSettings,
    pub vm: VmSettings,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            max_steps_per_frame: 5,
            max_entities: 65_536,
            spring_rest: 1e-3,
            min_time_scale: 0.05,
            motion: MotionSettings::default(),
            camera: CameraSettings::default(),
            input: InputSettings::default(),
            cutscene: CutsceneSettings::default(),
            vm: VmSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CutsceneSettings {
    pub start_tolerance: f32,
    pub start_blend_max: f32,
    pub start_blend: f32,
    pub start_walk_speed: f32,
    pub end_blend: f32,
    #[serde(serialize_with = "ease_out", deserialize_with = "ease_in")]
    pub end_ease: Ease,
    pub skip_fade: f32,
    pub advance_action: String,
    pub skip_action: String,
}

impl Default for CutsceneSettings {
    fn default() -> CutsceneSettings {
        CutsceneSettings {
            start_tolerance: 0.02,
            start_blend_max: 0.3,
            start_blend: 0.25,
            start_walk_speed: 1.5,
            end_blend: 0.6,
            end_ease: Ease::InOutCubic,
            skip_fade: 0.3,
            advance_action: "confirm".into(),
            skip_action: "skip".into(),
        }
    }
}

pub(crate) fn ease_out<S: Serializer>(e: &Ease, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(e.name())
}

pub(crate) fn ease_in<'de, D: Deserializer<'de>>(d: D) -> Result<Ease, D::Error> {
    let name = String::deserialize(d)?;
    Ease::from_name(&name).ok_or_else(|| serde::de::Error::custom(format!("unknown ease {name:?}")))
}

/// What each setting does, by its dotted path.
pub const DOCS: &[(&str, &str)] = &[
    (
        "max_steps_per_frame",
        "Fixed steps one display frame may run; the time beyond is dropped, so a stall slows the game instead of freezing it.",
    ),
    (
        "max_entities",
        "Entity slots a game may use; a spawn at a higher index is rejected with E_ENTITY_LIMIT.",
    ),
    (
        "min_time_scale",
        "The slowest a time_scale may play the world; a smaller scale is raised to it, so the world never stops.",
    ),
    (
        "spring_rest",
        "A spring stops, exactly on its target, once its offset and velocity are both within this.",
    ),
    (
        "motion.builtin",
        "The runtime moves actors itself; off when a collision module consumes the movement intents.",
    ),
    (
        "motion.turn_rate",
        "Degrees per second an actor turns; nothing turns faster, and a larger turn in one step is a jump.",
    ),
    (
        "motion.arrive_distance",
        "Metres from a walk target that count as arrived; a larger move in one step is a jump.",
    ),
    (
        "motion.camera_relative",
        "The hero's move axis turns with the camera's view direction.",
    ),
    (
        "camera.dead_zone",
        "Metres a followed subject moves before the camera follows.",
    ),
    (
        "camera.frequency",
        "Hz of the spring that carries the camera's focus.",
    ),
    (
        "camera.damping",
        "Damping of that spring; 1 is critical, below overshoots.",
    ),
    (
        "camera.reframe_blend",
        "Seconds the camera blends when the rig in use is reframed; 0 cuts.",
    ),
    ("camera.reframe_ease", "Easing of that blend."),
    (
        "camera.look_ahead",
        "Seconds of the subject's velocity the focus leads by.",
    ),
    (
        "input.axes",
        "Move axes: name, keys for up, down, left and right, an optional stick, whether the touch joystick drives it, and a dead zone.",
    ),
    ("input.buttons", "Buttons: name, keys and gamepad buttons."),
    (
        "input.buffer_steps",
        "Steps a button press stays buffered for the game to consume.",
    ),
    (
        "cutscene.start_tolerance",
        "Metres from its declared start an actor may be and still start in place.",
    ),
    (
        "cutscene.start_blend_max",
        "Up to this distance an actor is eased into its start place; farther, it walks there.",
    ),
    ("cutscene.start_blend", "Seconds of that easing."),
    (
        "cutscene.start_walk_speed",
        "Metres per second of the walk to the start place.",
    ),
    (
        "cutscene.end_blend",
        "Seconds of the camera blend back to the game's rig when a cutscene ends.",
    ),
    ("cutscene.end_ease", "Easing of that blend."),
    (
        "cutscene.skip_fade",
        "Seconds of the fade a skip runs under, out and back in.",
    ),
    (
        "cutscene.advance_action",
        "Button that ends a tap dialogue line.",
    ),
    (
        "cutscene.skip_action",
        "Button that skips a skippable cutscene.",
    ),
    (
        "vm.memory_bytes",
        "Ceiling of the game's memory; growing past it stops the game with E_GAME_MEMORY.",
    ),
    (
        "vm.fuel_per_step",
        "Execution budget of one tick, about one unit per instruction; exceeding it stops the game with E_GAME_BUDGET.",
    ),
    (
        "vm.fuel_first_step",
        "Budget of the first tick, which usually builds the world.",
    ),
    ("vm.table_elements", "Elements a table may grow to."),
    ("vm.max_call_depth", "Nested calls before the game traps."),
    ("vm.max_regions", "Bulk data regions a game may register."),
    ("vm.log_bytes_per_step", "Bytes of log text kept per tick."),
    ("vm.log_lines_per_step", "Log lines kept per tick."),
    (
        "vm.commands_bytes_per_step",
        "Bytes of commands one tick may submit; more stops the game with E_GAME_BUDGET.",
    ),
];
