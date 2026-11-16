// SPDX-License-Identifier: Apache-2.0

//! The hero walks from the door to the table and back. A trail of soft
//! amber dots leads to the desk while the hero walks there, a wave of light
//! running along it, and fades once the hero arrives. Motes drift in the
//! lamp's light along a line across the room; while the hero is within
//! reach the table glows faintly in its own colour and gives off a few
//! sparkles, both fading in and out; and the hero's feet raise a puff of
//! dust where each walk stops.

use sariema_sdk::{Command, Ctx, Ease, Entity, Event, Events, Game, Kind, PointList, game, id};

const START: [f32; 3] = [4.5, 0.0, -1.0];
const DESK: [f32; 3] = [1.5, 0.0, -0.5];
const TABLE_TOP: [f32; 3] = [1.5, 1.7, -1.5];
const TABLE_FOOT: [f32; 3] = [1.5, 0.0, -1.5];
const MOTES: [[f32; 3]; 3] = [[0.5, 1.6, -1.0], [2.5, 1.9, -2.5], [5.0, 1.5, -4.5]];
const HERO_SPEED: f32 = 1.2;
const REACH: f32 = 2.0;
const FOLLOW_HERO: u32 = 1;
const NEAR_TABLE: u32 = 1;
const SET_OFF: u32 = 1;
const GO_BACK: u32 = 2;
const MOTES_EMITTER: u32 = 1;
const SPARKLES: u32 = 2;
const FADE_S: f32 = 0.6;
const TO_DESK: u32 = 1;
const TABLE: &str = "a.mesa";
const AMBER: [f32; 3] = [1.0, 0.6, 0.2];
const GLOW_FILL: f32 = 0.12;
const GLOW_RIM: f32 = 0.35;
const GLOW_PULSE_S: f32 = 2.0;

pub struct Guide {
    hero: Entity,
    back: bool,
}

