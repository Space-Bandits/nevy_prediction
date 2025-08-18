use bevy::prelude::*;
use nevy_prediction::common::simulation::{ReadyUpdates, SimulationSchedule};

use crate::scheme::NewPhysicsBox;

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(SimulationSchedule, apply_new_boxes);
    }
}

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {}
