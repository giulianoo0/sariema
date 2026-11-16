// SPDX-License-Identifier: Apache-2.0

//! Seeded random programs of commands and transform region records. After
//! every step, no visible actor is drawn sliding farther than
//! `motion.max_visible_speed` allows in a step or turning faster than
//! `motion.turn_rate`, nor drawn starting anywhere but where the last step
//! left it, unless the step was covered or cut, or a forced teleport moved it
//! or a carrier it kept through the step.

mod common;

use glam::{Quat, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Command, Ease, Entity, Kind, Property};
use sariema_runtime::input::Actions;
use sariema_runtime::rng::Pcg32;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, REGION_TRANSFORMS, Simulation, Stop};
use sariema_runtime::world::Pose;

use common::{entity, send};

const SLOTS: u32 = 6;
const PROGRAMS: u64 = 300;
const STEPS: usize = 300;
/// Metres from the origin within which f32 rounding of composed placements
/// stays under a millimetre; a program that throws anything farther is
/// checked up to there.
const REACH: f32 = 10_000.0;

/// What a step left of one slot.
#[derive(Clone, Copy, Debug)]
struct Seen {
    alive: bool,
    actor: bool,
    visible: bool,
    parent: Entity,
    /// Where the step's frames end and start drawing it.
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
    actor: false,
    visible: false,
    parent: Entity::NONE,
    end: NOWHERE,
    start: NOWHERE,
};

fn spawn(index: u32, kind: Kind) -> Command<'static> {
    Command::Spawn {
        entity: entity(index),
        kind,
        asset: 0,
    }
}

fn pick<T: Copy>(rng: &mut Pcg32, from: &[T]) -> T {
    from[rng.next_u32() as usize % from.len()]
}

fn value(rng: &mut Pcg32) -> (Property, f32) {
    use Property as P;
    let property = pick(
        rng,
        &[
            P::PositionX,
            P::PositionY,
            P::PositionZ,
            P::Yaw,
            P::Pitch,
            P::Roll,
            P::Scale,
        ],
    );
    let to = match property {
        P::PositionX | P::PositionY | P::PositionZ => rng.range(-30.0, 30.0),
        P::Scale => rng.range(0.3, 2.5),
        _ => rng.range(-360.0, 360.0),
    };
    (property, to)
}

/// What a program does in one step: its commands, then the transform region
/// records `(entity, position, yaw)` it leaves after its tick.
#[derive(Clone, Debug, Default)]
struct Step {
    commands: Vec<Command<'static>>,
    region: Vec<(Entity, [f32; 3], f32)>,
}

/// Plays a program as a game.
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

/// One random command (two for a camera cut) or region record into `out`.
fn command(rng: &mut Pcg32, spawned: &mut u32, out: &mut Step) {
    let out_region = &mut out.region;
    let out = &mut out.commands;
    let e = entity(rng.next_u32() % SLOTS);
    let other = entity(rng.next_u32() % SLOTS);
    let at = [
        rng.range(-30.0, 30.0),
        rng.range(0.0, 3.0),
        rng.range(-30.0, 30.0),
    ];
    let yaw = rng.range(-180.0, 180.0);
    let coin = rng.next_u32().is_multiple_of(2);
    let ease = pick(rng, &[Ease::Linear, Ease::InOutCubic, Ease::OutBack]);
    match rng.next_u32() % 15 {
        0 if *spawned < SLOTS => {
            out.push(spawn(
                *spawned,
                if coin { Kind::Actor } else { Kind::Empty },
            ));
            *spawned += 1;
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
                duration: pick(rng, &[0.0, STEP_SECONDS, 2.0 * STEP_SECONDS, 0.1, 0.5, 2.0]),
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
            offset: [rng.range(-2.0, 2.0), 0.0, rng.range(-2.0, 2.0)],
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
                position: at,
                look_at: [0.0; 3],
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
            velocity: [rng.range(-25.0, 25.0), 0.0, rng.range(-25.0, 25.0)],
        }),
        13 => out.push(Command::SetScale {
            entity: e,
            scale: pick(rng, &[0.0, 0.5, 1.0, 2.0]),
        }),
        14 => out_region.push((e, at, yaw)),
        _ => {}
    }
}

