use bevy::{platform::collections::HashMap, prelude::*};
use serde::{Deserialize, Serialize};

pub fn build(app: &mut App) {
    app.init_resource::<SimulationEntityMap>();

    app.add_observer(add_simulation_entity);
    app.add_observer(remove_simulation_entity);
}

/// This component exists on an entity that entity that belongs to the server's simulation instance.
/// [SimulationEntityMap] can be used to find which entity has a given id.
///
/// This component is provided as a utility for having a consistent id for simulation entities across all instances of a simulation.
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
