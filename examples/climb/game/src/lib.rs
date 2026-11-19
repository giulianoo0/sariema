// SPDX-License-Identifier: Apache-2.0

//! The hero walks the room under the move axis, with collision. Moving
//! brings the objective in; the action button waves, and the wave's `tap`
//! marker rings a chime at the hero. Reaching the spot by the sill plays
//! `climb`: the hero climbs through the window and says a line over the
//! music past it, and the screen dips to black to a wide shot of the hero
//! where the climb left it.

use sariema_sdk::{Bus, ClipMode, Command, Ctx, Ease, Entity, Event, Events, Game, Kind, game, id};

const HERO_SPEED: f32 = 1.5;
const HERO_AT: [f32; 3] = [1.5, 0.0, -0.6];
const SILL_SPOT: [f32; 3] = [4.5, 0.0, -1.0];
const SILL_ZONE: u32 = 1;
const FOLLOW_HERO: u32 = 1;
const CLIMB: u32 = id("climb");
const WAVE: u32 = id("wave");
const TRAVERSAL_S: f32 = 1.6;

pub struct Climb {
    hero: Entity,
    action: Option<u16>,
    shown: bool,
    climbed: bool,
}

impl Game for Climb {
    fn new(ctx: &mut Ctx<'_>) -> Climb {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: HERO_AT,
            yaw: 0.0,
        });
        ctx.send(Command::SetHero {
            entity: hero,
            speed: HERO_SPEED,
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
        ctx.send(Command::Zone {
            zone: SILL_ZONE,
            center: SILL_SPOT,
            radius: 0.7,
        });
        ctx.send(Command::PlayMusic {
            music: id("theme"),
            fade: 1.0,
        });
        ctx.send(Command::HostReady {});
        Climb {
            hero,
            action: ctx.button(id("action")),
            shown: false,
            climbed: false,
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::Axis { x, y, .. } if !self.shown && (x != 0.0 || y != 0.0) => {
                    self.shown = true;
                    ctx.send(Command::UiPlay {
                        animation: id("enter"),
                    });
                }
                Event::Button { action, down: true } if Some(action) == self.action => {
                    ctx.send(Command::PlayClip {
                        entity: self.hero,
                        clip: WAVE,
                        mode: ClipMode::Once,
                        blend: 0.15,
                        speed: 1.0,
                    })
                }
                Event::ClipMarker {
                    entity, clip: WAVE, ..
                } if entity == self.hero => {
                    ctx.send(Command::PlaySound {
                        sound: id("chime"),
                        bus: Bus::Effects,
                        emitter: self.hero,
                        volume: 0.8,
                    });
                }
                Event::TriggerEnter {
                    trigger: SILL_ZONE,
                    entity,
                } if entity == self.hero && !self.climbed => {
                    ctx.bind(id("hero"), self.hero);
                    ctx.play_cutscene(CLIMB);
                }
                Event::CutsceneCall {
                    cutscene: CLIMB,
                    call: CLIMB,
                    ..
                } => ctx.send(Command::Traverse {
                    entity: self.hero,
                    traversal: id("window"),
                    duration: TRAVERSAL_S,
                }),
                Event::CutsceneEnded {
                    cutscene: CLIMB, ..
                } => {
                    self.climbed = true;
                    ctx.send(Command::UiPlay {
                        animation: id("done"),
                    });
                }
                _ => {}
            }
        }
    }
}

game!(Climb);

#[cfg(test)]
mod tests {
    use sariema_sdk::testing::{TestHost, commands};
    use sariema_sdk::{Command, Event, Runner};

    use super::*;

    fn started() -> (Runner<Climb>, TestHost) {
        let mut host = TestHost::with_buttons(&["confirm", "action"]);
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::new();
        runner.run(&mut host);
        host.take_commands();
        (runner, host)
    }

    #[test]
    fn the_cutscene_call_starts_the_traversal_through_the_window() {
        let (mut runner, mut host) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::CutsceneCall {
            cutscene: CLIMB,
            call: CLIMB,
            skipped: false,
        });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| c
            == Command::Traverse {
                entity: hero,
                traversal: id("window"),
                duration: TRAVERSAL_S,
            }));
    }

    #[test]
    fn the_action_button_waves_wherever_the_project_puts_it() {
        let (mut runner, mut host) = started();
        host.event(Event::Button {
            action: 1,
            down: true,
        });
        runner.run(&mut host);
        let names: Vec<&str> = commands(&host.take_commands()).map(|c| c.name()).collect();
        assert_eq!(names, ["play_clip"]);
    }

    #[test]
    fn the_wave_marker_rings_a_chime_at_the_hero() {
        let (mut runner, mut host) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::ClipMarker {
            entity: hero,
            clip: WAVE,
            marker: id("tap"),
        });
        runner.run(&mut host);
        let names: Vec<&str> = commands(&host.take_commands()).map(|c| c.name()).collect();
        assert_eq!(names, ["play_sound"]);
    }
}
