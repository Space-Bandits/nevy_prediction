//! An example game using `nevy_prediction`, built to be useful for testing and profiling it.
//!
//! - [`server`] and [`client`] build the server and client apps from a config.
//! - [`simulation`] is the shared simulation: players moved by inputs, drones that add load,
//!   and a configurable [`Workload`](simulation::workload::Workload) that makes ticks artificially expensive.
//! - [`loadtest`] runs a server and many headless clients in one process and reports how prediction performed.
//!
//! The `server`, `client` and `loadtest` binaries are thin wrappers around these.

use bevy::prelude::*;
use nevy::prelude::*;

pub mod args;
pub mod client;
pub mod loadtest;
pub mod networking;
pub mod report;
pub mod server;
pub mod simulation;

/// Common logic for the server and client apps.
pub fn build(app: &mut App) {
    networking::build(app);

    app.add_protocol_message::<(), simulation::player::SetLocalPlayer>();
    app.add_protocol_message::<(), simulation::player::RequestMovePlayer>();
}

/// Spins the current thread for a duration, to simulate expensive work.
pub fn spin_for(duration: std::time::Duration) {
    let start = std::time::Instant::now();

    while start.elapsed() < duration {
        std::hint::spin_loop();
    }
}
