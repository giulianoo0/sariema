// SPDX-License-Identifier: Apache-2.0

//! The runtime's no-jump property with the collision module moving the
//! actors: seeded random programs of commands and transform region records,
//! despawns that free slots for new actors among them, speed scales and a
//! changing hero. After every step no visible actor is drawn sliding
//! farther than `motion.max_visible_speed` allows in a step or turning
//! faster than `motion.turn_rate`, nor drawn starting anywhere but where the
//! last step left it, unless the step was covered or cut, or a forced
//! teleport moved it or a carrier it kept through the step.

mod common;

use glam::{Quat, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_engine::Engine;
use sariema_format::script::{Command, Ease, Entity, Kind, Property, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::motion::Goal;
use sariema_runtime::rng::Pcg32;
use sariema_runtime::sim::{Logic, REGION_TRANSFORMS, Stop};
use sariema_runtime::world::Pose;

use common::{engine, send, wired};

const SLOTS: u32 = 6;
const PROGRAMS: u64 = 60;
const STEPS: usize = 300;

#[derive(Clone, Copy, Debug)]
struct Seen {
    alive: bool,
    generation: u32,
    actor: bool,
    visible: bool,
    parent: Entity,
    end: Pose,
    start: Pose,
}

const NOWHERE: Pose = Pose {
    position: Vec3::ZERO,
    rotation: Quat::IDENTITY,
    scale: 1.0,
};

const DEAD: Seen = Seen {
    alive: false,
    generation: 0,
    actor: false,
    visible: false,
    parent: Entity::NONE,
    end: NOWHERE,
    start: NOWHERE,
};

fn pick<T: Copy>(rng: &mut Pcg32, from: &[T]) -> T {
    from[rng.next_u32() as usize % from.len()]
}

/// A point on the room's floor, either half of it.
fn floor(rng: &mut Pcg32) -> [f32; 3] {
    [rng.range(0.3, 5.7), 0.0, rng.range(-5.7, -0.3)]
}

fn value(rng: &mut Pcg32) -> (Property, f32) {
    use Property as P;
    let property = pick(rng, &[P::PositionX, P::PositionZ, P::Yaw, P::Scale]);
    let to = match property {
        P::PositionX => rng.range(0.3, 5.7),
        P::PositionZ => rng.range(-5.7, -0.3),
        P::Scale => rng.range(0.5, 1.5),
        _ => rng.range(-360.0, 360.0),
    };
    (property, to)
}

#[derive(Clone, Debug, Default)]
struct Step {
    commands: Vec<Command<'static>>,
    region: Vec<(Entity, [f32; 3], f32)>,
}

struct Played {
    steps: Vec<Step>,
    step: usize,
    region: Vec<u8>,
}

impl Logic for Played {
    fn tick(&mut self, _: &[u8], _: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        commands.clear();
        self.region.clear();
        if let Some(step) = self.steps.get(self.step) {
            step.commands.iter().for_each(|c| send(commands, *c));
            self.region
                .extend_from_slice(&(step.region.len() as u32).to_le_bytes());
            for &(e, position, yaw) in &step.region {
                for v in [e.index, e.generation] {
                    self.region.extend_from_slice(&v.to_le_bytes());
                }
                for v in [position[0], position[1], position[2], yaw] {
                    self.region.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        self.step += 1;
        Ok(())
    }

    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        each(REGION_TRANSFORMS, &self.region);
    }
}

/// Slots in use and the generation each is at.
struct Slots {
    generation: [u32; SLOTS as usize],
    alive: [bool; SLOTS as usize],
}

impl Slots {
    fn entity(&self, i: u32) -> Entity {
        Entity {
            index: i,
            generation: self.generation[i as usize],
        }
    }
}

fn spawn(slots: &mut Slots, i: u32, kind: Kind, at: [f32; 3], out: &mut Vec<Command<'static>>) {
    let e = slots.entity(i);
    slots.alive[i as usize] = true;
    out.push(Command::Spawn {
        entity: e,
        kind,
        asset: if kind == Kind::Actor {
            name_id("hero")
        } else {
            0
        },
    });
    out.push(Command::SetTransform {
        entity: e,
        position: at,
        yaw: 0.0,
    });
}

fn command(rng: &mut Pcg32, slots: &mut Slots, out: &mut Step) {
    let region = &mut out.region;
    let out = &mut out.commands;
    let i = rng.next_u32() % SLOTS;
    let e = slots.entity(i);
    let other = slots.entity(rng.next_u32() % SLOTS);
    let at = floor(rng);
    let yaw = rng.range(-180.0, 180.0);
    let coin = rng.next_u32().is_multiple_of(2);
    let ease = pick(rng, &[Ease::Linear, Ease::InOutCubic, Ease::OutBack]);
    match rng.next_u32() % 19 {
        0 if !slots.alive[i as usize] => spawn(
            slots,
            i,
            if coin { Kind::Actor } else { Kind::Empty },
            at,
            out,
        ),
        0 | 15 if slots.alive[i as usize] => {
            out.push(Command::Despawn { entity: e });
            slots.alive[i as usize] = false;
            slots.generation[i as usize] += 1;
        }
        1 => out.push(Command::SetParent {
            entity: e,
            parent: if coin { other } else { Entity::NONE },
        }),
        2 => out.push(Command::SetTransform {
            entity: e,
            position: at,
            yaw,
        }),
        3 | 4 => {
            let (property, to) = value(rng);
            out.push(Command::Tween {
                tween: rng.next_u32() % 12,
                entity: e,
                property,
                var: 0,
                to,
                duration: pick(rng, &[0.0, STEP_SECONDS, 0.1, 0.5, 2.0]),
                ease,
            });
        }
        5 => {
            let (property, to) = value(rng);
            out.push(Command::Spring {
                tween: rng.next_u32() % 12,
                entity: e,
                property,
                var: 0,
                to,
                frequency: rng.range(0.2, 40.0),
                damping: rng.range(0.2, 1.5),
            });
        }
        6 => out.push(Command::Follow {
            entity: e,
            target: other,
            offset: [rng.range(-1.0, 1.0), 0.0, rng.range(-1.0, 1.0)],
            lag: pick(rng, &[0.0, 0.05, 0.3]),
        }),
        7 => out.push(Command::SetVisible {
            entity: e,
            visible: coin,
        }),
        8 => out.push(Command::Teleport {
            entity: e,
            position: at,
            yaw,
            force: rng.next_u32().is_multiple_of(2),
        }),
        9 => out.push(Command::Fade {
            to: if coin { 1.0 } else { 0.0 },
            duration: pick(rng, &[0.0, 0.1, 0.4]),
            color: [0.0; 3],
        }),
        10 => {
            out.push(Command::RigFixed {
                rig: 1,
                position: [3.0, 6.0, 4.0],
                look_at: at,
                fov: 50.0,
            });
            out.push(Command::CameraUse {
                rig: 1,
                blend: if coin { 0.0 } else { 0.3 },
                ease,
            });
        }
        11 => out.push(Command::MoveTo {
            entity: e,
            target: at,
            speed: rng.range(0.5, 30.0),
        }),
        12 => out.push(Command::MoveIntent {
            entity: e,
            velocity: [rng.range(-6.0, 6.0), 0.0, rng.range(-6.0, 6.0)],
        }),
        13 => out.push(Command::SetSpeedScale {
            entity: e,
            scale: pick(rng, &[0.0, 0.25, 1.0, 2.5, 20.0]),
        }),
        14 => out.push(Command::SetHero {
            entity: e,
            speed: rng.range(0.5, 4.0),
        }),
        16 => out.push(Command::Knockback {
            entity: e,
            impulse: [rng.range(-8.0, 8.0), 0.0, rng.range(-8.0, 8.0)],
        }),
        17 => out.push(Command::SetScale {
            entity: e,
            scale: pick(rng, &[0.5, 1.0, 1.5]),
        }),
        18 => region.push((e, at, yaw)),
        _ => {}
    }
}

fn program(seed: u64) -> Vec<Step> {
    let mut rng = Pcg32::new(seed, 0x6b);
    let mut slots = Slots {
        generation: [0; SLOTS as usize],
        alive: [false; SLOTS as usize],
    };
    let mut first = Step::default();
    spawn(
        &mut slots,
        0,
        Kind::Actor,
        [0.75, 0.0, -0.4],
        &mut first.commands,
    );
    spawn(
        &mut slots,
        1,
        Kind::Actor,
        [4.5, 0.0, -1.0],
        &mut first.commands,
    );
    spawn(
        &mut slots,
        2,
        Kind::Empty,
        [3.0, 0.0, -1.0],
        &mut first.commands,
    );
    first.commands.push(Command::SetHero {
        entity: slots.entity(0),
        speed: 1.5,
    });
    let mut steps = vec![first];
    for _ in 1..STEPS {
        let mut out = Step::default();
        for _ in 0..rng.next_u32() % 4 {
            command(&mut rng, &mut slots, &mut out);
        }
        steps.push(out);
    }
    steps
}

fn snapshot<L: Logic>(engine: &Engine<L>) -> Vec<Seen> {
    let world = &engine.state().world;
    let mut seen = vec![DEAD; SLOTS as usize];
    for (e, n) in world.entities() {
        seen[e.index as usize] = Seen {
            alive: true,
            generation: e.generation,
            actor: n.kind == Kind::Actor,
            visible: n.visible,
            parent: n.parent,
            end: world.pose(e).unwrap(),
            start: world.interpolated(e, 0.0).unwrap(),
        };
    }
    seen
}

fn degrees(a: Quat, b: Quat) -> f32 {
    let d = a.conjugate() * b;
    (2.0 * d.xyz().length().atan2(d.w.abs())).to_degrees()
}

fn forced_along(i: usize, before: &[Seen], now: &[Seen], forced: &[u32]) -> bool {
    let mut at = i;
    loop {
        if forced.contains(&(at as u32)) {
            return true;
        }
        let parent = now[at].parent;
        if parent != before[at].parent || parent.is_none() {
            return false;
        }
        at = parent.index as usize;
    }
}

fn check(
    before: &[Seen],
    now: &[Seen],
    exempt: bool,
    forced: &[u32],
    limits: (f32, f32),
) -> Result<(), String> {
    for (i, s) in now.iter().enumerate() {
        if !(s.alive && s.actor && s.visible) {
            continue;
        }
        for p in [s.start, s.end] {
            if !(p.position.is_finite() && p.rotation.is_finite()) {
                return Err(format!("actor {i} at {p:?}"));
            }
        }
        let (slide, spin) = (
            s.start.position.distance(s.end.position),
            degrees(s.start.rotation, s.end.rotation),
        );
        if slide > limits.0 || spin > limits.1 {
            return Err(format!(
                "actor {i} drawn sliding {slide} m and turning {spin}° in one step"
            ));
        }
        let b = before[i];
        let same = b.alive && b.generation == s.generation;
        if same && b.visible && !exempt && !forced_along(i, before, now, forced) {
            let (pop, pop_turn) = (
                s.start.position.distance(b.end.position),
                degrees(s.start.rotation, b.end.rotation),
            );
            if pop > 1e-4 || pop_turn > 0.1 {
                return Err(format!(
                    "actor {i} drawn from {:?} after it was last drawn at {:?}: {pop} m, {pop_turn}° at once",
                    s.start.position, b.end.position
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn no_random_program_draws_a_visible_actor_jumping_with_collision_wired() {
    let (mut moved, mut walked, mut reused) = (0, 0, 0);
    for seed in 0..PROGRAMS {
        let steps = program(seed);
        reused += steps
            .iter()
            .flat_map(|s| &s.commands)
            .filter(|c| matches!(c, Command::Spawn { entity, kind: Kind::Actor, .. } if entity.generation > 0))
            .count();
        let logic = Played {
            steps: steps.clone(),
            step: 0,
            region: Vec::new(),
        };
        let settings = wired();
        let limits = (
            settings.motion.max_visible_speed * STEP_SECONDS * 1.001,
            settings.motion.turn_rate * STEP_SECONDS * 1.001,
        );
        let idle = Actions::for_settings(&settings.input);
        let mut engine = engine(logic, settings, &[]);
        let (mut before, mut covered_before) = (vec![DEAD; SLOTS as usize], false);
        for (k, step) in steps.iter().enumerate() {
            engine.step(&idle);
            let now = snapshot(&engine);
            let covered = engine.state().screen.covered();
            let exempt = covered_before || covered || engine.state().camera.cut();
            let forced: Vec<u32> = step
                .commands
                .iter()
                .filter_map(|c| match *c {
                    Command::Teleport {
                        entity,
                        force: true,
                        ..
                    } => Some(entity.index),
                    _ => None,
                })
                .collect();
            if let Err(why) = check(&before, &now, exempt, &forced, limits) {
                panic!(
                    "seed {seed}, step {k}: {why}\nthis step: {step:?}\nthe step before: {:?}",
                    steps[k.saturating_sub(1)]
                );
            }
            for (i, (b, n)) in before.iter().zip(&now).enumerate() {
                let went = n.alive && n.actor && b.end.position != n.end.position;
                moved += usize::from(went);
                let e = Entity {
                    index: i as u32,
                    generation: n.generation,
                };
                let goal = engine.state().motion.get(e).map(|m| m.goal);
                walked +=
                    usize::from(went && matches!(goal, Some(Goal::Velocity(_) | Goal::To { .. })));
            }
            (before, covered_before) = (now, covered);
        }
    }
    assert!(moved > 5_000, "only {moved} actor moves were checked");
    assert!(
        walked > 1_000,
        "only {walked} of them were walks the collision module made"
    );
    assert!(
        reused > 50,
        "only {reused} actors were spawned into a freed slot"
    );
}
