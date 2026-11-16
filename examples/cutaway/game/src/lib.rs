// SPDX-License-Identifier: Apache-2.0

//! A house of two rooms behind a wall of crates, seen from the south. The
//! hero walks from the back room through the doorway into the front one;
//! the room's cutaway keeps the crates between the camera and the hero
//! down to their first course, its line easing from one room's edge to the
//! other's.

use sariema_sdk::{Command, Controller, Ctx, Events, Game, Kind, game, id};

const START: [f32; 3] = [2.25, 0.0, -3.6];
const DOORWAY: [f32; 3] = [2.25, 0.0, -1.2];
const SPEED: f32 = 1.2;
const SETS_OFF_STEP: u32 = 60;

pub struct House {
    hero: sariema_sdk::Entity,
    step: u32,
}

impl Game for House {
    fn new(ctx: &mut Ctx<'_>) -> House {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: START,
            yaw: 180.0,
        });
        ctx.send(Command::SetHero {
            entity: hero,
            speed: SPEED,
        });
        ctx.send(Command::Control {
            by: Controller::Script,
        });
        House { hero, step: 0 }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, _: Events<'_>) {
        self.step += 1;
        if self.step == SETS_OFF_STEP {
            ctx.send(Command::MoveTo {
                entity: self.hero,
                target: DOORWAY,
                speed: SPEED,
            });
        }
    }
}

game!(House);
