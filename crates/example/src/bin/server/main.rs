use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
};
use example::{NewPhysicsBox, PhysicsScheme};
use nevy::*;
use nevy_prediction::server::*;

use crate::state::initialize_pairs::InitializePairs;

pub mod networking;
pub mod state;

fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins);
    app.add_plugins(LogPlugin {
        level: Level::DEBUG,
        filter: bevy::log::DEFAULT_FILTER.to_string()
            + ",bevy_render=info,bevy_app=info,offset_allocator=info,bevy_asset=info,gilrs=info,bevy_winit=info",
        ..default()
    });

    example::build(&mut app);

    networking::build(&mut app);
    state::build(&mut app);

    app.add_plugins(NevyPredictionServerPlugin::<PhysicsScheme>::default());

    app.add_systems(Startup, spawn_boxes);

    app.run();
}

#[derive(Component)]
struct PhysicsBox;

fn spawn_boxes(mut commands: Commands) {
    commands.spawn(PhysicsBox);
}
