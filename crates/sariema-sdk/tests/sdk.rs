// SPDX-License-Identifier: Apache-2.0

use sariema_format::script as engine;
use sariema_sdk::testing::{TestHost, commands};
use sariema_sdk::{Command, Ctx, Entity, Event, Events, Game, Kind, Machine, Runner, Transforms};

#[test]
fn the_sdk_encodes_every_command_as_the_engine_decodes_it() {
    let sdk: Vec<Command> = Command::samples().collect();
    let ours: Vec<engine::Command> = engine::Command::samples().collect();
    assert_eq!(sdk.len(), engine::COMMANDS.len());
    for (a, b) in sdk.iter().zip(&ours) {
        let mut bytes = Vec::new();
        a.write(&mut bytes);
        let decoded: Vec<_> = engine::records::<engine::Command>(&bytes).collect();
        assert_eq!(decoded, [Ok(*b)], "{}", a.name());
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}

#[test]
fn the_sdk_reads_every_event_the_engine_encodes() {
    let mut bytes = Vec::new();
    let ours: Vec<engine::Event> = engine::Event::samples().collect();
    ours.iter().for_each(|e| e.write(&mut bytes));
    let read: Vec<String> = Events::new(&bytes)
        .iter()
        .map(|e| format!("{e:?}"))
        .collect();
    let expected: Vec<String> = ours.iter().map(|e| format!("{e:?}")).collect();
    assert_eq!(read, expected);
    assert_eq!(read.len(), engine::EVENTS.len());
}

#[derive(Default)]
struct Probe {
    spawned: Vec<Entity>,
    seen: Vec<String>,
    rolls: Vec<u32>,
}

impl Game for Probe {
    fn new(ctx: &mut Ctx<'_>) -> Probe {
        let first = ctx.spawn(Kind::Actor, 1);
        let second = ctx.spawn(Kind::Empty, 0);
        Probe {
            spawned: vec![first, second],
            rolls: vec![ctx.rng().next_u32()],
            ..Probe::default()
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for e in events {
            self.seen.push(format!("{e:?}"));
        }
        if ctx.step() == 1 {
            ctx.despawn(self.spawned[1]);
            let reused = ctx.spawn(Kind::Empty, 0);
            self.spawned.push(reused);
        }
    }
}

fn started(seed: u64) -> TestHost {
    let mut host = TestHost::new();
    host.event(Event::Started { seed });
    host
}

#[test]
fn spawned_handles_are_reused_with_a_newer_generation() {
    let mut host = started(9);
    let mut runner = Runner::<Probe>::new();
    runner.run(&mut host);
    let first: Vec<String> = commands(&host.take_commands())
        .map(|c| format!("{c:?}"))
        .collect();
    assert_eq!(
        first,
        [
            "Spawn { entity: Entity { index: 0, generation: 0 }, kind: Actor, asset: 1 }",
            "Spawn { entity: Entity { index: 1, generation: 0 }, kind: Empty, asset: 0 }",
        ]
    );
    runner.run(&mut host);
    let game = runner.game().unwrap();
    assert_eq!(
        game.spawned[2],
        Entity {
            index: 1,
            generation: 1
        }
    );
}

#[test]
fn a_despawn_by_the_engine_or_a_refused_spawn_frees_the_slot() {
    let mut host = started(1);
    let mut runner = Runner::<Probe>::new();
    runner.run(&mut host);
    let hero = runner.game().unwrap().spawned[0];
    host.event(Event::Despawned { entity: hero });
    host.event(Event::Rejected {
        opcode: 0x0001,
        code: "E_ENTITY_LIMIT",
        entity: Entity {
            index: 1,
            generation: 0,
        },
    });
    runner.run(&mut host);
    let game = runner.game().unwrap();
    assert_eq!(
        game.spawned[2],
        Entity {
            index: 1,
            generation: 1
        },
        "the last slot freed is the first reused"
    );
}

#[test]
fn a_reserved_handle_is_freed_by_its_despawned_event() {
    struct Ghost(Vec<Entity>);
    impl Game for Ghost {
        fn new(ctx: &mut Ctx<'_>) -> Ghost {
            let ghost = ctx.reserve();
            Ghost(vec![ghost])
        }
        fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
            if events.iter().any(|e| matches!(e, Event::Despawned { .. })) {
                self.0.push(ctx.reserve());
            }
        }
    }
    let mut host = started(0);
    let mut runner = Runner::<Ghost>::new();
    runner.run(&mut host);
    assert!(host.take_commands().is_empty(), "reserving sends nothing");
    let ghost = runner.game().unwrap().0[0];
    host.event(Event::Despawned { entity: ghost });
    runner.run(&mut host);
    assert_eq!(
        runner.game().unwrap().0[1],
        Entity {
            index: 0,
            generation: 1
        }
    );
}

#[test]
fn the_generator_is_seeded_from_the_started_event() {
    let roll = |seed| {
        let mut host = started(seed);
        let mut runner = Runner::<Probe>::new();
        runner.run(&mut host);
        runner.game().unwrap().rolls[0]
    };
    assert_eq!(roll(5), roll(5));
    assert_ne!(roll(5), roll(6));
}

#[test]
fn events_larger_than_a_read_arrive_whole_and_in_order() {
    let mut host = started(0);
    let long = "x".repeat(10_000);
    for i in 0..300 {
        host.event(Event::TimerFired { timer: i });
    }
    host.event(Event::HostMessage {
        topic: "big",
        payload: long.as_bytes(),
    });
    host.event(Event::TimerFired { timer: 300 });
    let mut runner = Runner::<Probe>::new();
    runner.run(&mut host);
    let seen = &runner.game().unwrap().seen;
    assert_eq!(seen.len(), 303);
    assert!(seen[301].contains("HostMessage"));
    assert_eq!(seen[302], "TimerFired { timer: 300 }");
}

#[test]
fn a_transforms_region_has_the_engines_layout() {
    let mut region = Transforms::with_capacity(2);
    let bytes_at = region.as_bytes().as_ptr();
    region.push(
        Entity {
            index: 3,
            generation: 4,
        },
        [1.0, 2.0, 3.0],
        90.0,
    );
    let b = region.as_bytes();
    assert_eq!(b.len(), 4 + 2 * 24);
    assert_eq!(b.as_ptr(), bytes_at, "the buffer never moves");
    assert_eq!(&b[..4], &1u32.to_le_bytes());
    assert_eq!(&b[4..12], &[3, 0, 0, 0, 4, 0, 0, 0]);
    assert_eq!(&b[24..28], &90f32.to_le_bytes());
    region.clear();
    assert!(region.is_empty());
}

#[test]
fn a_machine_reports_each_entry_once() {
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum S {
        Idle,
        Walk,
    }
    let mut m = Machine::new(S::Idle);
    assert!(m.entered());
    assert!(!m.entered());
    m.go(S::Idle, 3);
    assert!(!m.entered());
    m.go(S::Walk, 5);
    assert!(m.is(S::Walk) && m.entered());
    assert_eq!(m.steps_in(9), 4);
}
