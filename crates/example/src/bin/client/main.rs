use std::time::Duration;

use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
};
use example::simulation::PhysicsScheme;
use nevy_prediction::client::prelude::*;

use crate::networking::ClientConnection;

pub mod networking;
pub mod player;

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins.set(LogPlugin {
        level: Level::DEBUG,
        filter: bevy::log::DEFAULT_FILTER.to_string()
            + ",bevy_render=info,bevy_app=info,offset_allocator=info,bevy_asset=info,gilrs=info,bevy_winit=info",
        ..default()
    }));

    example::build(&mut app);

    app.add_plugins(NevyPredictionClientPlugin::<PhysicsScheme>::default());

    networking::build(&mut app);
    player::build(&mut app);

    app.insert_resource(PredictionInterval(Duration::from_millis(1000)));

    app.add_systems(PostStartup, debug_connect_to_server);
    app.add_systems(Startup, setup_camera);

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
        PredictionServerConnection,
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
        Transform::from_xyz(0., 15., 5.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
