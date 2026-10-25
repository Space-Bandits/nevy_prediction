//! Runs a server and many headless clients in one process, each app on its own thread, and reports how prediction performed.
//!
//! Every app runs a real networking stack over localhost, so this exercises the same code paths as separate processes.
//! Use [`LoadtestConfig`] to add artificial workloads to the server or clients and see how the simulation slows down.

use std::{
    net::{Ipv4Addr, SocketAddr},
    thread::JoinHandle,
    time::Duration,
};

use bevy::prelude::*;
use serde::Serialize;

use crate::{
    client::{ClientConfig, ClientWorkload, build_client_app},
    report::{ClientReport, ReportSink, ServerReport},
    server::{ServerConfig, build_server_app},
};

#[derive(Clone)]
pub struct LoadtestConfig {
    pub clients: usize,
    /// How long each client runs for.
    pub duration: Duration,
    /// Time between clients connecting.
    pub stagger: Duration,
    pub server: ServerConfig,
    /// Used for every client, with the name, index, address and bot seed set per client.
    pub client: ClientConfig,
    /// If set, client 0 uses this workload instead of the one in [`Self::client`].
    pub slow_client: Option<ClientWorkload>,
}

impl Default for LoadtestConfig {
    fn default() -> Self {
        LoadtestConfig {
            clients: 2,
            duration: Duration::from_secs(10),
            stagger: Duration::from_millis(250),
            server: ServerConfig {
                log: false,
                ..default()
            },
            client: ClientConfig {
                window: false,
                log: false,
                ..default()
            },
            slow_client: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LoadtestSummary {
    pub server: ServerReport,
    pub clients: Vec<ClientReport>,
}

pub fn run(config: LoadtestConfig) -> LoadtestSummary {
    let sink = ReportSink::default();

    let server_duration = config.duration
        + config.stagger * config.clients as u32
        + Duration::from_millis(1500);

    let server = spawn_app("loadtest server", {
        let config = ServerConfig {
            duration: Some(server_duration),
            report: Some(sink.clone()),
            ..config.server.clone()
        };
        move || build_server_app(config)
    });

    // give the server time to open its endpoint
    std::thread::sleep(Duration::from_millis(300));

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, config.server.port));

    let clients: Vec<_> = (0..config.clients)
        .map(|index| {
            if index > 0 {
                std::thread::sleep(config.stagger);
            }

            let mut client = ClientConfig {
                name: format!("client {index}"),
                index,
                server_address: address,
                window: false,
                bot: Some(index as u64 + 1),
                duration: Some(config.duration),
                report: Some(sink.clone()),
                ..config.client.clone()
            };

            if index == 0
                && let Some(workload) = config.slow_client
            {
                client.workload = workload;
            }

            spawn_app("loadtest client", move || build_client_app(client))
        })
        .collect();

    for client in clients {
        join(client);
    }
    join(server);

    let reports = sink.lock();

    let mut clients = reports.clients.clone();
    clients.sort_by_key(|client| client.index);
    for client in &mut clients {
        client.errors = reports.truth.errors(client.index);
    }

    LoadtestSummary {
        server: reports
            .server
            .clone()
            .expect("the server didn't write a report"),
        clients,
    }
}

fn spawn_app(name: &str, build: impl FnOnce() -> App + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            build().run();
        })
        .expect("failed to spawn app thread")
}

fn join(thread: JoinHandle<()>) {
    if let Err(payload) = thread.join() {
        std::panic::resume_unwind(payload);
    }
}

impl LoadtestSummary {
    /// Prints a human readable summary.
    pub fn print(&self) {
        let server = &self.server;

        println!("server");
        println!(
            "  frames      mean {:6.2}ms  p50 {:6.2}ms  p99 {:6.2}ms  max {:6.2}ms",
            server.frames.mean_ms, server.frames.p50_ms, server.frames.p99_ms, server.frames.max_ms
        );
        println!(
            "  tick        {} ticks, {} cost, {:.0}% busy",
            server.lane.ticks,
            fmt_ms(server.lane.tick_cost_ms),
            server.lane.utilization * 100.
        );
        println!(
            "  time scale  min {:.2}  mean {:.2}  last {:.2}  (limited by server {:.0}%, by a client {:.0}%)",
            server.time_scale.min,
            server.time_scale.mean,
            server.time_scale.last,
            server.limited_by_server * 100.,
            server.limited_by_client * 100.
        );
        println!(
            "  lag         max {:.0} ticks, skipped {:.0}ms, {} late updates, {} clients joined, {} drones",
            server.lag_ticks.max,
            server.skipped_ms,
            server.late_updates,
            server.clients_joined,
            server.drones
        );

        for client in &self.clients {
            println!(
                "{}{}, {} drones",
                client.name,
                if client.connected { "" } else { " (never connected!)" },
                client.drones
            );
            println!(
                "  frames      mean {:6.2}ms  p50 {:6.2}ms  p99 {:6.2}ms  max {:6.2}ms",
                client.frames.mean_ms,
                client.frames.p50_ms,
                client.frames.p99_ms,
                client.frames.max_ms
            );

            for (name, lane) in [
                ("main", &client.main),
                ("template", &client.template),
                ("prediction", &client.prediction),
            ] {
                println!(
                    "  {:<10}  {:5} ticks, {} cost, {:.0}% busy",
                    name,
                    lane.ticks,
                    fmt_ms(lane.tick_cost_ms),
                    lane.utilization * 100.
                );
            }

            println!(
                "  predictions {} completed, {:.1} ticks long, interval {:.0}ms (min {:.0} max {:.0}), {} late probes",
                client.predictions_completed,
                client.prediction_length.unwrap_or(0.),
                client.interval_ms.mean,
                client.interval_ms.min,
                client.interval_ms.max,
                client.late_probes
            );
            println!(
                "  timing      capacity {:.2} (min {:.2}), time scale min {:.2}, main lag max {:.0}, template lag max {:.0}",
                client.capacity.mean,
                client.capacity.min,
                client.time_scale.min,
                client.main_lag.max,
                client.template_lag.max
            );

            println!(
                "  presenting  {:.1}% holds, {:.1}% jumps, fraction mean {:.2}, {:.1}ms behind latest tick (max {:.1})",
                client.presentation.holds * 100.,
                client.presentation.jumps * 100.,
                client.presentation.fraction.mean,
                client.presentation.behind_latest_ms.mean,
                client.presentation.behind_latest_ms.max
            );

            for (name, errors) in [
                ("local", &client.errors.local),
                ("remote", &client.errors.remote),
            ] {
                println!(
                    "  {:<6} err  {:5} samples, {:5.1}% mispredicted, mean {:.4} p99 {:.4} max {:.4}",
                    name,
                    errors.samples,
                    errors.mispredicted * 100.,
                    errors.mean,
                    errors.p99,
                    errors.max
                );
            }
        }
    }
}

fn fmt_ms(millis: Option<f32>) -> String {
    millis
        .map(|millis| format!("{millis:6.2}ms"))
        .unwrap_or_else(|| "     n/a".to_string())
}
