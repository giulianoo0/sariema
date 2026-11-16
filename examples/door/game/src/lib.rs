// SPDX-License-Identifier: Apache-2.0

//! A house of two rooms with an automatic door between them. The door opens
//! when the hero comes near and shuts a moment later. The hero walks through
//! and stops just past it, inside the leaf's swing: the leaf stops on the
//! hero instead of pushing it, the engine says `hinge_blocked`, and the door
//! opens again and waits. Once the hero walks on, it shuts.

use sariema_sdk::{Command, Ctx, Ease, Entity, Event, Events, Game, Kind, Property, game, id};

const HERO_AT: [f32; 3] = [1.5, 0.0, -2.5];
const PAST_THE_DOOR: [f32; 3] = [4.45, 0.0, -2.3];
const FAR_WALL: [f32; 3] = [6.8, 0.0, -2.5];
const HERO_SPEED: f32 = 1.6;

/// The leaf's node is `<room>.<piece>_<hinge>`; its angle from closed is the
/// scene parameter `hinge.<node>`.
const LEAF: u32 = id("hinge.house.door_leaf");
/// Into the east room, away from a hero coming from the west.
const OPEN_DEG: f32 = 90.0;
const OPEN_S: f32 = 0.8;
const SHUT_S: f32 = 1.2;
/// How long the door stays open before it shuts, and waits after it stops
/// on someone.
const HOLD_S: f32 = 2.0;
const LOOK_AROUND_S: f32 = 2.0;

const NEAR_DOOR: u32 = 1;
const DOORWAY: [f32; 3] = [4.0, 0.0, -2.5];
const SWING: u32 = 1;
const SHUT: u32 = 1;
const WALK_ON: u32 = 2;

pub struct Door {
    hero: Entity,
    walked_on: bool,
}

fn swing(ctx: &mut Ctx<'_>, to: f32, duration: f32) {
    ctx.send(Command::Tween {
        tween: SWING,
        entity: Entity::NONE,
        property: Property::Scene,
        var: LEAF,
        to,
        duration,
        ease: Ease::InOutSine,
    });
}

fn open(ctx: &mut Ctx<'_>) {
    swing(ctx, OPEN_DEG, OPEN_S);
    ctx.send(Command::Timer {
        timer: SHUT,
        duration: HOLD_S,
        repeat: false,
    });
}

impl Game for Door {
    fn new(ctx: &mut Ctx<'_>) -> Door {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: HERO_AT,
            yaw: 90.0,
        });
        ctx.send(Command::Zone {
            zone: NEAR_DOOR,
            center: DOORWAY,
            radius: 1.2,
        });
        ctx.send(Command::MoveTo {
            entity: hero,
            target: PAST_THE_DOOR,
            speed: HERO_SPEED,
        });
        ctx.send(Command::HostReady {});
        Door {
            hero,
            walked_on: false,
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::TriggerEnter {
                    trigger: NEAR_DOOR, ..
                } => open(ctx),
                Event::TimerFired { timer: SHUT } => swing(ctx, 0.0, SHUT_S),
                Event::HingeBlocked { hinge: LEAF, .. } => open(ctx),
                Event::Arrived { entity } if entity == self.hero && !self.walked_on => {
                    ctx.send(Command::Timer {
                        timer: WALK_ON,
                        duration: LOOK_AROUND_S,
                        repeat: false,
                    })
                }
                Event::TimerFired { timer: WALK_ON } => {
                    self.walked_on = true;
                    ctx.send(Command::MoveTo {
                        entity: self.hero,
                        target: FAR_WALL,
                        speed: HERO_SPEED,
                    })
                }
                _ => {}
            }
        }
    }
}

game!(Door);

#[cfg(test)]
mod tests {
    use sariema_sdk::testing::{TestHost, commands};
    use sariema_sdk::{Command, Event, Runner};

    use super::*;

    fn started() -> (Runner<Door>, TestHost) {
        let mut host = TestHost::with_buttons(&[]);
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::new();
        runner.run(&mut host);
        host.take_commands();
        (runner, host)
    }

    fn swung_to(sent: &[u8]) -> Option<f32> {
        commands(sent).find_map(|c| match c {
            Command::Tween { var: LEAF, to, .. } => Some(to),
            _ => None,
        })
    }

    #[test]
    fn the_door_opens_when_the_hero_comes_near_and_shuts_after_a_while() {
        let (mut runner, mut host) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::TriggerEnter {
            trigger: NEAR_DOOR,
            entity: hero,
        });
        runner.run(&mut host);
        assert_eq!(swung_to(&host.take_commands()), Some(OPEN_DEG));
        host.event(Event::TimerFired { timer: SHUT });
        runner.run(&mut host);
        assert_eq!(swung_to(&host.take_commands()), Some(0.0));
    }

    #[test]
    fn a_door_that_stops_on_someone_opens_again() {
        let (mut runner, mut host) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::HingeBlocked {
            hinge: LEAF,
            entity: hero,
        });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert_eq!(swung_to(&sent), Some(OPEN_DEG));
        assert!(commands(&sent).any(|c| matches!(c, Command::Timer { timer: SHUT, .. })));
    }
}
