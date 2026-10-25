//! Collects measurements from apps for automated testing and profiling.
//!
//! Apps given a [`ReportSink`] record their frame times and simulation statistics,
//! and write a report into the sink when they exit.
//!
//! The server and clients also record the position of every player for every tick that is extracted into their app,
//! so that what each client showed for a tick can be compared with what the server simulated for it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use bevy::prelude::*;
use nevy_prediction::prelude::*;
use serde::Serialize;

/// Shared between every app in a load test.
#[derive(Clone, Default, Resource)]
pub struct ReportSink(pub Arc<Mutex<Reports>>);

impl ReportSink {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Reports> {
        self.0.lock().unwrap()
    }
}

#[derive(Default)]
pub struct Reports {
    pub server: Option<ServerReport>,
    pub clients: Vec<ClientReport>,
    pub truth: TruthLog,
}

/// Player positions recorded by the server and clients, keyed by tick.
#[derive(Default)]
pub struct TruthLog {
    server: HashMap<(SimulationEntity, SimulationTick), Vec2>,
    /// The latest position each client had for each entity and tick.
    samples: HashMap<(usize, SimulationEntity, SimulationTick), ClientSample>,
}

struct ClientSample {
    local: bool,
    position: Vec2,
}

impl TruthLog {
    pub fn record_server(&mut self, entity: SimulationEntity, tick: SimulationTick, position: Vec2) {
        self.server.insert((entity, tick), position);
    }

    pub fn record_client(
        &mut self,
        client: usize,
        local: bool,
        entity: SimulationEntity,
        tick: SimulationTick,
        position: Vec2,
    ) {
        self.samples
            .insert((client, entity, tick), ClientSample { local, position });
    }

    /// Compares every client sample with the server's position for the same entity and tick.
    pub fn errors(&self, client: usize) -> PredictionErrors {
        let mut local = Vec::new();
        let mut remote = Vec::new();

        for (&(_, entity, tick), sample) in self
            .samples
            .iter()
            .filter(|&(&(sample_client, ..), _)| sample_client == client)
        {
            let Some(truth) = self.server.get(&(entity, tick)) else {
                continue;
            };

            let error = sample.position.distance(*truth);

            if sample.local {
                local.push(error);
            } else {
                remote.push(error);
            }
        }

        PredictionErrors {
            local: ErrorStats::new(local),
            remote: ErrorStats::new(remote),
        }
    }
}

/// How far a client's player positions were from the server's for the same tick.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PredictionErrors {
    /// The client's own player, which should match the server unless its inputs arrived late.
    pub local: ErrorStats,
    /// Other players, which mispredict until their inputs reach this client.
    pub remote: ErrorStats,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ErrorStats {
    pub samples: usize,
    /// Fraction of samples with an error above 1mm.
    pub mispredicted: f32,
    pub mean: f32,
    pub p99: f32,
    pub max: f32,
}

impl ErrorStats {
    fn new(mut errors: Vec<f32>) -> Self {
        if errors.is_empty() {
            return default();
        }

        errors.sort_by(f32::total_cmp);

        ErrorStats {
            samples: errors.len(),
            mispredicted: errors.iter().filter(|&&error| error > 0.001).count() as f32
                / errors.len() as f32,
            mean: errors.iter().sum::<f32>() / errors.len() as f32,
            p99: percentile(&errors, 0.99),
            max: *errors.last().unwrap(),
        }
    }
}

fn percentile(sorted: &[f32], percentile: f32) -> f32 {
    sorted[((sorted.len() - 1) as f32 * percentile).round() as usize]
}

/// Statistics about how long app frames took.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FrameStats {
    pub frames: usize,
    pub mean_ms: f32,
    pub p50_ms: f32,
    pub p99_ms: f32,
    pub max_ms: f32,
}

/// Records the duration of every frame.
#[derive(Resource, Default)]
pub struct FrameRecorder {
    frames: Vec<f32>,
    /// The first frames are slow while everything starts up.
    skip: u32,
}

impl FrameRecorder {
    pub fn stats(&self) -> FrameStats {
        let mut frames = self.frames.clone();

        if frames.is_empty() {
            return default();
        }

        frames.sort_by(f32::total_cmp);

        FrameStats {
            frames: frames.len(),
            mean_ms: frames.iter().sum::<f32>() / frames.len() as f32,
            p50_ms: percentile(&frames, 0.5),
            p99_ms: percentile(&frames, 0.99),
            max_ms: *frames.last().unwrap(),
        }
    }
}

pub fn record_frames(mut recorder: ResMut<FrameRecorder>, time: Res<Time<Real>>) {
    if recorder.skip < 30 {
        recorder.skip += 1;
        return;
    }

    recorder.frames.push(time.delta_secs() * 1000.);
}

/// Statistics about a simulation lane.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LaneReport {
    pub ticks: u64,
    pub tick_cost_ms: Option<f32>,
    /// Fraction of wall time the lane spent working.
    pub utilization: f32,
}

impl LaneReport {
    pub fn new(stats: &LaneStats, wall: Duration) -> Self {
        LaneReport {
            ticks: stats.ticks,
            tick_cost_ms: stats.tick_cost.map(|cost| cost.as_secs_f32() * 1000.),
            utilization: stats.busy.as_secs_f32() / wall.as_secs_f32().max(f32::EPSILON),
        }
    }
}

/// Tracks the minimum and mean of a value over time.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Range {
    pub min: f32,
    pub mean: f32,
    pub max: f32,
    pub last: f32,
    #[serde(skip)]
    sum: f64,
    #[serde(skip)]
    count: u64,
}

impl Range {
    pub fn record(&mut self, value: f32) {
        if self.count == 0 {
            self.min = value;
            self.max = value;
        }

        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.last = value;
        self.sum += value as f64;
        self.count += 1;
        self.mean = (self.sum / self.count as f64) as f32;
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ServerReport {
    pub frames: FrameStats,
    pub lane: LaneReport,
    pub time_scale: Range,
    /// Fraction of the time the time scale was limited by the server or a client.
    pub limited_by_server: f32,
    pub limited_by_client: f32,
    pub lag_ticks: Range,
    pub skipped_ms: f32,
    /// Client updates that arrived after the server executed their tick.
    pub late_updates: u64,
    pub clients_joined: u32,
    /// Drones in the app's world at the end.
    pub drones: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ClientReport {
    pub name: String,
    pub index: usize,
    /// True if the client was reset by the server.
    pub connected: bool,
    pub frames: FrameStats,
    pub main: LaneReport,
    pub template: LaneReport,
    pub prediction: LaneReport,
    pub time_scale: Range,
    pub capacity: Range,
    pub interval_ms: Range,
    pub main_lag: Range,
    pub template_lag: Range,
    pub predictions_completed: u64,
    pub prediction_length: Option<f32>,
    pub late_probes: u64,
    pub inputs_sent: u64,
    /// Drones in the app's world at the end.
    pub drones: usize,
    pub presentation: PresentationStats,
    pub errors: PredictionErrors,
}

/// How smoothly [`PredictionTime`](nevy_prediction::prelude::PredictionTime) advanced.
///
/// Every frame it should advance by the real time that passed multiplied by the time scale.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PresentationStats {
    pub frames: u64,
    /// Fraction of frames where presented time advanced less than half as much as it should have.
    pub holds: f32,
    /// Fraction of frames where presented time advanced more than one and a half times as much as it should have.
    pub jumps: f32,
    /// Average interpolation fraction between the latest two ticks.
    pub fraction: Range,
    /// How far presented time trailed the latest simulation state, in milliseconds.
    pub behind_latest_ms: Range,
}
