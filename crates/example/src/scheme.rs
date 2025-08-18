use bevy::{ecs::entity::MapEntities, prelude::*};
use nevy_prediction::common::scheme::{PredictionScheme, SchemeWorldUpdates};
use serde::{Deserialize, Serialize};

use crate::{networking::StreamHeader, simulation::SimulationPlugin};

pub struct PhysicsScheme;

impl PredictionScheme for PhysicsScheme {
    fn updates() -> SchemeWorldUpdates {
        SchemeWorldUpdates::default().with_message::<NewPhysicsBox>()
    }

    fn message_header() -> impl Into<u16> {
        StreamHeader::Messages
    }

    fn plugin() -> impl Plugin {
        SimulationPlugin
    }
}

#[derive(Serialize, Deserialize, Clone, MapEntities)]
pub struct NewPhysicsBox {
    pub entity: Entity,
}
