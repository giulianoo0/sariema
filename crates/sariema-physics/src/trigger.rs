// SPDX-License-Identifier: Apache-2.0

//! Named volumes that report actors entering and leaving.

use crate::actor::Probe;
use crate::math::Bounds;
use crate::narrow::{Part, contact};
use crate::shape::{Plane, Shape, ShapeError, inside, parts_of};

pub(crate) struct Trigger {
    pub name: String,
    parts: Vec<Part>,
    planes: Vec<Plane>,
    bounds: Bounds,
}

impl Trigger {
    pub fn new(name: &str, shape: &Shape) -> Result<Trigger, ShapeError> {
        let p = parts_of(shape)?;
        let bounds = p.parts.iter().fold(Bounds::EMPTY, |b, part| {
            b.union(match part {
                Part::Tri(t) => Bounds::of_points([t.a, t.b, t.c]),
                Part::Round(r) => Bounds::of_points([r.a, r.b]).grown(r.r),
            })
        });
        Ok(Trigger {
            name: name.to_owned(),
            parts: p.parts,
            planes: p.planes,
            bounds,
        })
    }

    pub fn overlaps(&self, probe: &Probe) -> bool {
        probe.rounds().iter().any(|r| {
            Bounds::of_points([r.a, r.b])
                .grown(r.r)
                .overlaps(&self.bounds)
                && (self
                    .parts
                    .iter()
                    .any(|p| contact(r.a, r.b, r.r, p, 1.0).separation < 0.0)
                    || inside(&self.planes, r.a, r.b, r.r).is_some())
        })
    }
}
