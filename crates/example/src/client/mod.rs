//! The client app.
//!
//! Clients can be windowed and controlled with the keyboard, or headless and controlled by a bot for testing.

use std::{net::SocketAddr, time::Duration};

use bevy::{
    app::ScheduleRunnerPlugin,
    log::{Level, LogPlugin},
    prelude::*,
};
use nevy::prelude::*;
use nevy_prediction::prelude::*;

use crate::{
    report::{
        ClientReport, FrameRecorder, LaneReport, PresentationStats, Range, ReportSink,
        record_frames,
    },
    simulation::{PhysicsScheme, workload::Workload},
};

pub mod debug_ui;
pub mod networking;
pub mod player;

/// Extra costs for each of the client's simulation worlds.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ClientWorkload {
    pub main: Workload,
    pub template: Workload,
    pub prediction: Workload,
}

impl ClientWorkload {
    pub fn all(workload: Workload) -> Self {
        ClientWorkload {
            main: workload,
            template: workload,
            prediction: workload,
        }
    }
}

#[derive(Clone)]
pub struct ClientConfig {
    pub name: String,
    /// Index of this client in a load test, used to tell reports apart.
    pub index: usize,
    pub server_address: SocketAddr,
    /// Open a window and render the game. Otherwise the client is headless.
    pub window: bool,
    /// Control the player with a bot using this seed instead of the keyboard.
    pub bot: Option<u64>,
    pub workload: ClientWorkload,
    /// Time to spin on the main thread every frame, to simulate an expensive frame such as rendering.
    pub frame_cost: Duration,
    /// Frames per second for headless clients.
    pub fps: f64,
    pub execution: SimulationExecution,
    pub prediction_interval: Duration,
    /// Exit after this long.
    pub duration: Option<Duration>,
    pub report: Option<ReportSink>,
    pub log: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            name: "client".to_string(),
            index: 0,
            server_address: "127.0.0.1:27510".parse().unwrap(),
            window: true,
            bot: None,
            workload: default(),
            frame_cost: Duration::ZERO,
            fps: 60.,
            execution: default(),
            prediction_interval: Duration::from_millis(100),
            duration: None,
            report: None,
            log: true,
        }
    }
}

#[derive(Resource, Clone)]
pub struct Config(pub ClientConfig);

pub fn build_client_app(config: ClientConfig) -> App {
    let mut app = App::new();

    let log = LogPlugin {
        level: Level::DEBUG,
        filter: bevy::log::DEFAULT_FILTER.to_string()
            + ",bevy_render=info,bevy_app=info,offset_allocator=info,bevy_asset=info,gilrs=info,bevy_winit=info",
        ..default()
    };

    if config.window {
        let plugins = DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("nevy_prediction example - {}", config.name),
                ..default()
            }),
            ..default()
        });

        if config.log {
            app.add_plugins(plugins.set(log));
        } else {
            app.add_plugins(plugins.disable::<LogPlugin>());
        }
    } else {
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1. / config.fps.max(1.)),
        )));

        if config.log {
            app.add_plugins(log);
        }
    }

    crate::build(&mut app);

    app.add_plugins(
        NevyPredictionClientPlugin::<PhysicsScheme>::new(Update).with_execution(config.execution),
    );
    app.include_protocol::<(), PredictionMessages>();

    // starting point, adapted to network conditions once connected
    app.insert_resource(PredictionInterval(config.prediction_interval));

    let window = config.window;
    app.init_resource::<FrameRecorder>();
    app.init_resource::<ClientRecorder>();
    app.insert_resource(config.workload);
    app.insert_resource(Config(config));

    networking::build(&mut app);
    player::build(&mut app);

    if window {
        debug_ui::build(&mut app);
    }

    app.add_systems(Startup, configure_workload);
    app.add_systems(Update, (spin_frame, apply_workload));
    app.add_systems(
        Last,
        (record_frames, record_client, exit_after_duration).chain(),
    );

    app
}

fn configure_workload(mut workload: ResMut<ClientWorkload>) {
    // marks it as changed so `apply_workload` sends it to the simulation worlds
    workload.set_changed();
}

