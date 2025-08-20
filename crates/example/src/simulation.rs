use bevy::prelude::*;
use nevy_prediction::{
    client::parallel_app::{ExtractSimulation, SourceWorld},
    common::simulation::{
        ReadyUpdates, SimulationInstance, SimulationSchedule,
        simulation_entity::ExtractSimulationEntities,
    },
    server::{SimulationEntity, SimulationEntityMap},
};

use crate::scheme::NewPhysicsBox;

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            SimulationSchedule,
            (log_simulation_time, apply_new_boxes).chain(),
        );

        app.add_systems(
            ExtractSimulation,
            (log_extracts, extract_boxes.after(ExtractSimulationEntities)),
        );
    }
}

fn log_simulation_time(time: Res<Time>, instance: Res<SimulationInstance>) {
    debug!(
        "simulation: {:?} time: {}",
        *instance,
        time.elapsed().as_millis()
    );
}

fn log_extracts(source_world: Res<SourceWorld>, instance: Res<SimulationInstance>) {
    debug!(
        "Extracting {:?} -> {:?}",
        *source_world.resource::<SimulationInstance>(),
        *instance
    );
}

#[derive(Component)]
pub struct PhysicsBox;

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {
    for NewPhysicsBox { entity } in updates.read() {
        commands.spawn((PhysicsBox, entity));

        debug!("Spawned a new physics box");
    }
}

fn extract_boxes(
    mut commands: Commands,
    map: Res<SimulationEntityMap>,
    mut source_world: ResMut<SourceWorld>,
    mut box_q: Local<Option<QueryState<&SimulationEntity, With<PhysicsBox>>>>,
) {
    let box_q = box_q.get_or_insert_with(|| source_world.query_filtered());

    for &simulation_entity in box_q.iter(&*source_world) {
        commands
            .entity(map.get(simulation_entity).unwrap())
            .insert(PhysicsBox);
    }
}
