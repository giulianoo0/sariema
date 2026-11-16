// SPDX-License-Identifier: Apache-2.0

//! An engine given a room's collision with the runtime's own mover still
//! on would move every actor twice; it refuses to start, as `load` would
//! switch the mover off.

mod common;

use sariema_engine::{Engine, EngineSettings};
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{assets, native, wired};

#[test]
fn collision_with_the_builtin_mover_on_is_refused() {
    let sim = Simulation::new(Settings::default(), Vec::new(), native(|_, _, _| {}), 1);
    let Err(e) = Engine::new(sim, assets(), EngineSettings::default()) else {
        panic!("an engine with collision and the built-in mover started");
    };
    assert_eq!(e.code, "E_SETTINGS");
    assert!(e.message.contains("motion.builtin"), "{}", e.message);
    let sim = Simulation::new(wired(), Vec::new(), native(|_, _, _| {}), 1);
    assert!(Engine::new(sim, assets(), EngineSettings::default()).is_ok());
}
