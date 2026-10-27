//! End to end tests that run a server and headless clients over localhost.
//!
//! Each test runs real apps for several seconds, so they are serialized to avoid competing for CPU,
//! which several of them deliberately saturate.

use std::{sync::Mutex, time::Duration};

use example::{
    client::ClientWorkload,
    loadtest::{self, LoadtestConfig, LoadtestSummary},
    report::ClientReport,
    simulation::{PhysicsScheme, workload::Workload},
};
use nevy_prediction::prelude::*;

static SERIAL: Mutex<()> = Mutex::new(());

fn run(port: u16, configure: impl FnOnce(&mut LoadtestConfig)) -> LoadtestSummary {
    let _guard = SERIAL.lock().unwrap_or_else(|error| error.into_inner());

    let mut config = LoadtestConfig::default();
    config.server.port = port;
    configure(&mut config);

    let summary = loadtest::run(config);
    summary.print();
    summary
}

/// Every client connected, and its own player was exactly where the server had it on every tick.
fn assert_healthy(client: &ClientReport) {
    assert_healthy_with(client, 2.);
}

/// Like [`assert_healthy`] with a limit on how far remote players may be mispredicted,
/// which grows with latency.
fn assert_healthy_with(client: &ClientReport, remote_limit: f32) {
    assert!(client.connected, "{} never connected", client.name);
    assert!(
        client.errors.local.samples > 50,
        "{} recorded too few samples: {}",
        client.name,
        client.errors.local.samples
    );
    assert!(
        client.errors.local.max < 1e-4,
        "{}'s local player diverged from the server by {}",
        client.name,
        client.errors.local.max
    );
    // remote players mispredict until their inputs arrive, but are corrected within a few ticks
    assert!(
        client.errors.remote.max < remote_limit,
        "{}'s remote players diverged from the server by {}",
        client.name,
        client.errors.remote.max
    );
    assert!(
        client.predictions_completed > 20,
        "{} only completed {} predictions",
        client.name,
        client.predictions_completed
    );
}

#[test]
fn clients_match_the_server() {
    let summary = run(27701, |config| {
        config.clients = 3;
        config.duration = Duration::from_secs(6);
    });

    for client in &summary.clients {
        assert_healthy(client);

        // the main world ran in real time
        let expected = 6. / PhysicsScheme::step_interval().as_secs_f32();
        assert!(
            (client.main.ticks as f32 - expected).abs() < expected * 0.1,
            "{} ran {} ticks, expected about {}",
            client.name,
            client.main.ticks,
            expected
        );
    }

    assert!(summary.server.time_scale.min > 0.99);
    assert_eq!(summary.server.skipped_ms, 0.);
}

#[test]
fn queued_updates_are_replayed_to_new_clients() {
    // The drones are queued at startup for a tick after every client has joined,
    // so clients only get them if queued updates are replayed to them.
    let summary = run(27702, |config| {
        config.clients = 2;
        config.duration = Duration::from_secs(5);
        config.server.drones = 50;
        config.server.drone_delay = Duration::from_secs(3);
    });

    assert_eq!(summary.server.drones, 50);

    for client in &summary.clients {
        assert_healthy(client);
        assert_eq!(client.drones, 50, "{} is missing drones", client.name);
    }
}

#[test]
fn slow_client_slows_down_the_simulation() {
    // client 0 takes longer than a step to execute each tick in every world
    let step = PhysicsScheme::step_interval();
    let cost = step.mul_f32(1.4);

    let summary = run(27703, |config| {
        config.clients = 2;
        config.duration = Duration::from_secs(10);
        config.slow_client = Some(ClientWorkload::all(Workload::tick(cost)));
    });

    let server = &summary.server;
    assert!(
        server.limited_by_client > 0.5,
        "the time scale was only limited by a client {:.0}% of the time",
        server.limited_by_client * 100.
    );
    assert!(
        server.time_scale.last < 0.75 && server.time_scale.last > 0.2,
        "unexpected time scale {}",
        server.time_scale.last
    );

    for client in &summary.clients {
        assert_healthy(client);
    }

    // the slow client kept up once the simulation slowed down
    let slow = &summary.clients[0];
    assert!(
        slow.main_lag.last <= 2.,
        "the slow client is {} ticks behind",
        slow.main_lag.last
    );
}

#[test]
fn slow_server_slows_down_the_simulation() {
    let step = PhysicsScheme::step_interval();

    let summary = run(27704, |config| {
        config.clients = 2;
        config.duration = Duration::from_secs(8);
        config.server.workload = Workload::tick(step.mul_f32(1.4));
    });

    let server = &summary.server;
    assert!(server.limited_by_server > 0.5);
    assert!(
        server.time_scale.last < 0.75 && server.time_scale.last > 0.4,
        "unexpected time scale {}",
        server.time_scale.last
    );
    assert_eq!(server.skipped_ms, 0., "the server fell behind");

    for client in &summary.clients {
        assert_healthy(client);
    }
}

#[test]
fn simulation_is_independent_of_frame_rate() {
    // clients run at about 12 frames per second
    let summary = run(27705, |config| {
        config.clients = 2;
        config.duration = Duration::from_secs(6);
        config.client.frame_cost = Duration::from_millis(80);
        // Messages wait up to a frame to be sent and received, so a larger interval is needed.
        // Starting with enough means inputs aren't late while the interval adapts.
        config.client.prediction_interval = Duration::from_millis(250);
    });

    for client in &summary.clients {
        assert_healthy_with(client, 4.);

        let expected = 6. / PhysicsScheme::step_interval().as_secs_f32();
        assert!(
            (client.main.ticks as f32 - expected).abs() < expected * 0.1,
            "{} ran {} ticks at a low frame rate, expected about {}",
            client.name,
            client.main.ticks,
            expected
        );
    }
}

#[test]
fn inline_execution_matches_the_server() {
    let summary = run(27706, |config| {
        config.clients = 2;
        config.duration = Duration::from_secs(5);
        config.server.execution = SimulationExecution::Inline;
        config.client.execution = SimulationExecution::Inline;
    });

    for client in &summary.clients {
        assert_healthy(client);
    }
}

#[test]
fn physics_props_match_the_server() {
    const PROPS: u32 = 40;

    let summary = run(27707, |config| {
        config.clients = 2;
        config.duration = Duration::from_secs(8);
        config.server.props = PROPS;
    });

    assert_eq!(summary.server.props, PROPS as usize);
    assert_eq!(summary.server.props_escaped, 0, "props aren't colliding with the walls");

    for client in &summary.clients {
        assert_healthy(client);
        assert_eq!(client.props, PROPS as usize, "{} is missing props", client.name);
        // every ray from the middle of the arena hits a wall, if not a prop
        assert!(
            client.raycast_hits > 0.99,
            "only {} of {}'s rays hit anything",
            client.raycast_hits,
            client.name
        );

        let props = &client.errors.props;
        assert!(
            props.samples > 1000,
            "{} recorded too few prop samples: {}",
            client.name,
            props.samples
        );
        // Contacts don't simulate exactly the same in every world, but corrections keep them close.
        assert!(
            props.p99 < 0.25,
            "{}'s props diverged from the server by {} at p99",
            client.name,
            props.p99
        );
    }
}
