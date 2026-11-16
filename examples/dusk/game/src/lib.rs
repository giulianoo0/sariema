// SPDX-License-Identifier: Apache-2.0

//! Dusk on the porch. The game tweens the sun, the hemisphere and the look
//! together; once they arrive it plays `nightfall`, whose `scene` step
//! brings the fog in and the lamps up while the night settles.

use sariema_sdk::{Ctx, Ease, Event, Events, Game, game, id};

const DUSK: u32 = 1;
const NIGHTFALL: u32 = id("nightfall");

pub struct Dusk;

impl Game for Dusk {
    fn new(ctx: &mut Ctx<'_>) -> Dusk {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let dusk = [
            (id("sun.intensity"), 0.25),
            (id("sun.elevation"), 12.0),
            (id("sun.color.g"), 0.45),
            (id("sun.color.b"), 0.25),
            (id("hemisphere.intensity"), 0.5),
            (id("background.top.b"), 0.3),
            (id("background.bottom.g"), 0.35),
            (id("background.bottom.b"), 0.3),
            (id("look.exposure"), 1.2),
        ];
        ctx.scene_tween(DUSK, &dusk, 3.0, Ease::InOutSine);
        Dusk
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            if let Event::TweenDone { tween: DUSK } = event {
                ctx.play_cutscene(NIGHTFALL);
            }
        }
    }
}

game!(Dusk);
