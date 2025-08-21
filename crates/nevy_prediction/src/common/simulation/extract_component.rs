use std::marker::PhantomData;

use bevy::{ecs::component::Mutable, prelude::*};

use crate::{
    client::parallel_app::{ExtractSimulation, SourceWorld},
    common::simulation::simulation_entity::ExtractSimulationEntities,
    server::{SimulationEntity, SimulationEntityMap},
};

/// This plugin acts as a utility to automatically extract a component on a [SimulationEntity].
pub struct ExtractSimulationComponentPlugin<C>(PhantomData<C>);

impl<C> Default for ExtractSimulationComponentPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C> Plugin for ExtractSimulationComponentPlugin<C>
where
    C: Send + Sync + 'static + Component<Mutability = Mutable> + Clone,
{
    fn build(&self, app: &mut App) {
        app.add_systems(
            ExtractSimulation,
            extract_component::<C>.after(ExtractSimulationEntities),
        );
    }
}

fn extract_component<C>(
    mut commands: Commands,
    mut source_world: ResMut<SourceWorld>,
    map: Res<SimulationEntityMap>,
    mut source_component_q: Local<Option<QueryState<(&SimulationEntity, &C)>>>,
    mut local_component_q: Query<&mut C>,
) -> Result
where
    C: Component<Mutability = Mutable> + Clone,
{
    let new_component_q = source_component_q.get_or_insert_with(|| source_world.query_filtered());

    for (&simulation_entity, source_component) in new_component_q.iter(&mut *source_world) {
        let entity = map.get(simulation_entity).ok_or("Simulation entity should exist because this system runs after `ExtractSimulationEntities`")?;

        if let Ok(mut local_component) = local_component_q.get_mut(entity) {
            *local_component = source_component.clone();
        } else {
            commands.entity(entity).insert(source_component.clone());
        }
    }

    Ok(())
}