/// Sends the workload to the simulation worlds whenever it changes.
fn apply_workload(workload: Res<ClientWorkload>, simulation: Res<ClientSimulation>) {
    if !workload.is_changed() {
        return;
    }

    let ClientWorkload {
        main,
        template,
        prediction,
    } = *workload;

    simulation.run_on_main(move |world| {
        world.insert_resource(main);
    });
    simulation.run_on_template(move |world| {
        world.insert_resource(template);
    });
    simulation.run_on_prediction(move |world| {
        world.insert_resource(prediction);
    });
}

fn spin_frame(config: Res<Config>) {
    if config.0.frame_cost > Duration::ZERO {
        let _span = info_span!("spin_frame").entered();
        crate::spin_for(config.0.frame_cost);
    }
}

/// Records what the client's report needs.
#[derive(Resource, Default)]
pub struct ClientRecorder {
    time_scale: Range,
    capacity: Range,
    interval: Range,
    main_lag: Range,
    template_lag: Range,
    pub inputs_sent: u64,
    pub raycasts: u64,
    pub raycast_hits: u64,
    presentation: PresentationStats,
    holds: u64,
    jumps: u64,
    last_presented: Option<Duration>,
}

fn record_client(
    mut recorder: ResMut<ClientRecorder>,
    stats: Res<ClientSimulationStats>,
    interval: Res<PredictionInterval>,
    prediction_time: Res<Time<PredictionTime>>,
    real_time: Res<Time<Real>>,
) {
    if !stats.active {
        return;
    }

    let presented = prediction_time.elapsed();
    if let Some(last) = recorder.last_presented {
        let advanced = presented.saturating_sub(last).as_secs_f32();
        let expected = real_time.delta_secs() * prediction_time.time_scale();

        if expected > 0. {
            recorder.presentation.frames += 1;

            if advanced < expected * 0.5 {
                recorder.holds += 1;
            } else if advanced > expected * 1.5 {
                recorder.jumps += 1;
            }
        }

        recorder.presentation.fraction.record(prediction_time.fraction());
        let latest = prediction_time.latest_tick().time::<PhysicsScheme>();
        recorder
            .presentation
            .behind_latest_ms
            .record(latest.saturating_sub(presented).as_secs_f32() * 1000.);
    }
    recorder.last_presented = Some(presented);

    recorder.time_scale.record(stats.time_scale);
    if let Some(capacity) = stats.capacity {
        recorder.capacity.record(capacity);
    }
    recorder
        .interval
        .record(interval.as_secs_f32() * 1000.);
    recorder.main_lag.record(stats.main_lag as f32);
    recorder.template_lag.record(stats.template_lag as f32);
}

fn exit_after_duration(
    config: Res<Config>,
    real_time: Res<Time<Real>>,
    recorder: Res<ClientRecorder>,
    frames: Res<FrameRecorder>,
    stats: Res<ClientSimulationStats>,
    interval_stats: Res<PredictionIntervalStats>,
    drone_q: Query<(), With<crate::simulation::drones::Drone>>,
    prop_q: Query<(), With<crate::simulation::props::Prop>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(duration) = config.0.duration else {
        return;
    };

    if real_time.elapsed() < duration {
        return;
    }

    if let Some(sink) = &config.0.report {
        let wall = real_time.elapsed();

        let report = ClientReport {
            name: config.0.name.clone(),
            index: config.0.index,
            connected: stats.active,
            frames: frames.stats(),
            main: LaneReport::new(&stats.main, wall),
            template: LaneReport::new(&stats.template, wall),
            prediction: LaneReport::new(&stats.prediction, wall),
            time_scale: recorder.time_scale,
            capacity: recorder.capacity,
            interval_ms: recorder.interval,
            main_lag: recorder.main_lag,
            template_lag: recorder.template_lag,
            predictions_completed: stats.predictions_completed,
            prediction_length: stats.prediction_length,
            late_probes: interval_stats.late_probes,
            inputs_sent: recorder.inputs_sent,
            raycast_hits: recorder.raycast_hits as f32 / recorder.raycasts.max(1) as f32,
            drones: drone_q.iter().count(),
            props: prop_q.iter().count(),
            presentation: PresentationStats {
                holds: recorder.holds as f32 / recorder.presentation.frames.max(1) as f32,
                jumps: recorder.jumps as f32 / recorder.presentation.frames.max(1) as f32,
                ..recorder.presentation.clone()
            },
            errors: default(),
        };

        sink.lock().clients.push(report);
    }

    exit.write(AppExit::Success);
}
