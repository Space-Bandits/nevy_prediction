use std::time::Duration;

use bevy::prelude::*;
use nevy_prediction::common::prelude::*;
use serde::{Deserialize, Serialize};

use crate::networking::StreamHeader;

// pub mod player;

pub struct PhysicsScheme;

impl PredictionScheme for PhysicsScheme {
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

/// World update that spawns or updates an example box.
#[derive(Serialize, Deserialize, Clone)]
pub struct UpdateExampleBox {
    pub entity: SimulationEntity,
    pub example_box: ExampleBox,
}

/// Client -> Server message to request an update to an example box.
#[derive(Serialize, Deserialize)]
pub struct RequestUpdateExampleBox {
    pub update: WorldUpdate<UpdateExampleBox>,
}

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        // player::build(app);

        app.add_world_update::<UpdateExampleBox>();

        app.add_plugins(ExtractSimulationComponentPlugin::<ExampleBox>::default());

        app.add_systems(SimulationUpdate, (apply_update_boxes, move_boxes).chain());
    }
}

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct ExampleBox {
    pub position: f32,
    pub velocity: f32,
}

fn apply_update_boxes(
    mut commands: Commands,
    map: Res<SimulationEntityMap>,
    mut box_q: Query<&mut ExampleBox>,
    mut updates: ReadyUpdates<UpdateExampleBox>,
) {
    for UpdateExampleBox {
        entity,
        example_box,
    } in updates.drain()
    {
        if let Some(box_entity) = map.get(entity) {
            let Ok(mut current_box) = box_q.get_mut(box_entity) else {
                warn!(
                    "Tried apply an example box update to a simulation entity that isn't an example box"
                );

                continue;
            };

            *current_box = example_box;
        } else {
            commands.spawn((example_box, entity));

            debug!("Spawned a new physics box");
        }
    }
}

fn move_boxes(time: Res<Time>, mut box_q: Query<&mut ExampleBox>) {
    for mut example_box in &mut box_q {
        example_box.position += example_box.velocity * time.delta_secs();
    }
}
