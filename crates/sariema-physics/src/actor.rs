// SPDX-License-Identifier: Apache-2.0

//! What an actor is made of for collision: a vertical body capsule standing on
//! the actor's position (its feet), and an optional head sphere for
//! characters whose head is wider than their body.

use glam::Vec3;

use crate::math::forward;
use crate::narrow::Round;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorCollider {
    pub radius: f32,
    /// From the feet to the top of the body capsule; at least `2 × radius`.
    pub height: f32,
    pub head: Option<Head>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Head {
    pub radius: f32,
    /// Height of the sphere's centre above the feet.
    pub height: f32,
    /// How far the centre sits ahead of the body's axis, along the facing.
    pub forward: f32,
}

impl Default for ActorCollider {
    fn default() -> ActorCollider {
        ActorCollider {
            radius: 0.30,
            height: 1.0,
            head: None,
        }
    }
}

/// An axis-aligned box in the actor's own frame: feet at the origin, facing +Z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl ActorCollider {
    /// The smallest collider that covers the body's bounds in the ground plane
    /// and up to their top, with a head sphere around `head` when given.
    pub fn fit(body: LocalBounds, head: Option<LocalBounds>) -> ActorCollider {
        let radius = body
            .min
            .x
            .abs()
            .max(body.max.x.abs())
            .max(body.min.z.abs())
            .max(body.max.z.abs());
        ActorCollider {
            radius,
            height: body.max.y.max(2.0 * radius),
            head: head.map(|h| {
                let centre = (h.min + h.max) * 0.5;
                Head {
                    radius: ((h.max - h.min) * 0.5).max_element(),
                    height: centre.y,
                    forward: centre.z,
                }
            }),
        }
    }

    /// The collider's own bounds in the actor's frame.
    pub fn bounds(&self) -> LocalBounds {
        let r = self.radius;
        let mut b = LocalBounds {
            min: Vec3::new(-r, 0.0, -r),
            max: Vec3::new(r, self.height, r),
        };
        if let Some(h) = self.head {
            let c = Vec3::new(0.0, h.height, h.forward);
            b.min = b.min.min(c - Vec3::splat(h.radius));
            b.max = b.max.max(c + Vec3::splat(h.radius));
        }
        b
    }

    pub(crate) fn is_valid(&self) -> bool {
        let ok = |x: f32| x.is_finite() && x > 0.0;
        ok(self.radius)
            && self.height.is_finite()
            && self.height >= 2.0 * self.radius
            && self
                .head
                .is_none_or(|h| ok(h.radius) && h.height.is_finite() && h.forward.is_finite())
    }

    /// The parts at `feet`, facing `yaw`, with the body's bottom raised by
    /// `lift` (used to ignore ledges low enough to step over).
    pub(crate) fn probe(&self, feet: Vec3, yaw: f32, lift: f32) -> Probe {
        let r = self.radius;
        let bottom = (r + lift).min(self.height - r);
        let mut probe = Probe {
            rounds: [Round {
                a: feet + Vec3::new(0.0, bottom, 0.0),
                b: feet + Vec3::new(0.0, self.height - r, 0.0),
                r,
            }; 2],
            len: 1,
        };
        if let Some(h) = self.head {
            let c = feet + Vec3::new(0.0, h.height, 0.0) + forward(yaw) * h.forward;
            probe.rounds[1] = Round {
                a: c,
                b: c,
                r: h.radius,
            };
            probe.len = 2;
        }
        probe
    }
}

/// An actor's collision parts placed in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Probe {
    rounds: [Round; 2],
    len: usize,
}

impl Probe {
    pub fn single(round: Round) -> Probe {
        Probe {
            rounds: [round; 2],
            len: 1,
        }
    }

    /// The same parts with radii grown by `by` (shrunk when negative).
    pub fn inflated(mut self, by: f32) -> Probe {
        for r in &mut self.rounds {
            r.r = (r.r + by).max(0.0);
        }
        self
    }

    pub fn rounds(&self) -> &[Round] {
        &self.rounds[..self.len]
    }
}

/// How an actor is added to a [`crate::World`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorDesc {
    /// The feet.
    pub position: Vec3,
    pub yaw: f32,
    pub collider: ActorCollider,
    /// Speed at full stick and on scripted moves.
    pub walk_speed: f32,
    /// Share of the push when two actors overlap: each moves by the other's
    /// mass over the sum. `f32::INFINITY` is never pushed.
    pub mass: f32,
}

impl Default for ActorDesc {
    fn default() -> ActorDesc {
        ActorDesc {
            position: Vec3::ZERO,
            yaw: 0.0,
            collider: ActorCollider::default(),
            walk_speed: 2.1,
            mass: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_covers_the_body_and_the_head() {
        let c = ActorCollider::fit(
            LocalBounds {
                min: Vec3::new(-0.25, 0.0, -0.2),
                max: Vec3::new(0.28, 0.9, 0.22),
            },
            Some(LocalBounds {
                min: Vec3::new(-0.35, 0.8, -0.3),
                max: Vec3::new(0.35, 1.5, 0.4),
            }),
        );
        assert_eq!(c.radius, 0.28);
        assert_eq!(c.height, 0.9);
        let h = c.head.unwrap();
        assert!(
            (h.radius - 0.35).abs() < 1e-6
                && (h.height - 1.15).abs() < 1e-6
                && (h.forward - 0.05).abs() < 1e-6
        );
        assert!(c.is_valid());
    }
}
