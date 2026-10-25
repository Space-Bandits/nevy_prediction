//! Drones are simulation entities that fly in circles, to add entities for the simulation and extraction to process.
//!
//! Their motion only depends on the simulation time, so they never mispredict.

use bevy::prelude::*;
use nevy_prediction::prelude::*;
use serde::{Deserialize, Serialize};

use crate::simulation::workload::{Workload, unit};

pub fn build(app: &mut App) {
    app.add_world_update::<SpawnDrones>();

    app.add_plugins(ExtractSimulationComponentPlugin::<Drone>::default());
    app.add_plugins(ExtractSimulationComponentPlugin::<DroneState>::default());

    app.add_systems(
        SimulationUpdate,
        (
            spawn_drones.before(UpdateComponentSystems),
            move_drones.after(UpdateComponentSystems),
        ),
    );
}

#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
#[require(DroneState)]
pub struct Drone {
    pub seed: u64,
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct DroneState {
    pub position: Vec2,
}

/// A world update that spawns drones.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpawnDrones {
    pub drones: Vec<(SimulationEntity, Drone)>,
}

fn spawn_drones(mut commands: Commands, mut updates: ReadyUpdates<SpawnDrones>) {
    for SpawnDrones { drones } in updates.drain() {
        commands.spawn_batch(drones);
    }
}

fn move_drones(
    mut drone_q: Query<(&Drone, &mut DroneState)>,
    time: Res<Time<SimulationTime>>,
    workload: Res<Workload>,
) {
    let elapsed = time.elapsed_secs();
    let drone_cost = workload.drone_cost;

    drone_q.par_iter_mut().for_each(|(drone, mut state)| {
        let radius = 2. + 8. * unit(drone.seed);
        let speed = 0.2 + unit(drone.seed + 1);
        let phase = unit(drone.seed + 2) * std::f32::consts::TAU;
        let angle = phase + elapsed * speed;

        state.position = Vec2::new(angle.cos(), angle.sin()) * radius;

        if drone_cost > std::time::Duration::ZERO {
            crate::spin_for(drone_cost);
        }
    });
}
