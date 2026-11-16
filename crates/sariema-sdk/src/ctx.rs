// SPDX-License-Identifier: Apache-2.0

use alloc::vec::Vec;

use crate::Rng;
use crate::host::Host;
use crate::table::{Command, Event};
use crate::wire::{Ease, Entity, Kind, Sink, put_scene_value};

impl Sink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

/// The game's entity handles. The game picks the slot of every entity it
/// spawns; a freed slot is reused with a newer generation, so an old handle
/// to it is refused by the engine as stale.
#[derive(Debug, Default)]
pub struct Entities {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
}

impl Entities {
    pub const fn new() -> Entities {
        Entities {
            generations: Vec::new(),
            alive: Vec::new(),
            free: Vec::new(),
        }
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        let i = e.index as usize;
        self.alive.get(i) == Some(&true) && self.generations[i] == e.generation
    }

    fn alloc(&mut self) -> Entity {
        match self.free.pop() {
            Some(index) => {
                let i = index as usize;
                self.generations[i] += 1;
                self.alive[i] = true;
                Entity {
                    index,
                    generation: self.generations[i],
                }
            }
            None => {
                self.generations.push(0);
                self.alive.push(true);
                Entity {
                    index: self.alive.len() as u32 - 1,
                    generation: 0,
                }
            }
        }
    }

    fn release(&mut self, e: Entity) {
        if self.is_alive(e) {
            self.alive[e.index as usize] = false;
            self.free.push(e.index);
        }
    }
}

/// The SDK's own state between ticks.
pub(crate) struct Shared {
    pub(crate) commands: Vec<u8>,
    /// The values of the `scene_tween` being sent.
    scene_values: Vec<u8>,
    pub(crate) entities: Entities,
    pub(crate) rng: Rng,
    pub(crate) step: u64,
}

/// The stream of the game's generator, apart from the engine's own.
const GAME_STREAM: u64 = 0x5a;

const SPAWN: u16 = Command::Spawn {
    entity: Entity::NONE,
    kind: Kind::Empty,
    asset: 0,
}
.opcode();

impl Shared {
    pub(crate) const fn new() -> Shared {
        Shared {
            commands: Vec::new(),
            scene_values: Vec::new(),
            entities: Entities::new(),
            rng: Rng::new(0, GAME_STREAM),
            step: 0,
        }
    }

    /// What the SDK itself learns from an event.
    pub(crate) fn observe(&mut self, event: &Event<'_>) {
        match *event {
            Event::Started { seed } => self.rng = Rng::new(seed, GAME_STREAM),
            Event::Despawned { entity } => self.entities.release(entity),
            Event::Rejected { opcode, entity, .. } if opcode == SPAWN => {
                self.entities.release(entity)
            }
            _ => {}
        }
    }
}

/// What a game acts through during a tick.
pub struct Ctx<'a> {
    pub(crate) shared: &'a mut Shared,
    pub(crate) host: &'a mut dyn Host,
}

impl Ctx<'_> {
    /// The step being simulated, from 0.
    pub fn step(&self) -> u64 {
        self.shared.step
    }

    /// The game's generator, seeded from the engine's seed.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.shared.rng
    }

    pub fn entities(&self) -> &Entities {
        &self.shared.entities
    }

    /// Sends a command; commands apply in the order sent, after the tick.
    pub fn send(&mut self, command: Command<'_>) {
        command.write(&mut self.shared.commands);
    }

    /// Tweens scene parameters of the active room together under `tween`:
    /// each pair is a parameter's [`crate::id`] and the value it goes to.
    pub fn scene_tween(&mut self, tween: u32, values: &[(u32, f32)], duration: f32, ease: Ease) {
        let bytes = &mut self.shared.scene_values;
        bytes.clear();
        for &(param, value) in values {
            put_scene_value(bytes, param, value);
        }
        Command::SceneTween {
            tween,
            values: bytes,
            duration,
            ease,
        }
        .write(&mut self.shared.commands);
    }

    /// Spawns an entity in a free slot and returns its handle.
    pub fn spawn(&mut self, kind: Kind, asset: u32) -> Entity {
        let entity = self.shared.entities.alloc();
        self.send(Command::Spawn {
            entity,
            kind,
            asset,
        });
        entity
    }

    /// A handle for an entity something else spawns: a cutscene role
    /// without a start place, bound before the cutscene plays. The slot is
    /// freed when its `despawned` event arrives.
    pub fn reserve(&mut self) -> Entity {
        self.shared.entities.alloc()
    }

    pub fn despawn(&mut self, entity: Entity) {
        self.send(Command::Despawn { entity });
        self.shared.entities.release(entity);
    }

    /// Casts `entity` in `role` for the next cutscene played.
    pub fn bind(&mut self, role: u32, entity: Entity) {
        self.send(Command::CutsceneBind { role, entity });
    }

    pub fn play_cutscene(&mut self, cutscene: u32) {
        self.send(Command::CutscenePlay { cutscene });
    }

    /// Skips the cutscene playing, under a fade, to the state playing it
    /// through would leave.
    pub fn skip_cutscene(&mut self) {
        self.send(Command::CutsceneSkip {});
    }

    /// Releases a cutscene's `wait_for` on `signal`.
    pub fn signal(&mut self, signal: u32) {
        self.send(Command::Signal { signal });
    }

    /// The index `button` events carry for the input button named `name`
    /// (an [`crate::id`]), so a game finds its buttons whatever their
    /// order in the project's settings; `None` when there is none.
    pub fn button(&self, name: u32) -> Option<u16> {
        u16::try_from(self.host.button(name)).ok()
    }

    /// `level` 0 debug, 1 info, 2 warning, 3 error.
    pub fn log(&mut self, level: u32, text: &str) {
        self.host.log(level, text);
    }

    /// Registers `bytes` as a bulk region the engine reads after every
    /// tick. The bytes must stay where they are: register a buffer that
    /// never grows, such as [`crate::Transforms`]. The region's id, or
    /// `None` when the engine refused it.
    pub fn register(&mut self, kind: u32, bytes: &[u8]) -> Option<u32> {
        u32::try_from(self.host.register(kind, bytes)).ok()
    }
}
