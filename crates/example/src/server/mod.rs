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
        props::{Prop, PropState, SpawnProps, Wall},
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
    /// Physics props bouncing around an arena.
    pub props: u32,
    /// How often bodies are corrected, see `RigidBodySyncSettings::interval`.
    pub body_sync_interval: Duration,
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
            props: 0,
            body_sync_interval: Duration::from_millis(250),
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
    app.add_plugins(nevy_prediction_avian::prelude::RigidBodySyncPlugin);
    app.insert_resource(nevy_prediction_avian::prelude::RigidBodySyncSettings {
        interval: config.body_sync_interval,
        ..default()
    });

    networking::build(&mut app);
    player::build(&mut app);

    app.init_resource::<SimulationEntityAllocator>();
    app.init_resource::<FrameRecorder>();
    app.init_resource::<ServerRecorder>();
    app.insert_resource(Config(config));

    app.add_systems(Startup, (configure_workload, spawn_drones, spawn_props));
    app.add_systems(
        Update,
        (init_drones, init_props)
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

fn spawn_props(
    config: Res<Config>,
    mut allocator: ResMut<SimulationEntityAllocator>,
    mut queue: ServerUpdateQueue<SpawnProps>,
) {
    if config.0.props == 0 {
        return;
    }

    let spawn = SpawnProps::random(|| allocator.next(), config.0.props, 1);
    queue.queue(WorldUpdate {
        tick: SimulationTick(0),
        update: spawn,
    });
}

/// Sends existing props to new clients.
///
/// Their exact state is also sent by `RigidBodySyncPlugin`, but this has to spawn them first.
fn init_props(
    client_q: Query<Entity, Added<PredictionClient>>,
    wall_q: Query<&SimulationEntity, With<Wall>>,
    prop_q: Query<(
        &SimulationEntity,
        &Prop,
        &avian3d::prelude::Position,
        &avian3d::prelude::Rotation,
        &avian3d::prelude::LinearVelocity,
        &avian3d::prelude::AngularVelocity,
    )>,
    mut updates: WorldUpdateSender,
) -> Result {
    if client_q.is_empty() {
        return Ok(());
    }

    let walls: Vec<_> = wall_q.iter().copied().collect();
    let Ok(walls) = <[SimulationEntity; 6]>::try_from(walls) else {
        return Ok(());
    };

    let props: Vec<_> = prop_q
        .iter()
        .map(
            |(&entity, &prop, position, rotation, linear_velocity, angular_velocity)| PropState {
                entity,
                prop,
                position: position.0,
                rotation: rotation.0,
                linear_velocity: linear_velocity.0,
                angular_velocity: angular_velocity.0,
            },
        )
        .collect();

    for client_entity in &client_q {
        updates.write_now(
            client_entity,
            true,
            SpawnProps {
                walls: Some(walls),
                props: props.clone(),
            },
        )?;
    }

    Ok(())
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
    prop_q: Query<(&SimulationEntity, &avian3d::prelude::Position), With<Prop>>,
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
        for (&entity, position) in &prop_q {
            reports.truth.record_server_prop(entity, tick, position.0);
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
    prop_q: Query<&avian3d::prelude::Position, With<Prop>>,
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
            props: prop_q.iter().count(),
            props_escaped: prop_q
                .iter()
                .filter(|position| {
                    position.abs().max_element() > crate::simulation::props::ARENA_HALF_SIZE + 0.5
                })
                .count(),
        });
    }

    exit.write(AppExit::Success);
}
