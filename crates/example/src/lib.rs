use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub mod networking;
pub mod scheme;
pub mod simulation;

/// common logic for the server and client apps
pub fn build(app: &mut App) {
    networking::build(app);

    // app.add_plugins(plugins)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerEntity(Entity);

impl From<ServerEntity> for Entity {
    fn from(value: ServerEntity) -> Self {
        value.0
    }
}

impl From<Entity> for ServerEntity {
    fn from(value: Entity) -> Self {
        ServerEntity(value)
    }
}
