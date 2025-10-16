use std::marker::PhantomData;

use bevy::{
    ecs::component::{Mutable, Tick},
    prelude::*,
};

use crate::common::simulation::{
    ExtractSimulationSystems, SourceWorld,
    schedules::ExtractSimulation,
    simulation_entity::{SimulationEntity, SimulationEntityMap},
};

/// This plugin is a utility to automatically extract components on [`SimulationEntity`]s.
///
/// It will add the component to the local entity if it doesn't exist but it will not remove it if it is removed from the [`SourceWorld`].
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
            extract_component::<C>.in_set(ExtractSimulationSystems::ExtractComponents),
        );
    }
}

fn extract_component<C>(
    mut commands: Commands,
    mut source_world: ResMut<SourceWorld>,
    map: Res<SimulationEntityMap>,
    mut source_component_q: Local<Option<QueryState<(&SimulationEntity, Ref<C>)>>>,
    mut last_run: Local<Tick>,
    mut local_component_q: Query<&mut C>,
) -> Result
where
    C: Component<Mutability = Mutable> + Clone,
{
    let this_run = source_world.change_tick();

    let new_component_q = source_component_q.get_or_insert_with(|| source_world.query_filtered());

    for (&simulation_entity, source_component) in new_component_q.iter(&mut *source_world) {
        if !source_component
            .last_changed()
            .is_newer_than(*last_run, this_run)
        {
            continue;
        }

        let local_entity = map.get(simulation_entity).ok_or(format!(
            "{:?} should exist because this system runs after `ExtractSimulationEntities`",
            simulation_entity
        ))?;

        if let Ok(mut local_component) = local_component_q.get_mut(local_entity) {
            *local_component = source_component.clone();
        } else {
            commands
                .entity(local_entity)
                .insert(source_component.clone());
        }
    }

    *last_run = this_run;

    Ok(())
}
