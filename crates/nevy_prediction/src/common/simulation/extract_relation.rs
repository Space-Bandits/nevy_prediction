use std::marker::PhantomData;

use bevy::{ecs::relationship::Relationship, prelude::*};

use crate::common::simulation::{
    ExtractSimulationSystems, SourceWorld,
    schedules::ExtractSimulation,
    simulation_entity::{SimulationEntity, SimulationEntityMap},
};

/// System set where a particular relation component is extracted.
/// You can configure this set to control what order relation components are extracted in.
pub struct ExtractRelationSystems<C>(pub PhantomData<C>);

impl<C> std::hash::Hash for ExtractRelationSystems<C>
where
    C: 'static,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::any::TypeId::of::<Self>().hash(state);
    }
}

impl<C> bevy::app::DynEq for ExtractRelationSystems<C>
where
    C: std::any::Any + Send + Sync + 'static,
{
    fn dyn_eq(&self, other: &dyn bevy::app::DynEq) -> bool {
        (other as &dyn std::any::Any).is::<Self>()
    }
}

impl<C> SystemSet for ExtractRelationSystems<C>
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

impl<C> Default for ExtractRelationSystems<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C> Clone for ExtractRelationSystems<C> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<C> std::fmt::Debug for ExtractRelationSystems<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExtractComponentSystems")
    }
}

/// This plugin extracts [`Relationship`] [`Component`]s between [`SimulationEntity`]s.
///
/// This plugin expects all simulation entities that have a relation be related to another simulation entity.
pub struct ExtractSimulationRelationPlugin<C>(PhantomData<C>);

impl<C> Default for ExtractSimulationRelationPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C> Plugin for ExtractSimulationRelationPlugin<C>
where
    C: Component + Relationship,
{
    fn build(&self, app: &mut App) {
        app.configure_sets(
            ExtractSimulation,
            ExtractRelationSystems::<C>::default()
                .in_set(ExtractSimulationSystems::ExtractComponents),
        );

        app.add_systems(
            ExtractSimulation,
            extract_relation::<C>.in_set(ExtractRelationSystems::<C>::default()),
        );
    }
}

fn extract_relation<C>(
    mut commands: Commands,
    local_relation_q: Query<&C, With<SimulationEntity>>,
    mut source_world: ResMut<SourceWorld>,
    map: Res<SimulationEntityMap>,
    mut source_relation_q: Local<Option<QueryState<(&SimulationEntity, Option<&C>)>>>,
    mut source_target_q: Local<Option<QueryState<&SimulationEntity>>>,
) -> Result
where
    C: Component + Relationship,
{
    let source_relation_q = source_relation_q.get_or_insert_with(|| source_world.query_filtered());
    let source_target_q = source_target_q.get_or_insert_with(|| source_world.query_filtered());

    for (&relation_simulation_entity, relation) in source_relation_q.iter(source_world.as_ref()) {
        let local_relation_entity = map.get(relation_simulation_entity).ok_or(format!(
            "Couldn't get local relation entity {} when extracting its `{}`",
            relation_simulation_entity,
            std::any::type_name::<C>()
        ))?;

        let Some(relation) = relation else {
            if local_relation_q.contains(local_relation_entity) {
                commands.entity(local_relation_entity).remove::<C>();
            }

            continue;
        };

        let source_target_entity = relation.get();

        let &target_simulation_entity =
            source_target_q.get(source_world.as_ref(), source_target_entity)?;

        let local_target_entity = map.get(target_simulation_entity).ok_or(format!(
            "Couldn't get local target entity {} of relation entity {} when extracting its `{}`",
            target_simulation_entity,
            relation_simulation_entity,
            std::any::type_name::<C>()
        ))?;

        // Don't insert if already relation of correct target.
        if let Ok(current_relation) = local_relation_q.get(local_relation_entity) {
            if current_relation.get() == local_target_entity {
                continue;
            }
        }

        commands
            .entity(local_relation_entity)
            .insert(C::from(local_target_entity));
    }

    Ok(())
}
