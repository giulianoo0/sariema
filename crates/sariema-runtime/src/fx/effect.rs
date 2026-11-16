// SPDX-License-Identifier: Apache-2.0

//! Particle effects as a game defines them in its settings, and the limits
//! of the particles a run keeps.

use serde::{Deserialize, Serialize};

/// The sprite a particle is drawn with, from the renderer's built-in set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A soft round spot.
    #[default]
    Dot,
    /// Four thin rays around a bright core.
    Star,
    /// A disc with a soft rim and a flat middle.
    Disc,
    /// A spot stretched along x.
    Spark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FxSettings {
    pub emitters: u32,
    pub pool: u32,
    pub path_points: u32,
    pub effects: Vec<Effect>,
    pub trails: u32,
    pub trail_points: u32,
    pub trail_dots: u32,
    pub trail: TrailStyle,
    pub highlights: u32,
    pub pulse_depth: f32,
}

impl Default for FxSettings {
    fn default() -> FxSettings {
        FxSettings {
            emitters: 32,
            pool: 64,
            path_points: 32,
            effects: Vec::new(),
            trails: 4,
            trail_points: 64,
            trail_dots: 256,
            trail: TrailStyle::default(),
            highlights: 16,
            pulse_depth: 0.4,
        }
    }
}

/// How every trail's dots are drawn; what each trail shows (spacing, size,
/// colour, intensity, the wave's speed, the fades at its ends) is in its
/// command.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TrailStyle {
    pub sprite: Shape,
    /// Metres above the route's points, so the dots never sink into the floor.
    pub lift: f32,
    /// Metres from the wave's crest to where it has fallen to e⁻² of it.
    pub wave_length: f32,
    /// Metres between one crest and the next.
    pub wave_every: f32,
    /// How much brighter, and larger, a dot is at the crest.
    pub wave_gain: f32,
    pub wave_grow: f32,
    pub additive: f32,
}

impl Default for TrailStyle {
    fn default() -> TrailStyle {
        TrailStyle {
            sprite: Shape::Disc,
            lift: 0.02,
            wave_length: 1.0,
            wave_every: 8.0,
            wave_gain: 1.0,
            wave_grow: 0.25,
            additive: 1.0,
        }
    }
}

/// One effect: how its particles are born, move, change and are drawn.
/// Ranges are `[least, most]`, each particle picking within them; keys are
/// over its life, 0 at birth and 1 at death.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Effect {
    pub name: String,
    pub sprite: Shape,
    /// Lies on the ground, facing up, instead of facing the camera.
    pub flat: bool,
    /// Particles alive at once from one emitter; `fx.pool` when absent.
    pub pool: Option<u32>,
    /// Particles per second while the emitter runs.
    pub rate: f32,
    /// Particles at once when the emitter starts, then every `every`
    /// seconds; 0 seconds puffs once.
    pub puff: u32,
    pub every: f32,
    /// Seconds.
    pub life: [f32; 2],
    /// Metres across at birth.
    pub size: [f32; 2],
    /// `[t, scale]` keys multiplying the size; none keeps it.
    pub grow: Vec<[f32; 2]>,
    /// Metres per second at birth, along `direction` within `spread`
    /// degrees of it.
    pub speed: [f32; 2],
    pub direction: [f32; 3],
    pub spread: f32,
    /// Metres around the emitter's place a particle is born within.
    pub radius: f32,
    /// Metres per second squared; positive y rises.
    pub gravity: [f32; 3],
    /// Fraction of its speed a particle loses per second, as `e^(−drag·t)`.
    pub drag: f32,
    /// A sideways sway of this many metres per second, `drift_hz` times a
    /// second.
    pub drift: f32,
    pub drift_hz: f32,
    /// Degrees per second the sprite turns.
    pub spin: [f32; 2],
    /// `[t, r, g, b, a]` keys, linear RGB; none is white and opaque.
    pub color: Vec<[f32; 5]>,
    /// Multiplies the colour; above 1 it blooms.
    pub intensity: f32,
    /// 1 adds its light to what is behind, 0 covers it, between both.
    pub additive: f32,
}

