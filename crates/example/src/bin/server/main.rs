use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
    scene::ScenePlugin,
};
use example::simulation::PhysicsScheme;
use nevy_prediction::server::prelude::*;

use crate::state::JoinedClient;

pub mod networking;
pub mod new_pairs;
pub mod player;
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

    app.add_plugins((AssetPlugin::default(), ScenePlugin));
    app.init_asset::<Mesh>();

    example::build(&mut app);

    app.add_plugins(NevyPredictionServerPlugin::<PhysicsScheme>::new(Update));

    networking::build(&mut app);
    state::build(&mut app);
    player::build(&mut app);

    app.add_observer(insert_prediction_clients);

    app.init_resource::<SimulationEntityAllocator>();

    app.run();
}

fn insert_prediction_clients(trigger: Trigger<OnAdd, JoinedClient>, mut commands: Commands) {
    commands.entity(trigger.target()).insert(PredictionClient);
}

#[derive(Resource, Default)]
pub struct SimulationEntityAllocator {
    next_id: u64,
}

impl SimulationEntityAllocator {
    pub fn next(&mut self) -> SimulationEntity {
        let id = self.next_id;
        self.next_id += 1;
        SimulationEntity(id)
    }
}
