//! Slows the simulation down when the server or a client can't keep up.
//!
//! The server measures how long its own ticks take, and every client reports the fastest time scale it can sustain.
//! The server's time scale moves towards the slowest of these, and is sent to clients along with the server's tick.
//!
//! Slowing down the simulation keeps everyone in sync at the cost of the simulation running in slow motion,
//! which is preferable to clients falling behind the server or the server skipping time.

use std::time::Duration;

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    platform::time::Instant,
    prelude::*,
};
use nevy::prelude::*;

use crate::{
    common::{SimulationCapacity, simulation::SyncSimulationSystems},
    server::{PredictionClient, ServerSimulationSystems, stepping::ServerSimulation},
};

pub(crate) fn build(app: &mut App, schedule: Interned<dyn ScheduleLabel>) {
    app.init_resource::<TimeScaleSettings>();
    app.init_resource::<ServerTimeScale>();

    app.add_systems(
        schedule,
        (
            receive_capacities.in_set(ServerSimulationSystems::QueueUpdates),
            update_time_scale.in_set(SyncSimulationSystems),
        ),
    );
}

/// Controls how the server slows down the simulation.
#[derive(Resource, Clone, Debug)]
pub struct TimeScaleSettings {
    /// If false the simulation always runs in real time, and machines that can't keep up fall behind.
    pub enabled: bool,
    /// The slowest the simulation will run.
    pub min: f32,
    /// The fraction of each machine's capacity that the simulation is allowed to use.
    ///
    /// Leaving some headroom avoids running right at the limit where small spikes would make a machine fall behind.
    pub headroom: f32,
    /// How fast the time scale may decrease, per second.
    pub decrease_rate: f32,
    /// How fast the time scale may increase, per second.
    pub increase_rate: f32,
    /// Client capacity reports older than this are ignored.
    pub capacity_timeout: Duration,
}

impl Default for TimeScaleSettings {
    fn default() -> Self {
        TimeScaleSettings {
            enabled: true,
            min: 0.1,
            headroom: 0.85,
            decrease_rate: 0.5,
            increase_rate: 0.1,
            capacity_timeout: Duration::from_secs(2),
        }
    }
}

/// The fastest time scale a client reported it could sustain.
#[derive(Component, Clone, Copy, Debug)]
pub struct ClientSimulationCapacity {
    pub time_scale: f32,
    /// Real time when this was received.
    pub received: Duration,
}

/// What is limiting the server's time scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeScaleLimit {
    #[default]
    None,
    /// The server can't execute ticks fast enough.
    Server,
    /// A client can't execute ticks fast enough.
    Client(Entity),
}

/// The server's time scale and what it's moving towards.
#[derive(Resource, Clone, Debug)]
pub struct ServerTimeScale {
    pub current: f32,
    pub target: f32,
    pub limit: TimeScaleLimit,
}

impl Default for ServerTimeScale {
    fn default() -> Self {
        ServerTimeScale {
            current: 1.,
            target: 1.,
            limit: TimeScaleLimit::None,
        }
    }
}

fn receive_capacities(
    mut commands: Commands,
    mut client_q: Query<(Entity, &mut ReceivedMessages<SimulationCapacity>), With<PredictionClient>>,
    real_time: Res<Time<Real>>,
) {
    for (client_entity, mut messages) in &mut client_q {
        if let Some(SimulationCapacity { time_scale }) = messages.drain().last() {
            commands
                .entity(client_entity)
                .insert(ClientSimulationCapacity {
                    time_scale,
                    received: real_time.elapsed(),
                });
        }
    }
}

pub(crate) fn update_time_scale(
    settings: Res<TimeScaleSettings>,
    mut time_scale: ResMut<ServerTimeScale>,
    mut simulation: ResMut<ServerSimulation>,
    client_q: Query<(Entity, &ClientSimulationCapacity), With<PredictionClient>>,
    real_time: Res<Time<Real>>,
) {
    let mut target = 1.;
    let mut limit = TimeScaleLimit::None;

    if settings.enabled {
        let mut consider = |capacity: f32, source: TimeScaleLimit| {
            let capacity = capacity * settings.headroom;

            if capacity < target {
                target = capacity;
                limit = source;
            }
        };

        if let Some(capacity) = simulation.capacity() {
            consider(capacity, TimeScaleLimit::Server);
        }

        let now = real_time.elapsed();
        for (client_entity, capacity) in &client_q {
            if now.saturating_sub(capacity.received) <= settings.capacity_timeout {
                consider(capacity.time_scale, TimeScaleLimit::Client(client_entity));
            }
        }

        target = target.clamp(settings.min.min(1.), 1.);
    }

    let delta = real_time.delta_secs();
    let current = time_scale.current;
    let new = if target < current {
        (current - settings.decrease_rate * delta).max(target)
    } else {
        (current + settings.increase_rate * delta).min(target)
    };

    time_scale.target = target;
    time_scale.limit = limit;

    if new != current {
        time_scale.current = new;
        simulation.set_time_scale(Instant::now(), new);
    }
}
