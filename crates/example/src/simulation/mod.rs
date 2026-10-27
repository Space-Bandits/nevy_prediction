use std::time::Duration;

use bevy::prelude::*;
use nevy_prediction::prelude::*;

pub mod drones;
pub mod player;
pub mod props;
pub mod workload;

pub struct PhysicsScheme;

impl PredictionScheme for PhysicsScheme {
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
        app.add_plugins(nevy_prediction_avian::NevyPredictionAvianPlugin::default());

        player::build(app);
        props::build(app);
        drones::build(app);
        workload::build(app);
    }
}
