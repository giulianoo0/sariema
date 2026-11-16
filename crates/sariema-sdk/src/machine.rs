// SPDX-License-Identifier: Apache-2.0

/// A state and the step it was entered on, for game logic written as states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine<S> {
    state: S,
    since: u64,
    changed: bool,
}

impl<S: Copy + PartialEq> Machine<S> {
    pub fn new(state: S) -> Machine<S> {
        Machine {
            state,
            since: 0,
            changed: true,
        }
    }

    pub fn state(&self) -> S {
        self.state
    }

    pub fn is(&self, state: S) -> bool {
        self.state == state
    }

    /// Moves to `state` at `step`; staying in the same state does nothing.
    pub fn go(&mut self, state: S, step: u64) {
        if state != self.state {
            self.state = state;
            self.since = step;
            self.changed = true;
        }
    }

    /// True once after each change, the first time it is asked.
    pub fn entered(&mut self) -> bool {
        core::mem::take(&mut self.changed)
    }

    /// Steps spent in the current state.
    pub fn steps_in(&self, step: u64) -> u64 {
        step - self.since
    }
}
