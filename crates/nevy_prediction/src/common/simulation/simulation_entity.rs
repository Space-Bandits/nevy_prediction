use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    platform::collections::HashMap,
    prelude::*,
};
use log::error;
use serde::{Deserialize, Serialize};

use crate::common::{
    scheme::AddWorldUpdate,
    simulation::{
        ExtractSimulation, ExtractSimulationSystems, ReadyUpdates, SimulationUpdate, SourceWorld,
        schedules::ResetSimulation,
    },
};

/// System set where [`SimulationEntity`]s are despawned by [`DespawnSimulatonEntity`] updates during [`SimulationUpdate`].
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DespawnSimulationEntities;

pub fn build(app: &mut App) {
    app.init_resource::<SimulationEntityMap>();

    app.add_systems(
        ExtractSimulation,
        (
            (
                mark_removed_simulation_entities,
                extract_simulation_entities,
            )
                .chain()
                .in_set(ExtractSimulationSystems::ExtractSimulationEntities),
            despawn_removed_simulation_entities
                .in_set(ExtractSimulationSystems::DespawnSimulationEntities),
        ),
    );

    app.add_systems(ResetSimulation, reset_simulation_entities);

    app.add_systems(
        SimulationUpdate,
        apply_despawn_simulation_entities.in_set(DespawnSimulationEntities),
    );

    app.add_world_update::<DespawnSimulatonEntity>();
}

/// This component is a unique id that can be used to map entities across instances of the simulation.
///
/// Use this component in your world updates and when extracting data between simulation instances to identify an entity.
///
/// Entities with this component will be automatically spawned and despawned
/// during the [`ExtractSimulation`] schedule in the [`ExtractSimulationSystems::ExtractSimulationEntities`] and [`ExtractSimulationSystems::DespawnSimulationEntities`] system sets.
/// If you order an extract system to run after this system set you can use the [`SimulationEntity`] on an entity
/// in the [`SourceWorld`] to identify its corresponding entity in the local world with the local [`SimulationEntityMap`].
///
/// Entities with this component will be despawned when the [ResetSimulation] schedule runs,
/// so if you have many types of entities with this component you don't have to create a system that despawns
/// each of them.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[component(immutable, on_insert = Self::on_insert, on_replace = Self::on_replace)]
pub struct SimulationEntity(pub u64);

impl std::fmt::Display for SimulationEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Simulation({})", self.0)
    }
}

/// This resource is updated using the lifecycle hooks of [SimulationEntity] to track which [Entity]
/// in the local world belongs to a [SimulationEntity].
#[derive(Resource, Default)]
pub struct SimulationEntityMap {
    /// Map of simulation entity -> local entity
    map: HashMap<SimulationEntity, Entity>,
}

impl SimulationEntityMap {
    /// Gets the local entity corresponding to the given simulation entity if it exists.
    pub fn get(&self, id: SimulationEntity) -> Option<Entity> {
        self.map.get(&id).copied()
    }
}

impl SimulationEntity {
    fn on_insert(mut world: DeferredWorld, ctx: HookContext) {
        let &simulation_entity = world.get::<Self>(ctx.entity).unwrap();

        if let Some(previous_entity) = world
            .resource_mut::<SimulationEntityMap>()
            .map
            .insert(simulation_entity, ctx.entity)
        {
            error!(
                "Simulation entity {:?} was inserted on {}, but already existed on {}.",
                simulation_entity, ctx.entity, previous_entity
            );
        }
    }

    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let &simulation_entity = world.get::<Self>(ctx.entity).unwrap();

        world
            .resource_mut::<SimulationEntityMap>()
            .map
            .remove(&simulation_entity);
    }
}

/// Lower priority entities are despawned first.
#[derive(Component, Default, Deref, Clone, Copy)]
pub struct ExtractDespawnPriority(pub i32);

/// Marker component used to determine which simulation entities but no longer exist in the source world.
#[derive(Component)]
#[component(storage = "SparseSet")]
struct RemovedSimulationEntity;

/// Marks every simulation entity in the local world with a [`RemovedSimulationEntity`] component.
fn mark_removed_simulation_entities(
    mut commands: Commands,
    entity_q: Query<Entity, With<SimulationEntity>>,
) {
    for entity in &entity_q {
        commands.entity(entity).insert(RemovedSimulationEntity);
    }
}

/// Extracts simulation entities from the source world and spawns them in the local world.
///
/// If a simulation entity doesn't exist in the local world, it is spawned.
///
/// If a simulation entity exists in the local world, the [`RemovedSimulationEntity`] component is removed.
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

/// Despawns any entities that don't have a corresponding simulation entity in the source world, as determined by [`extract_simulation_entities`].
fn despawn_removed_simulation_entities(
    mut commands: Commands,
    entity_q: Query<(Entity, Option<&ExtractDespawnPriority>), With<RemovedSimulationEntity>>,
) {
    let mut despawn_order: Vec<(i32, Entity)> = Vec::new();

    for (entity, priority) in &entity_q {
        let priority = **priority.unwrap_or(&default());

        let (Ok(index) | Err(index)) =
            despawn_order.binary_search_by(|&(probe, _)| probe.cmp(&priority));

        despawn_order.insert(index, (priority, entity));
    }

    for (_, entity) in despawn_order {
        // In situations where we are despawning entities in hierarchies, we may despawn parent entities first.
        // Use `try_despawn` to avoid warnings in these situations.
        commands.entity(entity).try_despawn();
    }
}

/// Despawns all simulation entities when the simulation is reset.
fn reset_simulation_entities(
    mut commands: Commands,
    entity_q: Query<Entity, With<SimulationEntity>>,
) {
    for entity in &entity_q {
        commands.entity(entity).despawn();
    }
}

/// A world update that despawns a simulation entity.
///
/// This world update is added by default.
#[derive(Serialize, Deserialize, Clone)]
pub struct DespawnSimulatonEntity {
    pub entity: SimulationEntity,
}

fn apply_despawn_simulation_entities(
    mut commands: Commands,
    mut updates: ReadyUpdates<DespawnSimulatonEntity>,
    map: Res<SimulationEntityMap>,
) -> Result {
    for DespawnSimulatonEntity { entity } in updates.drain() {
        let local_entity = map.get(entity).ok_or(format!(
            "Simulation entity {:?} did not exist locally when trying to despawn it.",
            entity
        ))?;

        commands.entity(local_entity).despawn();
    }

    Ok(())
}
