//! The server app.

use std::time::Duration;

use bevy::{
    app::ScheduleRunnerPlugin,
    log::{Level, LogPlugin},
    prelude::*,
};
use nevy::prelude::*;
use nevy_prediction::prelude::*;

use crate::{
    report::{FrameRecorder, Range, ReportSink, ServerReport, record_frames},
    simulation::{
        PhysicsScheme,
        drones::{Drone, SpawnDrones},
        player::{Player, PlayerState},
        workload::{Workload, hash},
    },
};

pub mod networking;
pub mod player;

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    /// Extra cost added to every tick of the server's simulation.
    pub workload: Workload,
    pub drones: u32,
    /// How long after starting the drones are spawned.
    ///
    /// The drones are queued at startup for a future tick, so clients that join before then
    /// only learn about them because queued updates are replayed to new clients.
    pub drone_delay: Duration,
    pub time_scale: TimeScaleSettings,
    pub execution: SimulationExecution,
    /// Minimum time between frames. The server busy loops if this is zero.
    pub frame_wait: Duration,
    /// Exit after this long.
    pub duration: Option<Duration>,
    pub report: Option<ReportSink>,
    pub log: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            port: 27510,
            workload: default(),
            drones: 0,
            drone_delay: Duration::ZERO,
            time_scale: default(),
            execution: default(),
            frame_wait: Duration::from_millis(1),
            duration: None,
            report: None,
            log: true,
        }
    }
}

#[derive(Resource, Clone)]
pub struct Config(pub ServerConfig);

pub fn build_server_app(config: ServerConfig) -> App {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(config.frame_wait)));

    if config.log {
        app.add_plugins(LogPlugin {
            level: Level::DEBUG,
            filter: bevy::log::DEFAULT_FILTER.to_string(),
            ..default()
        });
    }

    crate::build(&mut app);

    app.add_plugins(
        NevyPredictionServerPlugin::<PhysicsScheme>::new(Update).with_execution(config.execution),
    );
    app.include_protocol::<(), PredictionMessages>();
    app.insert_resource(config.time_scale.clone());

    networking::build(&mut app);
    player::build(&mut app);

    app.init_resource::<SimulationEntityAllocator>();
    app.init_resource::<FrameRecorder>();
    app.init_resource::<ServerRecorder>();
    app.insert_resource(Config(config));

    app.add_systems(Startup, (configure_workload, spawn_drones));
    app.add_systems(
        Update,
        init_drones
            .in_set(ServerSimulationSystems::QueueUpdates)
            .before(player::spawn_players),
    );
    app.add_systems(
        Last,
        (record_frames, record_server, exit_after_duration).chain(),
    );

    app
}

#[derive(Resource, Default)]
pub struct SimulationEntityAllocator {
    next_id: u64,
}

impl SimulationEntityAllocator {
    pub fn next(&mut self) -> SimulationEntity {
        let id = self.next_id;
        self.next_id += 1;
        SimulationEntity(id)
    }
}

fn configure_workload(config: Res<Config>, simulation: Res<ServerSimulation>) {
    let workload = config.0.workload;
    simulation.run_on_world(move |world| {
        world.insert_resource(workload);
    });
}

fn spawn_drones(
    config: Res<Config>,
    mut allocator: ResMut<SimulationEntityAllocator>,
    mut queue: ServerUpdateQueue<SpawnDrones>,
) {
    if config.0.drones == 0 {
        return;
    }

    let drones = (0..config.0.drones)
        .map(|index| {
            let entity = allocator.next();
            (entity, Drone {
                seed: hash(entity.0 ^ index as u64),
            })
        })
        .collect();

    let delay = config.0.drone_delay.as_secs_f64() / PhysicsScheme::step_interval().as_secs_f64();

    // No clients exist yet, so this only needs to be queued.
    // Clients that join before it reaches the app's world have it replayed to them.
    queue.queue(WorldUpdate {
        tick: SimulationTick(delay.ceil() as u32),
        update: SpawnDrones { drones },
    });
}

/// Sends existing drones to new clients.
fn init_drones(
    client_q: Query<Entity, Added<PredictionClient>>,
    drone_q: Query<(&SimulationEntity, &Drone)>,
    mut updates: WorldUpdateSender,
) -> Result {
    for client_entity in &client_q {
        let drones: Vec<_> = drone_q
            .iter()
            .map(|(&entity, &drone)| (entity, drone))
            .collect();

        if !drones.is_empty() {
            updates.write_now(client_entity, true, SpawnDrones { drones })?;
        }
    }

    Ok(())
}

/// Records what the server's report needs.
#[derive(Resource, Default)]
struct ServerRecorder {
    time_scale: Range,
    lag: Range,
    limited_by_server: u64,
    limited_by_client: u64,
    frames: u64,
    clients_joined: u32,
    last_tick: Option<SimulationTick>,
}

fn record_server(
    mut recorder: ResMut<ServerRecorder>,
    config: Res<Config>,
    stats: Res<ServerSimulationStats>,
    time_scale: Res<ServerTimeScale>,
    time: Res<Time<SimulationTime>>,
    new_client_q: Query<(), Added<PredictionClient>>,
    player_q: Query<(&SimulationEntity, &PlayerState), With<Player>>,
) {
    recorder.frames += 1;
    recorder.time_scale.record(stats.time_scale);
    recorder.lag.record(stats.lag as f32);
    recorder.clients_joined += new_client_q.iter().count() as u32;

    match time_scale.limit {
        TimeScaleLimit::None => {}
        TimeScaleLimit::Server => recorder.limited_by_server += 1,
        TimeScaleLimit::Client(_) => recorder.limited_by_client += 1,
    }

    let tick = time.current_tick();
    if recorder.last_tick == Some(tick) {
        return;
    }
    recorder.last_tick = Some(tick);

    if let Some(sink) = &config.0.report {
        let mut reports = sink.lock();
        for (&entity, state) in &player_q {
            reports.truth.record_server(entity, tick, state.position);
        }
    }
}

fn exit_after_duration(
    config: Res<Config>,
    real_time: Res<Time<Real>>,
    recorder: Res<ServerRecorder>,
    frames: Res<FrameRecorder>,
    stats: Res<ServerSimulationStats>,
    drone_q: Query<(), With<crate::simulation::drones::Drone>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(duration) = config.0.duration else {
        return;
    };

    if real_time.elapsed() < duration {
        return;
    }

    if let Some(sink) = &config.0.report {
        let frame_count = recorder.frames.max(1) as f32;

        sink.lock().server = Some(ServerReport {
            frames: frames.stats(),
            lane: crate::report::LaneReport::new(&stats.lane, real_time.elapsed()),
            time_scale: recorder.time_scale,
            limited_by_server: recorder.limited_by_server as f32 / frame_count,
            limited_by_client: recorder.limited_by_client as f32 / frame_count,
            lag_ticks: recorder.lag,
            skipped_ms: stats.skipped.as_secs_f32() * 1000.,
            late_updates: stats.late_updates,
            clients_joined: recorder.clients_joined,
            drones: drone_q.iter().count(),
        });
    }

    exit.write(AppExit::Success);
}
