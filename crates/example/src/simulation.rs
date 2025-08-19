use bevy::prelude::*;
use nevy_prediction::common::simulation::{ReadyUpdates, SimulationSchedule};

use crate::scheme::NewPhysicsBox;

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            SimulationSchedule,
            (log_simulation_time, apply_new_boxes).chain(),
        );
    }
}

fn log_simulation_time(time: Res<Time>) {
    debug!("Simulation time: {}", time.elapsed().as_millis());
}

#[derive(Component)]
pub struct PhysicsBox;

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {
    for NewPhysicsBox { entity } in updates.read() {
        commands.spawn((PhysicsBox, entity));

        debug!("Spawned a new physics box");
    }
}
