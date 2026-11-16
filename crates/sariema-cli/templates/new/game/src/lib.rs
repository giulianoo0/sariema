// SPDX-License-Identifier: Apache-2.0

//! The intro cutscene walks the hero in and says a line over the music;
//! then the hero walks with the move axis, with collision, the objective
//! comes in, and the action button waves, ringing a chime at the wave's
//! `tap` marker.

use sariema_sdk::{Bus, ClipMode, Command, Ctx, Ease, Entity, Event, Events, Game, Kind, game, id};

const HERO_SPEED: f32 = 1.5;
const HERO_AT: [f32; 3] = [4.5, 0.0, -1.0];
const FOLLOW_HERO: u32 = 1;
/// Index of `action` in the project's input buttons.
const ACTION: u16 = 3;
const INTRO: u32 = id("intro");
const WAVE: u32 = id("wave");

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
            position: HERO_AT,
            yaw: 0.0,
        });
        ctx.send(Command::RigFollow {
            rig: FOLLOW_HERO,
            subject: hero,
            distance: 6.0,
            pitch: 40.0,
            yaw: 0.0,
            height: 1.0,
            fov: 45.0,
        });
        ctx.send(Command::CameraUse {
            rig: FOLLOW_HERO,
            blend: 0.0,
            ease: Ease::Linear,
        });
        ctx.send(Command::PlayMusic {
            music: id("theme"),
            fade: 1.0,
        });
        ctx.bind(id("hero"), hero);
        ctx.play_cutscene(INTRO);
        ctx.send(Command::HostReady {});
        MyGame { hero }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::CutsceneEnded {
                    cutscene: INTRO, ..
                } => {
                    ctx.send(Command::SetHero {
                        entity: self.hero,
                        speed: HERO_SPEED,
                    });
                    ctx.send(Command::UiPlay {
                        animation: id("enter"),
                    });
                }
                Event::Button {
                    action: ACTION,
                    down: true,
                } => ctx.send(Command::PlayClip {
                    entity: self.hero,
                    clip: WAVE,
                    mode: ClipMode::Once,
                    blend: 0.15,
                    speed: 1.0,
                }),
                Event::ClipMarker { entity, clip: WAVE, .. } if entity == self.hero => {
                    ctx.send(Command::PlaySound {
                        sound: id("chime"),
                        bus: Bus::Effects,
                        emitter: self.hero,
                        volume: 0.8,
                    });
                }
                _ => {}
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

    #[test]
    fn the_end_of_the_intro_hands_the_hero_to_the_player_and_brings_the_objective_in() {
        let mut host = TestHost::new();
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::<MyGame>::new();
        runner.run(&mut host);
        host.take_commands();
        host.event(Event::CutsceneEnded {
            cutscene: INTRO,
            skipped: false,
        });
        runner.run(&mut host);
        let names: Vec<&str> = commands(&host.take_commands()).map(|c| c.name()).collect();
        assert_eq!(names, ["set_hero", "ui_play"]);
    }
}
