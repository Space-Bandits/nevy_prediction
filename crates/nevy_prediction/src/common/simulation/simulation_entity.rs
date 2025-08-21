use bevy::{platform::collections::HashMap, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{
    client::parallel_app::{ExtractSimulation, SourceWorld},
    common::simulation::ResetSimulation,
};

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractSimulationEntities;

pub fn build(app: &mut App) {
    app.init_resource::<SimulationEntityMap>();

    app.add_observer(add_simulation_entity);
    app.add_observer(remove_simulation_entity);

    app.add_systems(
        ExtractSimulation,
        (
            mark_removed_simulation_entities,
            extract_simulation_entities,
            despawn_removed_simulation_entities,
        )
            .chain()
            .in_set(ExtractSimulationEntities),
    );

    app.add_systems(ResetSimulation, reset_simulation_entities);
}

/// This component is a unique id that can be used to map entities across all instances of the simulation.
///
/// Use this component in your world updates and when extracting data between simulation instances.
///
/// Entities with this component will be automatically spawned and despawned
/// during the [ExtractSimulation] schedule in the [ExtractSimulationEntities] system set.
/// You can then utilize the [SimulationEntityMap] from the current world to find which [SimulationEntity]
/// in the current world belongs to a [SimulationEntity] in the [SourceWorld].
///
/// Entities with this component will be despawned when the [ResetSimulation] schedule runs,
/// so if you have many types of entities with this component you don't have to create a system that despawns
/// each of them.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[component(immutable)]
pub struct SimulationEntity(pub u64);

/// This component is updated using lifecycle hooks of [SimulationEntity] to track which [Entity]
/// in the local world belongs to a [SimulationEntity].
#[derive(Resource, Default)]
pub struct SimulationEntityMap {
    /// Map of simulation entity -> local entity
    map: HashMap<SimulationEntity, Entity>,
}

impl SimulationEntityMap {
    pub fn get(&self, id: SimulationEntity) -> Option<Entity> {
        self.map.get(&id).copied()
    }
}

fn add_simulation_entity(
    trigger: Trigger<OnInsert, SimulationEntity>,
    entity_q: Query<&SimulationEntity>,
    mut map: ResMut<SimulationEntityMap>,
) -> Result {
    let local_entity = trigger.target();
    let &simulation_entity = entity_q.get(local_entity)?;

    map.map.insert(simulation_entity, local_entity);

    Ok(())
}

fn remove_simulation_entity(
    trigger: Trigger<OnReplace, SimulationEntity>,
    entity_q: Query<&SimulationEntity>,
    mut map: ResMut<SimulationEntityMap>,
) -> Result {
    let local_entity = trigger.target();
    let &simulation_entity = entity_q.get(local_entity)?;

    map.map.remove(&simulation_entity);

    Ok(())
}

#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct RemovedSimulationEntity;

fn mark_removed_simulation_entities(
    mut commands: Commands,
    entity_q: Query<Entity, With<SimulationEntity>>,
) {
    for entity in &entity_q {
        commands.entity(entity).insert(RemovedSimulationEntity);
    }
}

fn extract_simulation_entities(
    mut commands: Commands,
    map: Res<SimulationEntityMap>,
    mut entity_q: Local<Option<QueryState<&SimulationEntity>>>,
    mut source_world: ResMut<SourceWorld>,
) {
    let entity_q = entity_q.get_or_insert_with(|| source_world.query());

    for &simulation_entity in entity_q.iter(&*source_world) {
        if let Some(local_entity) = map.get(simulation_entity) {
            commands
                .entity(local_entity)
                .remove::<RemovedSimulationEntity>();
        } else {
            commands.spawn(simulation_entity);
        }
    }
}

fn despawn_removed_simulation_entities(
    mut commands: Commands,
    entity_q: Query<Entity, With<RemovedSimulationEntity>>,
) {
    for entity in &entity_q {
        commands.entity(entity).despawn();
    }
}

fn reset_simulation_entities(
    mut commands: Commands,
    entity_q: Query<Entity, With<SimulationEntity>>,
) {
    for entity in &entity_q {
        commands.entity(entity).despawn();
    }
}