impl Default for Effect {
    fn default() -> Effect {
        Effect {
            name: String::new(),
            sprite: Shape::Dot,
            flat: false,
            pool: None,
            rate: 0.0,
            puff: 0,
            every: 0.0,
            life: [1.0, 1.0],
            size: [0.1, 0.1],
            grow: Vec::new(),
            speed: [0.0, 0.0],
            direction: [0.0, 1.0, 0.0],
            spread: 0.0,
            radius: 0.0,
            gravity: [0.0; 3],
            drag: 0.0,
            drift: 0.0,
            drift_hz: 0.5,
            spin: [0.0, 0.0],
            color: Vec::new(),
            intensity: 1.0,
            additive: 1.0,
        }
    }
}

impl FxSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.emitters == 0 || self.pool == 0 {
            return Err("fx.emitters and fx.pool must be at least 1".into());
        }
        let t = &self.trail;
        let style = [
            t.lift,
            t.wave_length,
            t.wave_every,
            t.wave_gain,
            t.wave_grow,
            t.additive,
        ];
        if style.iter().any(|v| !v.is_finite())
            || t.wave_length <= 0.0
            || t.wave_every <= 0.0
            || !(0.0..=1.0).contains(&t.additive)
        {
            return Err(
                "fx.trail: wave_length and wave_every must be positive, additive within 0 and 1"
                    .into(),
            );
        }
        if !(0.0..=1.0).contains(&self.pulse_depth) {
            return Err("fx.pulse_depth must be within 0 and 1".into());
        }
        for (i, e) in self.effects.iter().enumerate() {
            let name = &e.name;
            if name.is_empty() {
                return Err(format!("fx.effects[{i}] has no name"));
            }
            if self.effects[..i].iter().any(|o| &o.name == name) {
                return Err(format!("fx effect {name:?} is defined twice"));
            }
            e.validate()
                .map_err(|why| format!("fx effect {name:?}: {why}"))?;
        }
        Ok(())
    }
}

impl Effect {
    fn validate(&self) -> Result<(), String> {
        let numbers = [
            self.rate,
            self.every,
            self.spread,
            self.radius,
            self.drag,
            self.drift,
            self.drift_hz,
            self.intensity,
            self.additive,
        ]
        .into_iter()
        .chain(self.life)
        .chain(self.size)
        .chain(self.speed)
        .chain(self.direction)
        .chain(self.gravity)
        .chain(self.spin)
        .chain(self.grow.iter().flatten().copied())
        .chain(self.color.iter().flatten().copied());
        if numbers.into_iter().any(|v| !v.is_finite()) {
            return Err("every number must be finite".into());
        }
        let ordered = |[lo, hi]: [f32; 2]| lo <= hi;
        if self.life[0] <= 0.0 || !ordered(self.life) {
            return Err("life must be positive, least first".into());
        }
        for (field, range) in [
            ("size", self.size),
            ("speed", self.speed),
            ("spin", self.spin),
        ] {
            if !ordered(range) || (field != "spin" && range[0] < 0.0) {
                return Err(format!("{field} must be least first, not negative"));
            }
        }
        if [
            self.rate,
            self.every,
            self.radius,
            self.drag,
            self.drift,
            self.spread,
        ]
        .iter()
        .any(|&v| v < 0.0)
        {
            return Err("rate, every, radius, drag, drift and spread must not be negative".into());
        }
        if !(0.0..=1.0).contains(&self.additive) {
            return Err("additive must be within 0 and 1".into());
        }
        if self.pool == Some(0) {
            return Err("pool must be at least 1".into());
        }
        keys_in_order(self.grow.iter().map(|k| k[0])).map_err(|why| format!("grow {why}"))?;
        keys_in_order(self.color.iter().map(|k| k[0])).map_err(|why| format!("color {why}"))?;
        Ok(())
    }
}

fn keys_in_order(mut times: impl Iterator<Item = f32>) -> Result<(), &'static str> {
    let mut last = 0.0;
    times.try_for_each(|t| {
        if !(last..=1.0).contains(&t) {
            return Err("keys must run from 0 to 1 in order");
        }
        last = t;
        Ok(())
    })
}

/// The keys' value at `t`, linear between them and held past the ends.
pub(crate) fn sample<const N: usize>(keys: &[[f32; N]], t: f32, empty: [f32; N]) -> [f32; N] {
    let Some(first) = keys.first() else {
        return empty;
    };
    if t <= first[0] {
        return *first;
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b[0] {
            let f = if b[0] > a[0] {
                (t - a[0]) / (b[0] - a[0])
            } else {
                1.0
            };
            return std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f);
        }
    }
    *keys.last().unwrap_or(first)
}
