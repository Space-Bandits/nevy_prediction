use bevy::{prelude::*, scene::ScenePlugin};
use nevy_prediction::{
    client::parallel_app::SourceWorld,
    common::simulation::{
        ReadyUpdates, SimulationInstance, SimulationTime, SimulationUpdate,
        extract_component::ExtractSimulationComponentPlugin,
    },
    server::SimulationEntityMap,
};
use serde::{Deserialize, Serialize};

use crate::scheme::UpdateExampleBox;

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        let instance = *app.world().resource::<SimulationInstance>();

        if let SimulationInstance::ClientServerWorld | SimulationInstance::ClientPrediction =
            instance
        {
            app.init_resource::<AppTypeRegistry>();
            app.register_type::<Name>();
            app.register_type::<ChildOf>();
            app.register_type::<Children>();

            app.add_plugins((AssetPlugin::default(), ScenePlugin));
            app.init_asset::<Mesh>();
        }

        app.add_plugins(ExtractSimulationComponentPlugin::<ExampleBox>::default());

        app.add_systems(SimulationUpdate, (apply_update_boxes, move_boxes).chain());

        // app.add_systems(SimulationUpdate, log_simulation_time);
        // app.add_systems(
        //     nevy_prediction::client::parallel_app::ExtractSimulation,
        //     log_extracts,
        // );
        // app.add_systems(
        //     nevy_prediction::common::simulation::ResetSimulation,
        //     log_resets,
        // );
    }
}

#[allow(dead_code)]
fn log_simulation_time(time: Res<Time>, instance: Res<SimulationInstance>) {
    let (SimulationInstance::ClientMain | SimulationInstance::Server) = *instance else {
        return;
    };

    debug!("Update {:?} at {}", *instance, time.elapsed().as_millis());
}

#[allow(dead_code)]
fn log_extracts(
    source_world: Res<SourceWorld>,
    instance: Res<SimulationInstance>,
    time: Res<Time<SimulationTime>>,
) {
    let SimulationInstance::ClientMain = *instance else {
        return;
    };

    debug!(
        "Extracting {:?} -> {:?} time is {}",
        *source_world.resource::<SimulationInstance>(),
        *instance,
        time.elapsed().as_millis()
    );
}

#[allow(dead_code)]
fn log_resets(instance: Res<SimulationInstance>, time: Res<Time<SimulationTime>>) {
    debug!("Reset {:?} time {:?}", *instance, time.elapsed());
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

            debug!("Updated an example box");
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
