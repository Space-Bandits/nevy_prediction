use bevy::prelude::*;
use nevy_prediction::common::simulation::ReadyUpdates;

use crate::scheme::NewPhysicsBox;

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {}
}

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {}
