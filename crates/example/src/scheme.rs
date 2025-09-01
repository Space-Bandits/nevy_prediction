use std::time::Duration;

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
        SchemeWorldUpdates::default().with_message::<NewPhysicsBox>()
    }

    fn message_header() -> impl Into<u16> {
        StreamHeader::Messages
    }

    fn plugin() -> impl Plugin {
        SimulationPlugin
    }

    fn step_interval() -> Duration {
        Duration::from_secs_f32(1. / 20.)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NewPhysicsBox {
    pub entity: SimulationEntity,
}
