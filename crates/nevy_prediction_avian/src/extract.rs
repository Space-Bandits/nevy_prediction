//! Extraction of physics state between simulation worlds.
//!
//! Most components are extracted with [`ExtractSimulationComponentPlugin`].
//! [`RigidBody`] and [`Collider`] have their own extraction, because they are expensive to change:
//! avian moves colliders between trees when a body's type changes and recomputes mass properties when a collider changes.
//! They are only written when they differ from the source, and are removed when the source doesn't have them.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use nevy_prediction::prelude::*;

pub(crate) fn build(app: &mut App) {
    // Snapshots don't have physics plugins, but still need the resource to extract it.
    app.init_resource::<Time<Physics>>();
    app.add_plugins(ExtractSimulationResourcePlugin::<Time<Physics>>::default());

    app.add_plugins((
        ExtractSimulationComponentPlugin::<Position>::default(),
        ExtractSimulationComponentPlugin::<Rotation>::default(),
        ExtractSimulationComponentPlugin::<LinearVelocity>::default(),
        ExtractSimulationComponentPlugin::<AngularVelocity>::default(),
        // Every world could compute these itself, but then bodies have no mass until they do,
        // and app worlds would need the whole mass property pipeline.
        ExtractSimulationComponentPlugin::<ComputedMass>::default(),
        ExtractSimulationComponentPlugin::<ComputedAngularInertia>::default(),
        ExtractSimulationComponentPlugin::<ComputedCenterOfMass>::default(),
    ));

    app.add_plugins((
        ExtractSimulationComponentPlugin::<ConstantForce>::default(),
        ExtractSimulationComponentPlugin::<ConstantTorque>::default(),
        ExtractSimulationComponentPlugin::<ConstantLocalForce>::default(),
        ExtractSimulationComponentPlugin::<ConstantLocalTorque>::default(),
        ExtractSimulationComponentPlugin::<ConstantLinearAcceleration>::default(),
        ExtractSimulationComponentPlugin::<ConstantAngularAcceleration>::default(),
        ExtractSimulationComponentPlugin::<ConstantLocalLinearAcceleration>::default(),
        ExtractSimulationComponentPlugin::<ConstantLocalAngularAcceleration>::default(),
    ));

    app.add_systems(
        ExtractSimulation,
        (extract_rigid_bodies, extract_colliders).in_set(ExtractSimulationSystems::ExtractComponents),
    );
}

fn extract_rigid_bodies(
    mut commands: Commands,
    mut source_world: ResMut<SourceWorld>,
    map: Res<SimulationEntityMap>,
    mut source_q: Local<Option<QueryState<(&SimulationEntity, Option<&RigidBody>)>>>,
    local_q: Query<Option<&RigidBody>>,
) -> Result {
    let source_q = source_q.get_or_insert_with(|| source_world.query());

    for (&simulation_entity, source_body) in source_q.iter(&source_world) {
        let local_entity = local_entity(&map, simulation_entity)?;
        let local_body = local_q.get(local_entity).ok().flatten();

        match (source_body, local_body) {
            (Some(source), Some(local)) if source == local => {}
            (Some(&source), _) => {
                commands.entity(local_entity).insert(source);
            }
            (None, Some(_)) => {
                commands.entity(local_entity).remove::<RigidBody>();
            }
            (None, None) => {}
        }
    }

    Ok(())
}

fn extract_colliders(
    mut commands: Commands,
    mut source_world: ResMut<SourceWorld>,
    map: Res<SimulationEntityMap>,
    mut source_q: Local<Option<QueryState<(&SimulationEntity, Option<&Collider>)>>>,
    local_q: Query<Option<&Collider>>,
) -> Result {
    let source_q = source_q.get_or_insert_with(|| source_world.query());

    for (&simulation_entity, source_collider) in source_q.iter(&source_world) {
        let local_entity = local_entity(&map, simulation_entity)?;
        let local_collider = local_q.get(local_entity).ok().flatten();

        match (source_collider, local_collider) {
            (Some(source), Some(local)) if same_collider(source, local) => {}
            (Some(source), _) => {
                // the shape is behind an `Arc`, so this doesn't copy it
                commands.entity(local_entity).insert(source.clone());
            }
            (None, Some(_)) => {
                commands.entity(local_entity).remove::<Collider>();
            }
            (None, None) => {}
        }
    }

    Ok(())
}

/// Colliders are the same if they share a shape, which they do once one has been extracted from the other.
fn same_collider(a: &Collider, b: &Collider) -> bool {
    Arc::ptr_eq(&a.shape().0, &b.shape().0) && a.scale() == b.scale()
}

fn local_entity(map: &SimulationEntityMap, simulation_entity: SimulationEntity) -> Result<Entity> {
    Ok(map.get(simulation_entity).ok_or(format!(
        "{:?} should exist because this system runs after `ExtractSimulationEntities`",
        simulation_entity
    ))?)
}
