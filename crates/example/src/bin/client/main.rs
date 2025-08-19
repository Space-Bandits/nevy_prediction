use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
};
use example::scheme::PhysicsScheme;
use nevy_prediction::client::*;

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

    app.add_systems(PostStartup, debug_connect_to_server);

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
