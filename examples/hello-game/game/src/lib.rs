// SPDX-License-Identifier: Apache-2.0

//! A hero walks the meadow with the move axis. Stepping near the friend
//! plays the `meet` cutscene once; the save remembers it, and later visits
//! get a single line instead.

use sariema_sdk::{Advance, Command, Ctx, Ease, Entity, Event, Events, Game, Kind, game, id};

const HERO_SPEED: f32 = 2.0;
const FRIEND_AT: [f32; 3] = [0.0, 0.0, -8.0];
const NEAR_FRIEND: u32 = 1;
const FOLLOW_HERO: u32 = 1;
const AGAIN_LINE: u32 = 1;
const MEET: u32 = id("meet");
const MET_KEY: &str = "met";

pub struct Hello {
    hero: Entity,
    friend: Entity,
    met: bool,
}

impl Game for Hello {
    fn new(ctx: &mut Ctx<'_>) -> Hello {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: [0.0, 0.0, 0.0],
            yaw: 0.0,
        });
        let friend = ctx.spawn(Kind::Actor, id("friend"));
        ctx.send(Command::SetTransform {
            entity: friend,
            position: FRIEND_AT,
            yaw: 180.0,
        });
        ctx.send(Command::SetHero {
            entity: hero,
            speed: HERO_SPEED,
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
        ctx.send(Command::Zone {
            zone: NEAR_FRIEND,
            center: FRIEND_AT,
            radius: 3.0,
        });
        ctx.send(Command::SaveLoad {});
        ctx.send(Command::HostReady {});
        Hello {
            hero,
            friend,
            met: false,
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::SaveLoaded {
                    key: MET_KEY,
                    value: [1],
                } => self.met = true,
                Event::TriggerEnter {
                    trigger: NEAR_FRIEND,
                    entity,
                } if entity == self.hero => self.greet(ctx),
                Event::CutsceneEnded { cutscene: MEET, .. } => {
                    self.met = true;
                    ctx.send(Command::Save {
                        key: MET_KEY,
                        value: &[1],
                    });
                }
                _ => {}
            }
        }
    }
}

impl Hello {
    fn greet(&mut self, ctx: &mut Ctx<'_>) {
        if self.met {
            ctx.send(Command::Say {
                line: AGAIN_LINE,
                speaker: self.friend,
                text: "Back again? The meadow is yours.",
                voice: 0,
                advance: Advance::Auto,
                duration: 2.0,
            });
            return;
        }
        ctx.bind(id("hero"), self.hero);
        ctx.bind(id("friend"), self.friend);
        ctx.play_cutscene(MEET);
    }
}

game!(Hello);

#[cfg(test)]
mod tests {
    use sariema_sdk::testing::{TestHost, commands};
    use sariema_sdk::{Command, Event, Runner};

    use super::*;

    fn started() -> (Runner<Hello>, TestHost) {
        let mut host = TestHost::new();
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::new();
        runner.run(&mut host);
        (runner, host)
    }

    #[test]
    fn the_first_tick_builds_the_meadow_and_asks_for_the_save() {
        let (_, mut host) = started();
        let sent: Vec<&str> = commands(&host.take_commands()).map(|c| c.name()).collect();
        assert_eq!(sent.first(), Some(&"spawn"));
        assert!(sent.contains(&"zone") && sent.contains(&"save_load"));
    }

    #[test]
    fn reaching_the_friend_plays_the_cutscene_once_then_a_line() {
        let (mut runner, mut host) = started();
        let hero = runner.game().unwrap().hero;
        let enter = Event::TriggerEnter {
            trigger: NEAR_FRIEND,
            entity: hero,
        };
        host.take_commands();
        host.event(enter);
        runner.run(&mut host);
        let first = host.take_commands();
        assert!(commands(&first).any(|c| c == Command::CutscenePlay { cutscene: MEET }));

        host.event(Event::CutsceneEnded {
            cutscene: MEET,
            skipped: false,
        });
        host.event(enter);
        runner.run(&mut host);
        let names: Vec<&str> = commands(&host.take_commands()).map(|c| c.name()).collect();
        assert_eq!(names, ["save", "say"]);
    }
}
