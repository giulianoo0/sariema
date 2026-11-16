// SPDX-License-Identifier: Apache-2.0

//! Property animation on the fixed step: tweens with cubic Bézier easing,
//! springs set by response and damping, and the typewriter. An animation
//! that starts takes over its property from the value it has then, and a
//! spring keeps its velocity, so interrupting never jumps.

use serde::Deserialize;

/// A cubic Bézier from (0, 0) to (1, 1), as CSS `cubic-bezier`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(from = "EaseDesc")]
pub struct Ease {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EaseDesc {
    Named(Named),
    Points([f32; 4]),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Named {
    Linear,
    Out,
    InOut,
}

impl From<EaseDesc> for Ease {
    fn from(d: EaseDesc) -> Ease {
        match d {
            EaseDesc::Named(Named::Linear) => Ease::LINEAR,
            EaseDesc::Named(Named::Out) => Ease::OUT,
            EaseDesc::Named(Named::InOut) => Ease::IN_OUT,
            EaseDesc::Points([x1, y1, x2, y2]) => Ease { x1, y1, x2, y2 },
        }
    }
}

impl Ease {
    pub const LINEAR: Ease = Ease {
        x1: 0.0,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    /// For what enters, leaves or answers a touch.
    pub const OUT: Ease = Ease {
        x1: 0.22,
        y1: 1.0,
        x2: 0.36,
        y2: 1.0,
    };
    /// For what is already on screen and changes shape.
    pub const IN_OUT: Ease = Ease {
        x1: 0.65,
        y1: 0.0,
        x2: 0.35,
        y2: 1.0,
    };

    /// Progress at time fraction `x` in [0, 1].
    pub fn at(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let bezier = |a: f32, b: f32, t: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
        };
        // Newton's method on x(t) = x, falling back to bisection.
        let mut t = x;
        for _ in 0..8 {
            let err = bezier(self.x1, self.x2, t) - x;
            let u = 1.0 - t;
            let slope = 3.0 * u * u * self.x1
                + 6.0 * u * t * (self.x2 - self.x1)
                + 3.0 * t * t * (1.0 - self.x2);
            if err.abs() < 1e-6 {
                return bezier(self.y1, self.y2, t);
            }
            if slope.abs() < 1e-6 {
                break;
            }
            t = (t - err / slope).clamp(0.0, 1.0);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..40 {
            t = 0.5 * (lo + hi);
            if bezier(self.x1, self.x2, t) < x {
                lo = t;
            } else {
                hi = t;
            }
        }
        bezier(self.y1, self.y2, t)
    }
}

/// A spring set like iOS's: `response` seconds and `damping` (1 does not
/// overshoot).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spring {
    pub response: f32,
    #[serde(default = "one")]
    pub damping: f32,
}

fn one() -> f32 {
    1.0
}

impl From<Spring> for Motion {
    fn from(s: Spring) -> Motion {
        Motion::Spring {
            response: s.response,
            damping: s.damping,
        }
    }
}

/// How a property moves to its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Tween {
        duration: f32,
        ease: Ease,
    },
    /// Like iOS: stiffness (2π / response)², damping 4π · damping / response,
    /// mass 1. A damping of 1 does not overshoot.
    Spring {
        response: f32,
        damping: f32,
    },
    /// Reveals characters of a text: `rate` per second, waiting `pause`
    /// seconds after `.`, `!`, `?` and `…`.
    Typewriter {
        rate: f32,
        pause: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Anim {
    pub node: u32,
    pub prop: u8,
    pub from: Option<f32>,
    pub to: f32,
    pub motion: Motion,
    /// Seconds left before it starts.
    pub delay: f32,
    pub started: bool,
    pub elapsed: f32,
    pub velocity: f32,
    pub start_value: f32,
    /// Typewriter: seconds left in the current punctuation pause.
    pub hold: f32,
    /// Started this step: it moves only by the part of the step after its
    /// delay ran out, `-delay`.
    pub lead: bool,
    pub done: bool,
}

impl Anim {
    pub fn new(
        node: u32,
        prop: u8,
        from: Option<f32>,
        to: f32,
        motion: Motion,
        delay: f32,
    ) -> Anim {
        Anim {
            node,
            prop,
            from,
            to,
            motion,
            delay,
            started: false,
            elapsed: 0.0,
            velocity: 0.0,
            start_value: 0.0,
            hold: 0.0,
            lead: false,
            done: false,
        }
    }
}

pub(crate) struct SpringRest {
    pub hz: f32,
    pub delta: f32,
    pub speed: f32,
}

/// Advances a started animation by `dt` from `value`; returns the new value
/// and whether it is done. `text` is the revealed text for a typewriter.
pub(crate) fn advance(
    a: &mut Anim,
    value: f32,
    dt: f32,
    rest: &SpringRest,
    text: &str,
) -> (f32, bool) {
    match a.motion {
        Motion::Tween { duration, ease } => {
            a.elapsed += dt;
            if duration <= 0.0 || a.elapsed >= duration {
                return (a.to, true);
            }
            let p = ease.at(a.elapsed / duration);
            (a.start_value + (a.to - a.start_value) * p, false)
        }
        Motion::Spring { response, damping } => {
            let response = response.max(1e-3);
            let k = (std::f32::consts::TAU / response).powi(2);
            let c = 4.0 * std::f32::consts::PI * damping / response;
            let h = 1.0 / rest.hz;
            let steps = (dt / h).round().max(1.0) as u32;
            let mut x = value;
            for _ in 0..steps {
                let acc = -k * (x - a.to) - c * a.velocity;
                a.velocity += acc * h;
                x += a.velocity * h;
            }
            if (x - a.to).abs() < rest.delta && a.velocity.abs() < rest.speed {
                a.velocity = 0.0;
                return (a.to, true);
            }
            (x, false)
        }
        Motion::Typewriter { rate, pause } => {
            let mut left = dt;
            let mut x = value;
            let total = text.chars().count() as f32;
            while left > 0.0 && x < total {
                if a.hold > 0.0 {
                    let wait = a.hold.min(left);
                    a.hold -= wait;
                    left -= wait;
                    continue;
                }
                let next = x.floor() + 1.0;
                let need = (next - x) / rate;
                if need > left {
                    x += left * rate;
                    break;
                }
                left -= need;
                x = next;
                let just = text.chars().nth(next as usize - 1);
                if matches!(just, Some('.' | '!' | '?' | '…')) && x < total {
                    a.hold = pause;
                }
            }
            (x.min(total), x >= total)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_endpoints_and_linear() {
        for e in [Ease::OUT, Ease::IN_OUT, Ease::LINEAR] {
            assert_eq!(e.at(0.0), 0.0);
            assert_eq!(e.at(1.0), 1.0);
        }
        assert!((Ease::LINEAR.at(0.3) - 0.3).abs() < 1e-5);
        assert!((Ease::IN_OUT.at(0.5) - 0.5).abs() < 1e-4);
        assert!(Ease::OUT.at(0.2) > 0.5, "ease-out is fast at first");
    }
}
