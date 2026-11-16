// SPDX-License-Identifier: Apache-2.0

//! The intro cutscene plays, then the hero walks with the move axis.

use sariema_sdk::{Command, Ctx, Ease, Entity, Event, Events, Game, Kind, game, id};

const HERO_SPEED: f32 = 2.0;
const FOLLOW_HERO: u32 = 1;
const INTRO: u32 = id("intro");

pub struct MyGame {
    hero: Entity,
}

impl Game for MyGame {
    fn new(ctx: &mut Ctx<'_>) -> MyGame {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: [0.0, 0.0, 0.0],
            yaw: 0.0,
        });
        ctx.send(Command::RigFollow {
            rig: FOLLOW_HERO,
            subject: hero,
            distance: 6.0,
            pitch: 35.0,
            yaw: 0.0,
            height: 1.0,
            fov: 45.0,
        });
        ctx.send(Command::CameraUse {
            rig: FOLLOW_HERO,
            blend: 0.0,
            ease: Ease::Linear,
        });
        ctx.bind(id("hero"), hero);
        ctx.play_cutscene(INTRO);
        ctx.send(Command::HostReady {});
        MyGame { hero }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            if let Event::CutsceneEnded {
                cutscene: INTRO, ..
            } = event
            {
                ctx.send(Command::SetHero {
                    entity: self.hero,
                    speed: HERO_SPEED,
                });
            }
        }
    }
}

game!(MyGame);

#[cfg(test)]
mod tests {
    use sariema_sdk::testing::{TestHost, commands};
    use sariema_sdk::{Event, Runner};

    use super::*;

    #[test]
    fn the_first_tick_spawns_the_hero_and_plays_the_intro() {
        let mut host = TestHost::new();
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::<MyGame>::new();
        runner.run(&mut host);
        let sent: Vec<Command> = commands(&host.commands).collect();
        assert!(matches!(sent[0], Command::Spawn { kind: Kind::Actor, .. }));
        assert!(sent.contains(&Command::CutscenePlay { cutscene: INTRO }));
    }
}
