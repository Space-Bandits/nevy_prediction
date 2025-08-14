use bevy::prelude::*;
use nevy_prediction::common::*;
use serde::{Deserialize, Serialize};

use crate::networking::StreamHeader;

pub mod networking;

/// common logic for the server and client apps
pub fn build(app: &mut App) {
    networking::build(app);
}

pub struct PhysicsScheme;

impl PredictionScheme for PhysicsScheme {
    fn updates() -> SchemeWorldUpdates<Self> {
        SchemeWorldUpdates::default().with_message::<NewPhysicsBox>()
    }

    fn message_header() -> impl Into<u16> {
        StreamHeader::Messages
    }
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

#[derive(Serialize, Deserialize)]
pub struct NewPhysicsBox {
    pub entity: ServerEntity,
}
