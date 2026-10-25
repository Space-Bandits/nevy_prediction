//! `server [port] [--tick-cost-ms N] [--drones N] [--inline] [--no-time-scale]`

use example::{
    args::Args,
    server::{ServerConfig, build_server_app},
    simulation::workload::Workload,
};
use nevy_prediction::prelude::*;

fn main() {
    let args = Args::from_env();

    let mut config = ServerConfig::default();

    if let Some(port) = args.positional(0) {
        config.port = port.parse().expect("Invalid bind port");
    }

    config.workload = Workload {
        tick_cost: args.millis("tick-cost-ms").unwrap_or_default(),
        drone_cost: args.millis("drone-cost-ms").unwrap_or_default(),
        ..Default::default()
    };
    config.drones = args.get_or("drones", 0);
    config.time_scale.enabled = !args.flag("no-time-scale");

    if args.flag("inline") {
        config.execution = SimulationExecution::Inline;
    }

    build_server_app(config).run();
}
