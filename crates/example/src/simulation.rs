use bevy::{prelude::*, scene::ScenePlugin};
use nevy_prediction::{
    client::parallel_app::{ExtractSimulation, SourceWorld},
    common::simulation::{
        ReadyUpdates, SimulationInstance, SimulationStartup, SimulationTime, SimulationUpdate,
        extract_component::ExtractSimulationComponentPlugin,
    },
    server::SimulationEntityMap,
};

use crate::scheme::NewPhysicsBox;

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

        app.add_plugins(ExtractSimulationComponentPlugin::<PhysicsBox>::default());

        app.add_systems(SimulationUpdate, apply_new_boxes);

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

#[derive(Component, Clone)]
#[require(Transform)]
pub struct PhysicsBox;

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {
    for NewPhysicsBox { entity } in updates.drain() {
        commands.spawn((PhysicsBox, entity));

        debug!("Spawned a new physics box");
    }
}
