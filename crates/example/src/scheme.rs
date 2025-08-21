use std::time::Duration;

use avian3d::prelude::*;
use bevy::prelude::*;
use nevy_prediction::common::{
    scheme::{PredictionScheme, SchemeWorldUpdates},
    simulation::simulation_entity::SimulationEntity,
};
use serde::{Deserialize, Serialize};

use crate::{networking::StreamHeader, simulation::SimulationPlugin};

pub struct PhysicsScheme;

impl PredictionScheme for PhysicsScheme {
    fn updates() -> SchemeWorldUpdates {
        SchemeWorldUpdates::default()
            .with_message::<NewPhysicsBox>()
            .with_message::<UpdatePhysicsBody>()
    }

    fn message_header() -> impl Into<u16> {
        StreamHeader::Messages
    }

    fn plugin() -> impl Plugin {
        SimulationPlugin
    }

    fn step_interval() -> Duration {
        Duration::from_millis(50)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NewPhysicsBox {
    pub entity: SimulationEntity,
}

/// Updates the state of a physics body.
///
/// Expects that the [SimulationEntity] already exists and has the needed components.
#[derive(Serialize, Deserialize, Clone)]
pub struct UpdatePhysicsBody {
    pub entity: SimulationEntity,
    pub position: Position,
    pub rotation: Rotation,
    pub linear_velocity: LinearVelocity,
    pub angular_velocity: AngularVelocity,
}
