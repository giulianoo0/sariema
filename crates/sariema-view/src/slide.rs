// SPDX-License-Identifier: Apache-2.0

//! The camera of a game without a rig: its room's own, sliding to the next
//! room's on a damped spring across a continuous switch.

use sariema_core::STEP_SECONDS;
use sariema_engine::Engine;
use sariema_format::RoomCamera;
use sariema_format::script::RoomTransition;
use sariema_runtime::sim::Logic;

const SUBSTEPS: u32 = 4;

pub(crate) struct Slide {
    frequency: f32,
    damping: f32,
    /// The camera shown when the slide began.
    from: RoomCamera,
    /// The room slid to, and the step it was at last.
    room: usize,
    step: u64,
    /// How far it is, 0 to 1 (a little past it, damped), at the step before
    /// the last and at the last.
    done: [f32; 2],
    velocity: f32,
}

impl Slide {
    pub(crate) fn new(
        room: usize,
        step: u64,
        camera: RoomCamera,
        frequency: f32,
        damping: f32,
    ) -> Slide {
        Slide {
            frequency,
            damping,
            from: camera,
            room,
            step,
            done: [1.0; 2],
            velocity: 0.0,
        }
    }

    /// Follows the engine to its last step: a continuous switch starts a
    /// slide from the camera shown, any other cuts to the next room's.
    pub(crate) fn advance<L: Logic>(
        &mut self,
        engine: &Engine<L>,
        cameras: &dyn Fn(usize) -> RoomCamera,
    ) {
        let now = engine.steps();
        let active = engine.state().scenery.active();
        let mut steps = now.saturating_sub(self.step);
        if active != self.room {
            let continuous = (engine.switched())
                .and_then(|s| s.switch.map(|w| (s.step, w.kind)))
                .filter(|&(_, kind)| kind == RoomTransition::Continuous);
            self.from = self.at(&cameras(self.room), 1.0);
            self.velocity = 0.0;
            self.room = active;
            match continuous {
                Some((at, _)) if self.frequency > 0.0 && self.frequency.is_finite() => {
                    self.done = [0.0; 2];
                    steps = now.saturating_sub(at);
                }
                _ => {
                    self.done = [1.0; 2];
                    steps = 0;
                }
            }
        }
        self.step = now;
        for _ in 0..steps {
            self.done[0] = self.done[1];
            if self.done[1] != 1.0 || self.velocity != 0.0 {
                self.spring();
            }
        }
    }

    fn spring(&mut self) {
        let (w, z) = (self.frequency, self.damping.max(0.0));
        let h = STEP_SECONDS / SUBSTEPS as f32;
        let (mut x, mut v) = (self.done[1], self.velocity);
        for _ in 0..SUBSTEPS {
            v += (w * w * (1.0 - x) - 2.0 * z * w * v) * h;
            x += v * h;
        }
        if (1.0 - x).abs() < 1e-4 && v.abs() < 1e-3 {
            (x, v) = (1.0, 0.0);
        }
        self.done[1] = x;
        self.velocity = v;
    }

    /// The camera at `alpha` between the last two steps, sliding to `to`.
    pub(crate) fn at(&self, to: &RoomCamera, alpha: f32) -> RoomCamera {
        let t = self.done[0] + (self.done[1] - self.done[0]) * alpha;
        if t == 1.0 {
            return to.clone();
        }
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let (a, b) = (&self.from, to);
        RoomCamera {
            fov_y_deg: mix(a.fov_y_deg, b.fov_y_deg),
            pitch_deg: mix(a.pitch_deg, b.pitch_deg),
            distance: mix(a.distance, b.distance),
            focus: [0, 1, 2].map(|i| mix(a.focus[i], b.focus[i])),
        }
    }
}
