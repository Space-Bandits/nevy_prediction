use std::marker::PhantomData;

use bevy::{ecs::component::Mutable, prelude::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::common::{
    scheme::AddWorldUpdate,
    simulation::{
        ReadyUpdates, SimulationUpdate,
        simulation_entity::{SimulationEntity, SimulationEntityMap},
    },
};

/// System set where [`UpdateComponent`] world updates that are added by [`UpdateComponentPlugin`]s are applied.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct UpdateComponentSystems;

/// A utility plugin that adds an [`UpdateComponent<C>`] world update,
/// and the system that applies it during [`UpdateComponentSystems`].
pub struct UpdateComponentPlugin<C>(PhantomData<C>);

impl<C> Default for UpdateComponentPlugin<C> {
    fn default() -> Self {
        UpdateComponentPlugin(PhantomData)
    }
}

impl<C> Plugin for UpdateComponentPlugin<C>
where
    C: Send
        + Sync
        + 'static
        + Serialize
        + DeserializeOwned
        + Clone
        + Component<Mutability = Mutable>,
{
    fn build(&self, app: &mut App) {
        app.add_world_update::<UpdateComponent<C>>();

        app.add_systems(
            SimulationUpdate,
            update_component::<C>.in_set(UpdateComponentSystems),
        );
    }
}

/// This is a world updated added by [`UpdateComponentPlugin<C>`].
///
/// It updates a component on a simulation entity.
/// It will fail if the simulation entity or component does not already exist.
#[derive(Serialize, Deserialize, Clone)]
pub struct UpdateComponent<C> {
    pub entity: SimulationEntity,
    pub component: C,
}

fn update_component<C>(
    mut updates: ReadyUpdates<UpdateComponent<C>>,
    map: Res<SimulationEntityMap>,
    mut query: Query<&mut C>,
) -> Result
where
    C: Send + Sync + 'static + Component<Mutability = Mutable>,
{
    for UpdateComponent { entity, component } in updates.drain() {
        let local_entity = map.get(entity).ok_or(format!(
            "Simulation entity {:?} did not exist locally when attemptiny to update \"{}\"",
            entity,
            std::any::type_name::<C>()
        ))?;

        *query.get_mut(local_entity)? = component;
    }

    Ok(())
}
