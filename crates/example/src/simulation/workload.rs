//! An artificial per-tick cost, used to force machines to fall behind so that the simulation has to slow down.
//!
//! The [`Workload`] resource lives in each simulation world and isn't extracted,
//! so every world can be given a different cost.
//! The client inserts it with [`ClientSimulation::run_on_main`](nevy_prediction::prelude::ClientSimulation::run_on_main)
//! and friends, and the server inserts it into its own world, which is the server's simulation.

use std::time::Duration;

use bevy::prelude::*;
use nevy_prediction::prelude::*;
use serde::{Deserialize, Serialize};

pub fn build(app: &mut App) {
    app.init_resource::<Workload>();

    app.add_systems(SimulationUpdate, spin_workload);
}

/// Makes every tick of a simulation world take extra time.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Workload {
    /// Time to spin for every tick, on the simulation thread.
    pub tick_cost: Duration,
    /// Probability (0..1) that a tick has an extra spike.
    pub spike_chance: f32,
    /// Extra time to spin for a spike.
    pub spike_cost: Duration,
    /// Time to spin for every drone every tick, spread across the compute task pool.
    pub drone_cost: Duration,
}

impl Workload {
    pub fn tick(tick_cost: Duration) -> Self {
        Workload {
            tick_cost,
            ..default()
        }
    }

    pub fn is_idle(&self) -> bool {
        *self == Workload::default()
    }
}

/// A small deterministic random number generator, so that workloads don't need a dependency.
pub fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^= x >> 33;
    x
}

/// A random number from 0 to 1.
pub fn unit(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

fn spin_workload(
    workload: Res<Workload>,
    time: Res<Time<SimulationTime>>,
    instance: Res<SimulationInstance>,
) {
    let mut cost = workload.tick_cost;

    // Spikes are random but the same for a tick in every world of the same instance.
    let seed = (*time.current_tick() as u64) << 8 | *instance as u64;
    if workload.spike_chance > 0. && unit(seed) < workload.spike_chance {
        cost += workload.spike_cost;
    }

    if cost > Duration::ZERO {
        let _span = info_span!("spin_workload", ?cost).entered();
        crate::spin_for(cost);
    }
}
