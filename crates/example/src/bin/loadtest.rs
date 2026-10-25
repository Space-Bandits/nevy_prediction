//! Runs a server and headless clients in one process and prints how prediction performed.
//!
//! ```text
//! loadtest [--clients 2] [--duration 10] [--port 27610] [--drones 0] [--drone-delay SECONDS]
//!          [--server-cost-ms 0] [--client-cost-ms 0]
//!          [--client-main-cost-ms N] [--client-template-cost-ms N] [--client-prediction-cost-ms N]
//!          [--slow-client-cost-ms N]   extra cost for every world of client 0 only
//!          [--spike-chance 0..1] [--spike-ms N]   random spikes on every client world
//!          [--drone-cost-us N]         cost per drone per tick, spread across threads
//!          [--client-frame-cost-ms N] [--client-fps 60]
//!          [--interval-ms 100] [--inline] [--no-time-scale] [--log] [--json PATH]
//! ```

use std::time::Duration;

use example::{
    args::Args,
    client::ClientWorkload,
    loadtest::{self, LoadtestConfig},
    simulation::workload::Workload,
};
use nevy_prediction::prelude::*;

fn main() {
    let args = Args::from_env();

    let mut config = LoadtestConfig::default();

    config.clients = args.get_or("clients", config.clients);
    config.duration = args.seconds("duration").unwrap_or(config.duration);
    config.server.port = args.get_or("port", 27610);
    config.server.drones = args.get_or("drones", 0);
    config.server.drone_delay = args.seconds("drone-delay").unwrap_or_default();
    config.server.time_scale.enabled = !args.flag("no-time-scale");
    config.server.log = args.flag("log");
    config.client.log = false;

    let drone_cost = args
        .get::<f64>("drone-cost-us")
        .map(|micros| Duration::from_secs_f64(micros / 1e6))
        .unwrap_or_default();

    config.server.workload = Workload {
        tick_cost: args.millis("server-cost-ms").unwrap_or_default(),
        drone_cost,
        ..Default::default()
    };

    let client_cost = args.millis("client-cost-ms").unwrap_or_default();
    let client_workload = |key: &str| Workload {
        tick_cost: args.millis(key).unwrap_or(client_cost),
        spike_chance: args.get_or("spike-chance", 0.),
        spike_cost: args.millis("spike-ms").unwrap_or_default(),
        drone_cost,
    };

    config.client.workload = ClientWorkload {
        main: client_workload("client-main-cost-ms"),
        template: client_workload("client-template-cost-ms"),
        prediction: client_workload("client-prediction-cost-ms"),
    };

    if let Some(cost) = args.millis("slow-client-cost-ms") {
        config.slow_client = Some(ClientWorkload::all(Workload {
            tick_cost: cost,
            drone_cost,
            ..Default::default()
        }));
    }

    config.client.frame_cost = args.millis("client-frame-cost-ms").unwrap_or_default();
    config.client.fps = args.get_or("client-fps", config.client.fps);
    config.client.prediction_interval = args
        .millis("interval-ms")
        .unwrap_or(config.client.prediction_interval);

    if args.flag("inline") {
        config.server.execution = SimulationExecution::Inline;
        config.client.execution = SimulationExecution::Inline;
    }

    let summary = loadtest::run(config);
    summary.print();

    if let Some(path) = args.get::<String>("json") {
        std::fs::write(&path, serde_json::to_string_pretty(&summary).unwrap())
            .expect("failed to write json report");
        println!("wrote {path}");
    }

    if summary.clients.iter().any(|client| !client.connected) {
        std::process::exit(1);
    }
}
