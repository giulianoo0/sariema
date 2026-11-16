// SPDX-License-Identifier: Apache-2.0

//! The hero walks east from the porch into the hall next to it, the camera
//! following: the hall, loaded in the background as the porch's
//! neighbour, takes over without a seam as the hero crosses the doorway.
//! Once there, the hall dissolves into the attic, which stands in the same
//! place: a room that does not continue the last one.

use sariema_sdk::{Command, Ctx, Ease, Entity, Event, Events, Game, Kind, RoomTransition, game, id};

const HERO_AT: [f32; 3] = [1.0, 0.0, -3.0];
const HALL_SPOT: [f32; 3] = [9.5, 0.0, -3.0];
const HERO_SPEED: f32 = 1.6;
const DOORWAY: u32 = 1;
const DOORWAY_AT: [f32; 3] = [6.0, 0.0, -3.0];
const DREAM: u32 = 2;
const FOLLOW_HERO: u32 = 1;
/// Seconds the camera takes to blend the two rooms' light as the hero
/// crosses.
const CROSSING_S: f32 = 0.8;
const DISSOLVE_S: f32 = 0.5;

pub struct Rooms {
    hero: Entity,
}

impl Game for Rooms {
    fn new(ctx: &mut Ctx<'_>) -> Rooms {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: HERO_AT,
            yaw: -90.0,
        });
        ctx.send(Command::RigFollow {
            rig: FOLLOW_HERO,
            subject: hero,
            distance: 7.0,
            pitch: 45.0,
            yaw: 0.0,
            height: 1.0,
            fov: 45.0,
        });
        ctx.send(Command::CameraUse {
            rig: FOLLOW_HERO,
            blend: 0.0,
            ease: Ease::Linear,
        });
        ctx.send(Command::Zone {
            zone: DOORWAY,
            center: DOORWAY_AT,
            radius: 0.5,
        });
        ctx.send(Command::MoveTo {
            entity: hero,
            target: HALL_SPOT,
            speed: HERO_SPEED,
        });
        ctx.send(Command::HostReady {});
        Rooms { hero }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::TriggerEnter {
                    trigger: DOORWAY,
                    entity,
                } if entity == self.hero => ctx.send(Command::RoomSwitch {
                    room: id("hall"),
                    transition: RoomTransition::Continuous,
                    duration: CROSSING_S,
                }),
                Event::Arrived { entity } if entity == self.hero => ctx.send(Command::Timer {
                    timer: DREAM,
                    duration: 1.0,
                    repeat: false,
                }),
                Event::TimerFired { timer: DREAM } => ctx.send(Command::RoomSwitch {
                    room: id("attic"),
                    transition: RoomTransition::Dissolve,
                    duration: DISSOLVE_S,
                }),
                _ => {}
            }
        }
    }
}

game!(Rooms);

#[cfg(test)]
mod tests {
    use sariema_sdk::testing::{TestHost, commands};
    use sariema_sdk::{Command, Event, Runner};

    use super::*;

    fn started() -> (Runner<Rooms>, TestHost) {
        let mut host = TestHost::with_buttons(&[]);
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::new();
        runner.run(&mut host);
        host.take_commands();
        (runner, host)
    }

    #[test]
    fn crossing_the_doorway_switches_to_the_hall_continuously() {
        let (mut runner, mut host) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::TriggerEnter {
            trigger: DOORWAY,
            entity: hero,
        });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| c
            == Command::RoomSwitch {
                room: id("hall"),
                transition: RoomTransition::Continuous,
                duration: CROSSING_S,
            }));
    }
}
