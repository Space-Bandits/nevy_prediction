use avian3d::prelude::*;
use bevy::{prelude::*, scene::ScenePlugin};
use nevy_prediction::{
    client::parallel_app::{ExtractSimulation, SourceWorld},
    common::simulation::{
        ReadyUpdates, ResetSimulation, SimulationInstance, SimulationStartup, SimulationTime,
        SimulationUpdate, extract_component::ExtractSimulationComponentPlugin,
    },
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

        app.add_plugins(PhysicsPlugins::new(SimulationUpdate));

        app.add_plugins(ExtractSimulationComponentPlugin::<PhysicsBox>::default());
        app.add_plugins(ExtractSimulationComponentPlugin::<Position>::default());
        app.add_plugins(ExtractSimulationComponentPlugin::<Rotation>::default());

        app.add_systems(SimulationStartup, spawn_ground_plane);

        app.add_systems(
            SimulationUpdate,
            (log_simulation_time, apply_new_boxes).chain(),
        );

        app.add_systems(ExtractSimulation, (log_extracts));

        app.add_systems(ResetSimulation, log_resets);
    }
}

fn log_simulation_time(time: Res<Time>, instance: Res<SimulationInstance>) {
    debug!("Update {:?} at {}", *instance, time.elapsed().as_millis());
}

fn log_extracts(source_world: Res<SourceWorld>, instance: Res<SimulationInstance>) {
    debug!(
        "Extracting {:?} -> {:?}",
        *source_world.resource::<SimulationInstance>(),
        *instance
    );
}

fn log_resets(instance: Res<SimulationInstance>, time: Res<Time<SimulationTime>>) {
    debug!("Reset {:?} time {:?}", *instance, time.elapsed());
}

#[derive(Component, Clone)]
#[require(
    RigidBody::Static,
    Transform,
    Position,
    Rotation,
    Collider::cuboid(1., 1., 1.)
)]
pub struct PhysicsBox;

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {
    for NewPhysicsBox { entity } in updates.read() {
        commands.spawn((PhysicsBox, entity));

        debug!("Spawned a new physics box");
    }
}

fn spawn_ground_plane(mut commands: Commands) {
    commands.spawn((
        RigidBody::Static,
        Transform::default(),
        Collider::half_space(Vec3::Y),
    ));
}
