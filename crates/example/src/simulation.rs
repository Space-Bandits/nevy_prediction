use avian3d::prelude::*;
use bevy::{prelude::*, scene::ScenePlugin};
use nevy_prediction::{
    client::parallel_app::SourceWorld,
    common::simulation::{
        ReadyUpdates, SimulationInstance, SimulationStartup, SimulationTime, SimulationUpdate,
        extract_component::ExtractSimulationComponentPlugin,
    },
    server::SimulationEntityMap,
};

use crate::scheme::{NewPhysicsBox, UpdatePhysicsBody};

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
            (apply_new_boxes, apply_update_body).chain(),
        );

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
    let SimulationInstance::ClientMain = *instance else {
        return;
    };
    debug!("Update {:?} at {}", *instance, time.elapsed().as_millis());
}

#[allow(dead_code)]
fn log_extracts(source_world: Res<SourceWorld>, instance: Res<SimulationInstance>) {
    debug!(
        "Extracting {:?} -> {:?}",
        *source_world.resource::<SimulationInstance>(),
        *instance
    );
}

#[allow(dead_code)]
fn log_resets(instance: Res<SimulationInstance>, time: Res<Time<SimulationTime>>) {
    debug!("Reset {:?} time {:?}", *instance, time.elapsed());
}

#[derive(Component, Clone)]
#[require(
    RigidBody::Dynamic,
    Transform,
    Position,
    Rotation,
    Collider::cuboid(1., 1., 1.)
)]
pub struct PhysicsBox;

fn apply_new_boxes(mut commands: Commands, mut updates: ReadyUpdates<NewPhysicsBox>) {
    for NewPhysicsBox { entity } in updates.drain() {
        commands.spawn((PhysicsBox, entity));

        debug!("Spawned a new physics box");
    }
}

fn apply_update_body(
    mut updates: ReadyUpdates<UpdatePhysicsBody>,
    map: Res<SimulationEntityMap>,
    mut body_q: Query<(
        &mut Position,
        &mut Rotation,
        &mut LinearVelocity,
        &mut AngularVelocity,
    )>,
) -> Result {
    for UpdatePhysicsBody {
        entity,
        position,
        rotation,
        linear_velocity,
        angular_velocity,
    } in updates.drain()
    {
        let Some(body_entity) = map.get(entity) else {
            error!(
                "Couldn't get simulation entity {:?} to update physics body",
                entity
            );

            continue;
        };

        let (
            mut current_position,
            mut current_rotation,
            mut current_linear_velocity,
            mut current_angular_velocity,
        ) = body_q.get_mut(body_entity)?;

        *current_position = position;
        *current_rotation = rotation;
        *current_linear_velocity = linear_velocity;
        *current_angular_velocity = angular_velocity;
    }

    Ok(())
}

fn spawn_ground_plane(mut commands: Commands) {
    commands.spawn((
        RigidBody::Static,
        Transform::default(),
        Collider::half_space(Vec3::Y),
    ));
}