impl Game for Guide {
    fn new(ctx: &mut Ctx<'_>) -> Guide {
        #[cfg(target_arch = "wasm32")]
        std::panic::set_hook(Box::new(|info| sariema_sdk::report_panic(info)));
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetTransform {
            entity: hero,
            position: START,
            yaw: 90.0,
        });
        ctx.send(Command::RigFollow {
            rig: FOLLOW_HERO,
            subject: hero,
            distance: 6.5,
            pitch: 50.0,
            yaw: 0.0,
            height: 0.8,
            fov: 45.0,
        });
        ctx.send(Command::CameraUse {
            rig: FOLLOW_HERO,
            blend: 0.0,
            ease: Ease::Linear,
        });
        let seed = ctx.rng().next_u32();
        ctx.send(Command::FxStart {
            emitter: MOTES_EMITTER,
            effect: id("motes"),
            entity: Entity::NONE,
            position: [0.0; 3],
            points: PointList::Slice(&MOTES),
            seed,
        });
        ctx.send(Command::Zone {
            zone: NEAR_TABLE,
            center: TABLE_FOOT,
            radius: REACH,
        });
        ctx.send(Command::Timer {
            timer: SET_OFF,
            duration: 0.5,
            repeat: false,
        });
        ctx.send(Command::HostReady {});
        Guide { hero, back: false }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            match event {
                Event::TimerFired { timer: SET_OFF } => {
                    self.walk(ctx, DESK);
                    ctx.send(Command::Trail {
                        trail: TO_DESK,
                        from: self.hero,
                        points: PointList::Slice(&[DESK]),
                        spacing: 1.0,
                        size: 0.2,
                        color: AMBER,
                        intensity: 0.55,
                        wave_speed: 2.0,
                        fade_from: 0.6,
                        fade_to: 0.4,
                        fade: 0.4,
                    });
                }
                Event::TimerFired { timer: GO_BACK } => {
                    self.back = true;
                    self.walk(ctx, START);
                }
                Event::TriggerEnter {
                    trigger: NEAR_TABLE,
                    entity,
                } if entity == self.hero => {
                    let seed = ctx.rng().next_u32();
                    ctx.send(Command::FxStart {
                        emitter: SPARKLES,
                        effect: id("sparkle"),
                        entity: Entity::NONE,
                        position: TABLE_TOP,
                        points: PointList::EMPTY,
                        seed,
                    });
                    ctx.send(table_glow(GLOW_FILL, GLOW_RIM));
                }
                Event::TriggerExit {
                    trigger: NEAR_TABLE,
                    entity,
                } if entity == self.hero => {
                    ctx.send(Command::FxStop {
                        emitter: SPARKLES,
                        fade: FADE_S,
                    });
                    ctx.send(table_glow(0.0, 0.0));
                }
                Event::Arrived { entity } if entity == self.hero => {
                    let seed = ctx.rng().next_u32();
                    ctx.send(Command::FxBurst {
                        effect: id("dust"),
                        entity: self.hero,
                        position: [0.0, 0.05, 0.0],
                        count: 6,
                        seed,
                    });
                    if !self.back {
                        ctx.send(Command::TrailOff {
                            trail: TO_DESK,
                            fade: FADE_S,
                        });
                        ctx.send(Command::Timer {
                            timer: GO_BACK,
                            duration: 2.5,
                            repeat: false,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

impl Guide {
    fn walk(&mut self, ctx: &mut Ctx<'_>, to: [f32; 3]) {
        ctx.send(Command::MoveTo {
            entity: self.hero,
            target: to,
            speed: HERO_SPEED,
        });
    }
}

fn table_glow(fill: f32, rim: f32) -> Command<'static> {
    Command::Highlight {
        entity: Entity::NONE,
        node: id(TABLE),
        fill,
        rim,
        pulse: GLOW_PULSE_S,
        fade: FADE_S,
    }
}

game!(Guide);

#[cfg(test)]
mod tests {
    use sariema_sdk::testing::{TestHost, commands};
    use sariema_sdk::{Command, Event, Runner};

    use super::*;

    fn started() -> (Runner<Guide>, TestHost, Vec<u8>) {
        let mut host = TestHost::with_buttons(&[]);
        host.event(Event::Started { seed: 1 });
        let mut runner = Runner::new();
        runner.run(&mut host);
        let sent = host.take_commands();
        (runner, host, sent)
    }

    #[test]
    fn motes_drift_along_their_line_from_the_start() {
        let (_, _, sent) = started();
        assert!(commands(&sent).any(|c| matches!(c,
            Command::FxStart { emitter: MOTES_EMITTER, points, .. } if points.len() == MOTES.len())));
    }

    #[test]
    fn the_table_sparkles_only_while_the_hero_is_near() {
        let (mut runner, mut host, _) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::TriggerEnter {
            trigger: NEAR_TABLE,
            entity: hero,
        });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| matches!(
            c,
            Command::FxStart {
                emitter: SPARKLES,
                position: TABLE_TOP,
                ..
            }
        )));
        host.event(Event::TriggerExit {
            trigger: NEAR_TABLE,
            entity: hero,
        });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| c
            == Command::FxStop {
                emitter: SPARKLES,
                fade: FADE_S,
            }));
        assert!(commands(&sent).any(|c| c == table_glow(0.0, 0.0)));
    }

    #[test]
    fn the_table_glows_in_its_own_colour_while_the_hero_is_near() {
        let (mut runner, mut host, _) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::TriggerEnter {
            trigger: NEAR_TABLE,
            entity: hero,
        });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| c == table_glow(GLOW_FILL, GLOW_RIM)));
    }

    #[test]
    fn a_trail_leads_to_the_desk_and_fades_when_the_hero_arrives() {
        let (mut runner, mut host, _) = started();
        let hero = runner.game().unwrap().hero;
        host.event(Event::TimerFired { timer: SET_OFF });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| matches!(c,
            Command::Trail { trail: TO_DESK, from, points, .. } if from == hero && points.len() == 1)));
        host.event(Event::Arrived { entity: hero });
        runner.run(&mut host);
        let sent = host.take_commands();
        assert!(commands(&sent).any(|c| c
            == Command::TrailOff {
                trail: TO_DESK,
                fade: FADE_S,
            }));
    }
}
