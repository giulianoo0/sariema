// SPDX-License-Identifier: Apache-2.0

//! The frame loop's clock. The simulation advances in fixed steps; the page
//! calls once per display frame at whatever rate the display runs, and the
//! renderer draws between the last two steps. The wall clock only decides
//! how many steps a frame runs, never what a step computes.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StepSettings {
    /// Seconds of simulation per step. Default 1/60.
    pub step_s: f64,
    /// Most steps one frame runs; past it the simulation falls behind the
    /// wall clock instead of spending ever longer frames catching up.
    /// Default 4.
    pub max_steps_per_frame: u32,
}

impl Default for StepSettings {
    fn default() -> StepSettings {
        StepSettings {
            step_s: 1.0 / 60.0,
            max_steps_per_frame: 4,
        }
    }
}

/// What one display frame does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub steps: u32,
    /// Where the frame lies between the last two steps, 0 to 1.
    pub alpha: f32,
}

#[derive(Clone, Debug)]
pub struct FixedStep {
    settings: StepSettings,
    last_ms: Option<f64>,
    /// Seconds of wall clock not yet stepped.
    carry_s: f64,
}

impl FixedStep {
    pub fn new(settings: StepSettings) -> FixedStep {
        FixedStep {
            settings,
            last_ms: None,
            carry_s: 0.0,
        }
    }

    pub fn settings(&self) -> &StepSettings {
        &self.settings
    }

    /// The steps due at `now_ms`, the page's clock in milliseconds. The
    /// first call after [`FixedStep::new`] or [`FixedStep::reset`] runs none.
    pub fn advance(&mut self, now_ms: f64) -> Tick {
        let step = self.settings.step_s;
        let elapsed = match self.last_ms {
            Some(last) => ((now_ms - last) / 1000.0).max(0.0),
            None => 0.0,
        };
        self.last_ms = Some(now_ms);
        self.carry_s += elapsed;
        // A display at exactly the step's rate must not drop a step to
        // rounding: summed frame times fall a hair short of whole steps.
        let due = (self.carry_s / step + 1e-6).floor();
        let steps = due.min(self.settings.max_steps_per_frame as f64);
        self.carry_s -= steps * step;
        if due > steps {
            // Behind by more than a frame may catch up: keep only the
            // part of a step, for the interpolation.
            self.carry_s = self.carry_s.rem_euclid(step);
        }
        self.carry_s = self.carry_s.max(0.0);
        Tick {
            steps: steps as u32,
            alpha: (self.carry_s / step).clamp(0.0, 1.0) as f32,
        }
    }

    /// Forgets the time since the last frame, so that after a pause the
    /// simulation resumes where it stopped instead of catching up.
    pub fn reset(&mut self) {
        self.last_ms = None;
        self.carry_s = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> FixedStep {
        FixedStep::new(StepSettings::default())
    }

    #[test]
    fn a_60_hz_display_runs_one_step_a_frame() {
        let mut c = clock();
        assert_eq!(c.advance(1000.0).steps, 0);
        let steps: u32 = (1..=600)
            .map(|i| c.advance(1000.0 + i as f64 * 1000.0 / 60.0).steps)
            .sum();
        assert_eq!(steps, 600);
    }

    #[test]
    fn a_120_hz_display_steps_every_other_frame_and_interpolates() {
        let mut c = clock();
        c.advance(0.0);
        let ticks: Vec<Tick> = (1..=4)
            .map(|i| c.advance(i as f64 * 1000.0 / 120.0))
            .collect();
        assert_eq!(
            ticks.iter().map(|t| t.steps).collect::<Vec<_>>(),
            [0, 1, 0, 1]
        );
        assert!((ticks[0].alpha - 0.5).abs() < 1e-3, "{ticks:?}");
        assert!(ticks[1].alpha < 1e-3, "{ticks:?}");
    }

    #[test]
    fn a_long_frame_is_capped_and_not_caught_up_later() {
        let mut c = clock();
        c.advance(0.0);
        assert_eq!(c.advance(1000.0).steps, 4);
        assert_eq!(c.advance(1000.0 + 1000.0 / 60.0).steps, 1);
    }

    #[test]
    fn reset_forgets_the_pause() {
        let mut c = clock();
        c.advance(0.0);
        c.advance(10.0);
        c.reset();
        assert_eq!(c.advance(60_000.0).steps, 0);
        assert_eq!(c.advance(60_000.0 + 1000.0 / 60.0).steps, 1);
    }

    #[test]
    fn time_going_back_runs_nothing() {
        let mut c = clock();
        c.advance(500.0);
        assert_eq!(
            c.advance(400.0),
            Tick {
                steps: 0,
                alpha: 0.0
            }
        );
    }
}
