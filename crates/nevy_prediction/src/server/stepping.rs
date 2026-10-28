//! Steps the server's simulation in the app's world.
//!
//! The authoritative [`Server`](crate::common::simulation::SimulationInstance::Server) simulation
//! is the server app's own world. It is stepped in [`SyncSimulationSystems`](crate::common::simulation::SyncSimulationSystems)
//! for the ticks that the server's clock has passed, up to
//! [`max_ticks_per_frame`](crate::server::NevyPredictionServerPlugin::max_ticks_per_frame) every frame.
//! The clock runs at the server's time scale, which is lowered when the server or a client can't keep up.

use std::time::Duration;

use bevy::{platform::time::Instant, prelude::*};
use tracing::warn;

use crate::common::{
    clock::SimulationClock,
    runtime::{LaneMeter, LaneStats},
    scheme::PredictionScheme,
    simulation::{SimulationTick, SimulationTime, SimulationTimeExt, world::step_world},
};

/// Statistics about the server's simulation, updated every frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct ServerSimulationStats {
    /// How many simulation seconds pass per real second.
    pub time_scale: f32,
    /// The fastest time scale the server estimates it could sustain.
    pub capacity: Option<f32>,
    pub lane: LaneStats,
    /// How many ticks the server's simulation was behind its clock after it last stepped.
    pub lag: u32,
    /// Simulation time that was skipped because the server fell too far behind.
    pub skipped: Duration,
    /// Updates that were queued for a tick that had already been executed, so were applied late.
    pub late_updates: u64,
}

/// Decides when the server's simulation ticks.
#[derive(Resource)]
pub(crate) struct ServerSimulation {
    /// False until the app first steps the simulation, so that the clock starts when the app does.
    active: bool,
    clock: SimulationClock,
    meter: LaneMeter,
    /// The most ticks the server will fall behind its clock before it skips time instead.
    max_catch_up: u32,
    max_ticks_per_frame: u32,
    /// Simulation time that was skipped because the server fell too far behind.
    skipped: Duration,
    lag: u32,
    step: Duration,
}

impl ServerSimulation {
    pub fn new<S>(max_catch_up: Duration, max_ticks_per_frame: u32) -> Self
    where
        S: PredictionScheme,
    {
        let step = S::step_interval();

        ServerSimulation {
            active: false,
            clock: SimulationClock::new(Instant::now(), Duration::ZERO, 1.),
            meter: LaneMeter::default(),
            max_catch_up: (max_catch_up.as_secs_f64() / step.as_secs_f64()).ceil() as u32,
            max_ticks_per_frame: max_ticks_per_frame.max(1),
            skipped: Duration::ZERO,
            lag: 0,
            step,
        }
    }

    pub fn clock(&self) -> SimulationClock {
        self.clock
    }

    pub fn set_time_scale(&mut self, now: Instant, time_scale: f32) {
        self.clock.rescale(now, time_scale);
    }

    /// The fastest time scale the server could sustain, based on how long its ticks take.
    pub fn capacity(&self) -> Option<f32> {
        let cost = self.meter.cost.average()?;
        Some(self.step.as_secs_f32() / cost.as_secs_f32())
    }

    pub fn stats(&self) -> ServerSimulationStats {
        ServerSimulationStats {
            time_scale: self.clock.scale(),
            capacity: self.capacity(),
            lane: self.meter.stats(),
            lag: self.lag,
            skipped: self.skipped,
            late_updates: 0,
        }
    }

    /// Decides which tick the simulation should be stepped up to this frame, given the next tick it will execute.
    fn due(&mut self, tick: SimulationTick, now: Instant) -> SimulationTick {
        if !self.active {
            self.active = true;
            self.clock = SimulationClock::new(now, self.step * *tick, self.clock.scale());
        }

        let due = self.clock.tick_at(now, self.step);

        if *due > *tick + self.max_catch_up {
            // Fell too far behind to catch up, skip time so that only the next tick is due.
            let skip = self.step * (*due - *tick - 1);
            warn!(
                "The server simulation fell {} ticks behind, skipping {:?} of simulation time",
                *due - *tick,
                skip
            );

            self.skipped += skip;
            self.clock = SimulationClock::new(now, self.step * (*tick + 1), self.clock.scale());

            return SimulationTick(*tick + 1);
        }

        due
    }
}

/// Executes the ticks that the server's clock has passed, up to the most it may execute in a frame.
pub(crate) fn step_simulation<S>(world: &mut World)
where
    S: PredictionScheme,
{
    world.resource_scope(|world, mut simulation: Mut<ServerSimulation>| {
        let tick = world.resource::<Time<SimulationTime>>().current_tick();
        let due = simulation.due(tick, Instant::now());

        let end = (*due).min(*tick + simulation.max_ticks_per_frame);

        for _ in *tick..end {
            let start = Instant::now();
            step_world::<S>(world);
            simulation.meter.record_tick(start.elapsed());
        }

        let tick = world.resource::<Time<SimulationTime>>().current_tick();
        simulation.lag = simulation
            .clock
            .tick_at(Instant::now(), simulation.step)
            .saturating_sub(*tick);
    });
}
