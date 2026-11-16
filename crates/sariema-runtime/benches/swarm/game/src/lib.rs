// SPDX-License-Identifier: Apache-2.0

//! The M1 load case: 2,000 wanderers, each deciding every step whether to
//! rest, pick a new goal or steer toward it. Placements go out as one
//! `set_transform` command each, or, after a `mode` host message with
//! payload 1, through a bulk transforms region.

use sariema_sdk::{Command, Ctx, Entity, Event, Events, Game, Kind, Transforms, game};

const COUNT: usize = 2_000;
const STEP: f32 = 1.0 / 60.0;
const ARENA: f32 = 40.0;
const SPEED: f32 = 1.5;

struct Wanderer {
    entity: Entity,
    at: [f32; 2],
    goal: [f32; 2],
    yaw: f32,
    rest: u32,
}

pub struct Swarm {
    wanderers: Vec<Wanderer>,
    region: Option<Transforms>,
}

impl Game for Swarm {
    fn new(ctx: &mut Ctx<'_>) -> Swarm {
        let mut wanderers = Vec::with_capacity(COUNT);
        for _ in 0..COUNT {
            let entity = ctx.spawn(Kind::Empty, 0);
            let at = [
                ctx.rng().range(-ARENA, ARENA),
                ctx.rng().range(-ARENA, ARENA),
            ];
            wanderers.push(Wanderer {
                entity,
                at,
                goal: at,
                yaw: 0.0,
                rest: 0,
            });
        }
        Swarm {
            wanderers,
            region: None,
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            if let Event::HostMessage {
                topic: "mode",
                payload: [1],
            } = event
            {
                let region = Transforms::with_capacity(COUNT);
                ctx.register(Transforms::KIND, region.as_bytes());
                self.region = Some(region);
            }
        }
        if let Some(region) = &mut self.region {
            region.clear();
        }
        for w in &mut self.wanderers {
            decide(w, ctx);
            let position = [w.at[0], 0.0, w.at[1]];
            match &mut self.region {
                Some(region) => region.push(w.entity, position, w.yaw),
                None => ctx.send(Command::SetTransform {
                    entity: w.entity,
                    position,
                    yaw: w.yaw,
                }),
            }
        }
    }
}

fn decide(w: &mut Wanderer, ctx: &mut Ctx<'_>) {
    if w.rest > 0 {
        w.rest -= 1;
        return;
    }
    let (dx, dz) = (w.goal[0] - w.at[0], w.goal[1] - w.at[1]);
    let distance = (dx * dx + dz * dz).sqrt();
    if distance < SPEED * STEP {
        w.at = w.goal;
        let rng = ctx.rng();
        w.goal = [rng.range(-ARENA, ARENA), rng.range(-ARENA, ARENA)];
        w.rest = 30 + (rng.next_u32() % 90);
        return;
    }
    let k = SPEED * STEP / distance;
    w.at = [w.at[0] + dx * k, w.at[1] + dz * k];
    w.yaw = (-dx).atan2(-dz).to_degrees();
}

game!(Swarm);
