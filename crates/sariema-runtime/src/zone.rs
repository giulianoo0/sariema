// SPDX-License-Identifier: Apache-2.0

//! Built-in trigger spheres: report actors entering and leaving, once per
//! crossing. A collision module's named triggers report the same events.

use glam::Vec3;
use sariema_format::script::Event;

use crate::world::{Entity, Kind, World};

#[derive(Clone, Debug)]
struct Zone {
    id: u32,
    center: Vec3,
    radius: f32,
    /// Sorted.
    inside: Vec<Entity>,
}

#[derive(Clone, Debug, Default)]
pub struct Zones {
    list: Vec<Zone>,
    now: Vec<Entity>,
}

impl Zones {
    /// Moving or resizing a zone keeps who is inside until the next step.
    pub fn set(&mut self, id: u32, center: Vec3, radius: f32) {
        match self.list.binary_search_by_key(&id, |z| z.id) {
            Ok(i) => {
                self.list[i].center = center;
                self.list[i].radius = radius;
            }
            Err(i) => self.list.insert(
                i,
                Zone {
                    id,
                    center,
                    radius,
                    inside: Vec::new(),
                },
            ),
        }
    }

    pub fn remove(&mut self, id: u32) {
        self.list.retain(|z| z.id != id);
    }

    pub fn step<'a>(&mut self, world: &World, mut emit: impl FnMut(Event<'a>)) {
        for zone in &mut self.list {
            self.now.clear();
            let r2 = zone.radius * zone.radius;
            self.now.extend(
                world
                    .entities()
                    .filter(|(_, n)| n.kind == Kind::Actor)
                    .filter(|(e, _)| {
                        world
                            .pose(*e)
                            .is_some_and(|p| p.position.distance_squared(zone.center) <= r2)
                    })
                    .map(|(e, _)| e),
            );
            for &e in &self.now {
                if zone.inside.binary_search(&e).is_err() {
                    emit(Event::TriggerEnter {
                        trigger: zone.id,
                        entity: e,
                    });
                }
            }
            for &e in &zone.inside {
                if self.now.binary_search(&e).is_err() {
                    emit(Event::TriggerExit {
                        trigger: zone.id,
                        entity: e,
                    });
                }
            }
            std::mem::swap(&mut zone.inside, &mut self.now);
        }
    }
}
