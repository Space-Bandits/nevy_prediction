use std::time::Duration;

use bevy::prelude::*;
use nevy_prediction::common::prelude::*;

use crate::networking::StreamHeader;

pub mod player;

pub struct PhysicsScheme;

impl PredictionScheme for PhysicsScheme {
    fn message_header() -> impl Into<u16> {
        StreamHeader::Messages
    }

    fn plugin() -> impl Plugin {
        SimulationPlugin
    }

    fn step_interval() -> Duration {
        Duration::from_secs_f32(1. / 30.)
    }
}

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        player::build(app);
    }
}
