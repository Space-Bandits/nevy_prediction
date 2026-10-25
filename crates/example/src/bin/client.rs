//! `client [server address] [--headless] [--bot SEED] [--tick-cost-ms N] [--frame-cost-ms N] [--inline]`

use example::{
    args::Args,
    client::{ClientConfig, ClientWorkload, build_client_app},
    simulation::workload::Workload,
};
use nevy_prediction::prelude::*;

fn main() {
    let args = Args::from_env();

    let mut config = ClientConfig::default();

    if let Some(address) = args.positional(0) {
        config.server_address = address.parse().expect("Invalid server address");
    }

    config.window = !args.flag("headless");
    config.bot = args.get("bot");
    config.frame_cost = args.millis("frame-cost-ms").unwrap_or_default();
    config.fps = args.get_or("fps", config.fps);
    config.workload = ClientWorkload::all(Workload::tick(
        args.millis("tick-cost-ms").unwrap_or_default(),
    ));

    if args.flag("inline") {
        config.execution = SimulationExecution::Inline;
    }

    build_client_app(config).run();
}
