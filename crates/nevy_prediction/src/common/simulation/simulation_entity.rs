use bevy::{ecs::entity::EntityHashMap, prelude::*};

pub fn build(app: &mut App) {
    app.add_observer(add_simulation_entity);
    app.add_observer(remove_simulation_entity);
}

/// This component exists on an entity that entity that belongs to the server's simulation instance.
#[derive(Component, Deref)]
pub struct LocalSimulationEntity(pub Entity);

#[derive(Resource)]
pub(crate) struct SimulationEntityMap {
    /// Map of simulation entity -> local entity
    map: EntityHashMap<Entity>,
}

fn add_simulation_entity(
    trigger: Trigger<OnInsert, LocalSimulationEntity>,
    entity_q: Query<&LocalSimulationEntity>,
    mut map: ResMut<SimulationEntityMap>,
) -> Result {
    let local_entity = trigger.target();
    let simulation_entity = entity_q.get(local_entity)?.0;

    map.map.insert(simulation_entity, local_entity);

    Ok(())
}

fn remove_simulation_entity(
    trigger: Trigger<OnReplace, LocalSimulationEntity>,
    entity_q: Query<&LocalSimulationEntity>,
    mut map: ResMut<SimulationEntityMap>,
) -> Result {
    let local_entity = trigger.target();
    let simulation_entity = entity_q.get(local_entity)?.0;

    map.map.remove(&simulation_entity);

    Ok(())
}
