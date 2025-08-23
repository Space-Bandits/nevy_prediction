use avian3d::prelude::*;
use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
};
use example::{
    scheme::{PhysicsScheme, UpdatePhysicsBody},
    simulation::PhysicsBox,
};
use nevy_prediction::{client::*, server::SimulationEntity};

use crate::networking::ClientConnection;

pub mod networking;

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins.set(LogPlugin {
        level: Level::DEBUG,
        filter: bevy::log::DEFAULT_FILTER.to_string()
            + ",bevy_render=info,bevy_app=info,offset_allocator=info,bevy_asset=info,gilrs=info,bevy_winit=info",
        ..default()
    }));

    example::build(&mut app);
    networking::build(&mut app);

    app.add_plugins(NevyPredictionClientPlugin::<PhysicsScheme>::default());

    app.add_plugins(PhysicsDebugPlugin::new(PostUpdate));

    app.add_systems(PostStartup, debug_connect_to_server);
    app.add_systems(Startup, setup_camera);
    app.add_systems(
        Update,
        simulation_input.in_set(ClientSimulationSet::QueueUpdates),
    );

    app.run();
}

fn debug_connect_to_server(
    mut commands: Commands,
    endpoint_q: Query<Entity, With<networking::ClientEndpoint>>,
) -> Result {
    let endpoint_entity = endpoint_q.single()?;

    let address = std::env::args()
        .nth(1)
        .expect("Expected server address as first argument")
        .parse()
        .expect("Invalid server address");

    commands.spawn((
        ClientConnection,
        nevy::ConnectionOf(endpoint_entity),
        nevy::QuicConnectionConfig {
            client_config: networking::create_connection_config(),
            address,
            server_name: "example.server".to_string(),
        },
    ));

    Ok(())
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-10., 10., 10.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn simulation_input(
    input: Res<ButtonInput<KeyCode>>,
    box_q: Query<&SimulationEntity, With<PhysicsBox>>,
    mut sender: PredictionUpdateSender<UpdatePhysicsBody>,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    for &entity in &box_q {
        sender.write(UpdatePhysicsBody {
            entity,
            position: Position(Vec3::new(0., 3., -1.)),
            rotation: default(),
            linear_velocity: LinearVelocity(Vec3::new(0., -5., 0.)),
            angular_velocity: AngularVelocity(Vec3::new(1., 1., 1.)),
        });

        debug!("Sent input for {:?}", entity);
    }
}
