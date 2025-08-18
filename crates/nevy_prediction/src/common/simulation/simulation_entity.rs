use bevy::prelude::*;

/// This component exists on an entity that entity that belongs to the server's simulation instance.
#[derive(Component)]
pub struct SimulationEntity(Entity);

#[derive(Resource)]
pub(crate) enum SimulationEntityMap {}
