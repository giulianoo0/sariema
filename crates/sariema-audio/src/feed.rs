// SPDX-License-Identifier: Apache-2.0

//! The decoding thread behind [`crate::Feeding::Thread`]. Targets without
//! threads (`wasm32`, the web) leave this module out.

use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::AudioError;
use crate::stream::{Feeder, Pump};

/// Dropping it lets the thread end once its streams are abandoned.
pub(crate) struct FeedThread {
    feeders: Sender<Feeder>,
    _thread: JoinHandle<()>,
}

/// Hands `feeder` to the decoding thread, starting one when there is none
/// or the last one ended.
pub(crate) fn feed(
    slot: &mut Option<FeedThread>,
    feeder: Feeder,
    poll: Duration,
    errors: &Arc<Mutex<Vec<String>>>,
) -> Result<(), AudioError> {
    let feeder = match slot {
        Some(running) => match running.feeders.send(feeder) {
            Ok(()) => return Ok(()),
            Err(mpsc::SendError(feeder)) => feeder,
        },
        None => feeder,
    };
    let (tx, rx) = mpsc::channel::<Feeder>();
    let errors = errors.clone();
    let thread = thread::Builder::new()
        .name("sariema-audio-decode".into())
        .spawn(move || {
            let mut feeders = vec![feeder];
            loop {
                match rx.try_recv() {
                    Ok(f) => feeders.push(f),
                    Err(TryRecvError::Disconnected) if feeders.is_empty() => return,
                    Err(_) => {}
                }
                feeders.retain_mut(|f| match f.pump() {
                    Ok(Pump::Abandoned) => false,
                    Ok(_) => true,
                    Err(e) => {
                        errors.lock().expect("never poisoned").push(e.to_string());
                        true
                    }
                });
                thread::sleep(poll);
            }
        })
        .map_err(|e| AudioError::Io(format!("cannot start the decoding thread: {e}")))?;
    *slot = Some(FeedThread {
        feeders: tx,
        _thread: thread,
    });
    Ok(())
}
