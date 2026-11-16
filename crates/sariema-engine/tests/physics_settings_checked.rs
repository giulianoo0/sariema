// SPDX-License-Identifier: Apache-2.0

//! Physics settings a host builds in Rust are checked like a game's: the
//! engine refuses to start on them with `E_SETTINGS` naming the key.

mod common;

use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_runtime::sim::Simulation;

use common::{assets, block, collision, native, wired};

#[test]
fn the_engine_refuses_physics_settings_out_of_range() {
    let mut settings = wired();
    settings.physics.nav.cell_size = 0.0;
    let sim = Simulation::new(settings, Vec::new(), native(|_, _, _| {}), 1);
    let mut assets = assets();
    let floor = block("floor", [-2.0, -0.1, -2.0], [2.0, 0.0, 2.0]);
    assets.rooms = vec![RoomAssets::collision(
        "room",
        collision("room", vec![floor]),
    )];
    let e = match Engine::new(sim, assets, EngineSettings::default()) {
        Ok(_) => panic!("the engine started on a cell size of 0"),
        Err(e) => e,
    };
    assert_eq!(e.code, "E_SETTINGS");
    assert!(e.message.contains("physics.nav.cell_size"), "{}", e.message);
}