fn program(seed: u64) -> Vec<Step> {
    let mut rng = Pcg32::new(seed, 0x6a);
    let mut steps = vec![Step {
        commands: vec![
            spawn(0, Kind::Actor),
            spawn(1, Kind::Actor),
            spawn(2, Kind::Empty),
        ],
        region: Vec::new(),
    }];
    let mut spawned = 3;
    for _ in 1..STEPS {
        let mut out = Step::default();
        for _ in 0..rng.next_u32() % 4 {
            command(&mut rng, &mut spawned, &mut out);
        }
        steps.push(out);
    }
    steps
}

fn snapshot<L: Logic>(sim: &Simulation<L>) -> Vec<Seen> {
    let world = &sim.state().world;
    (0..SLOTS)
        .map(|i| match world.node(entity(i)) {
            Some(n) => Seen {
                alive: true,
                actor: n.kind == Kind::Actor,
                visible: n.visible,
                parent: n.parent,
                end: world.pose(entity(i)).unwrap(),
                start: world.interpolated(entity(i), 0.0).unwrap(),
            },
            None => DEAD,
        })
        .collect()
}

/// The angle between two rotations, exact near zero, where
/// `Quat::angle_between` reads rounding as a tenth of a degree.
fn degrees(a: Quat, b: Quat) -> f32 {
    let d = a.conjugate() * b;
    (2.0 * d.xyz().length().atan2(d.w.abs())).to_degrees()
}

/// Whether a forced teleport this step moved slot `i`, or a carrier it has
/// kept since the step began.
fn forced_along(i: usize, before: &[Seen], now: &[Seen], forced: &[Entity]) -> bool {
    let mut at = i;
    loop {
        if forced.contains(&entity(at as u32)) {
            return true;
        }
        let parent = now[at].parent;
        if parent != before[at].parent || parent.is_none() {
            return false;
        }
        at = parent.index as usize;
    }
}

struct Limits {
    distance: f32,
    degrees: f32,
}

fn check(
    before: &[Seen],
    now: &[Seen],
    exempt: bool,
    forced: &[Entity],
    limits: &Limits,
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
        if slide > limits.distance || spin > limits.degrees {
            return Err(format!(
                "actor {i} drawn sliding {slide} m and turning {spin}° in one step"
            ));
        }
        let b = before[i];
        if b.alive && b.visible && !exempt && !forced_along(i, before, now, forced) {
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
fn no_random_program_draws_a_visible_actor_jumping() {
    let mut checked = 0;
    for seed in 0..PROGRAMS {
        let steps = program(seed);
        let logic = Played {
            steps: steps.clone(),
            step: 0,
            region: Vec::new(),
        };
        let settings = Settings::default();
        let limits = Limits {
            distance: settings.motion.max_visible_speed * STEP_SECONDS * 1.001,
            degrees: settings.motion.turn_rate * STEP_SECONDS * 1.001,
        };
        let idle = Actions::for_settings(&settings.input);
        let mut sim = Simulation::new(settings, vec![], logic, seed);
        let (mut before, mut covered_before) = (vec![DEAD; SLOTS as usize], false);
        for (k, step) in steps.iter().enumerate() {
            sim.step(&idle);
            let now = snapshot(&sim);
            if now
                .iter()
                .any(|s| s.alive && s.end.position.length() > REACH)
            {
                break;
            }
            checked += 1;
            let covered = sim.state().screen.covered();
            let exempt = covered_before || covered || sim.state().camera.cut();
            let forced: Vec<Entity> = step
                .commands
                .iter()
                .filter_map(|c| match *c {
                    Command::Teleport {
                        entity,
                        force: true,
                        ..
                    } => Some(entity),
                    _ => None,
                })
                .collect();
            if let Err(why) = check(&before, &now, exempt, &forced, &limits) {
                panic!(
                    "seed {seed}, step {k}: {why}\nthis step: {step:?}\nthe step before: {:?}\nslots before: {before:#?}\nslots now: {now:#?}",
                    steps[k.saturating_sub(1)]
                );
            }
            (before, covered_before) = (now, covered);
        }
    }
    let all = PROGRAMS as usize * STEPS;
    assert!(
        checked * 10 >= all * 9,
        "only {checked} of {all} steps were within reach"
    );
}
