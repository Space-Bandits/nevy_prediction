use std::marker::PhantomData;

use bevy::{ecs::component::Mutable, prelude::*};

use crate::common::simulation::{
    ExtractSimulationSystems, SourceWorld,
    schedules::ExtractSimulation,
    simulation_entity::{SimulationEntity, SimulationEntityMap},
};

/// System set where a particular component is extracted.
/// You can configure this set to control what order components are extracted in.
pub struct ExtractComponentSystems<C>(pub PhantomData<C>);

impl<C> std::hash::Hash for ExtractComponentSystems<C>
where
    C: 'static,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::any::TypeId::of::<Self>().hash(state);
    }
}

impl<C> bevy::app::DynEq for ExtractComponentSystems<C>
where
    C: std::any::Any + Send + Sync + 'static,
{
    fn dyn_eq(&self, other: &dyn bevy::app::DynEq) -> bool {
        (other as &dyn std::any::Any).is::<Self>()
    }
}

impl<C> SystemSet for ExtractComponentSystems<C>
where
    C: Send + Sync + 'static,
{
    #[doc = r" Clones this `"]
    #[doc = stringify!(SystemSet)]
    #[doc = r"`."]
    fn dyn_clone(&self) -> Box<dyn SystemSet> {
        Box::new(self.clone())
    }
}

impl<C> Default for ExtractComponentSystems<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C> Clone for ExtractComponentSystems<C> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<C> std::fmt::Debug for ExtractComponentSystems<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExtractComponentSystems")
    }
}

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
        app.configure_sets(
            ExtractSimulation,
            ExtractComponentSystems::<C>::default()
                .in_set(ExtractSimulationSystems::ExtractComponents),
        );

        app.add_systems(
            ExtractSimulation,
            extract_component::<C>.in_set(ExtractComponentSystems::<C>::default()),
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

    Ok(())
}
