// SPDX-License-Identifier: Apache-2.0

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::ctx::{Ctx, Shared};
use crate::host::Host;
use crate::table::Event;
use crate::wire::Reader;

/// A game's logic. [`Game::new`] runs on the first tick, after the SDK
/// read that tick's events; [`Game::tick`] then runs on every tick,
/// the first included.
pub trait Game: Sized {
    fn new(ctx: &mut Ctx<'_>) -> Self;
    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>);
}

/// The events of one tick, oldest first.
#[derive(Clone, Copy)]
pub struct Events<'a> {
    bytes: &'a [u8],
}

impl<'a> Events<'a> {
    pub fn new(bytes: &'a [u8]) -> Events<'a> {
        Events { bytes }
    }

    pub fn iter(self) -> EventIter<'a> {
        EventIter {
            reader: Reader::new(self.bytes),
        }
    }

    pub fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }
}

impl<'a> IntoIterator for Events<'a> {
    type Item = Event<'a>;
    type IntoIter = EventIter<'a>;

    fn into_iter(self) -> EventIter<'a> {
        self.iter()
    }
}

pub struct EventIter<'a> {
    reader: Reader<'a>,
}

impl<'a> Iterator for EventIter<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        if self.reader.is_empty() {
            return None;
        }
        Some(Event::read(&mut self.reader).expect(
            "the engine sent an event this SDK cannot read; build the game against the engine's SDK",
        ))
    }
}

/// Bytes asked of the engine per call when reading events.
const EVENT_CHUNK: usize = 4096;

/// Drives a [`Game`]: reads the tick's events, ticks the game, submits its
/// commands.
pub struct Runner<G> {
    game: UnsafeCell<Option<G>>,
    shared: UnsafeCell<Shared>,
    inbox: UnsafeCell<Vec<u8>>,
}

// SAFETY: only `tick` touches the cells through a shared reference, and its
// caller promises the calls never overlap (a wasm32 module has one thread).
unsafe impl<G> Sync for Runner<G> {}

impl<G: Game> Default for Runner<G> {
    fn default() -> Runner<G> {
        Runner::new()
    }
}

impl<G: Game> Runner<G> {
    pub const fn new() -> Runner<G> {
        Runner {
            game: UnsafeCell::new(None),
            shared: UnsafeCell::new(Shared::new()),
            inbox: UnsafeCell::new(Vec::new()),
        }
    }

    /// One tick through a shared reference, for the `static` in [`crate::game!`].
    ///
    /// # Safety
    ///
    /// No other call to `tick` or [`Runner::run`] on this runner may be in
    /// progress.
    pub unsafe fn tick(&self, host: &mut dyn Host) {
        // SAFETY: the caller guarantees exclusive access for the call.
        let (game, shared, inbox) = unsafe {
            (
                &mut *self.game.get(),
                &mut *self.shared.get(),
                &mut *self.inbox.get(),
            )
        };
        tick(game, shared, inbox, host);
    }

    /// One tick, for tests that own the runner.
    pub fn run(&mut self, host: &mut dyn Host) {
        tick(
            self.game.get_mut(),
            self.shared.get_mut(),
            self.inbox.get_mut(),
            host,
        );
    }

    pub fn game(&mut self) -> Option<&mut G> {
        self.game.get_mut().as_mut()
    }
}

fn tick<G: Game>(
    game: &mut Option<G>,
    shared: &mut Shared,
    inbox: &mut Vec<u8>,
    host: &mut dyn Host,
) {
    read_events(host, inbox);
    for event in Events::new(inbox) {
        shared.observe(&event);
    }
    let mut ctx = Ctx { shared, host };
    let game = game.get_or_insert_with(|| G::new(&mut ctx));
    game.tick(&mut ctx, Events::new(inbox));
    host.submit(&shared.commands);
    shared.commands.clear();
    shared.step += 1;
}

fn read_events(host: &mut dyn Host, inbox: &mut Vec<u8>) {
    inbox.clear();
    let mut chunk = EVENT_CHUNK;
    loop {
        let start = inbox.len();
        inbox.resize(start + chunk, 0);
        let n = host.events(&mut inbox[start..]);
        inbox.truncate(start + usize::try_from(n).unwrap_or(0));
        match n {
            0 => return,
            n if n < 0 => chunk = n.unsigned_abs() as usize,
            _ => chunk = EVENT_CHUNK,
        }
    }
}
