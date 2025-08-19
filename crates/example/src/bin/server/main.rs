use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
};
use example::{
    networking::StreamHeader,
    scheme::{NewPhysicsBox, PhysicsScheme},
    simulation::PhysicsBox,
};
use nevy::*;
use nevy_prediction::server::*;

use crate::{new_pairs::NewPairs, state::JoinedClient};

pub mod networking;
pub mod new_pairs;
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

    app.add_plugins(NevyPredictionServerPlugin::<PhysicsScheme>::new(Update));

    app.add_observer(insert_prediction_clients);

    app.init_resource::<SimulationEntityAllocator>();

    app.add_systems(Startup, spawn_boxes);
    app.add_systems(
        Update,
        replicate_new_boxes.in_set(ServerSimulationSet::QueueUpdates),
    );

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

fn spawn_boxes(mut commands: Commands, mut allocator: ResMut<SimulationEntityAllocator>) {
    commands.spawn((PhysicsBox, allocator.next()));
}

fn replicate_new_boxes(
    pairs: NewPairs<PredictionClient, PhysicsBox>,
    box_q: Query<&SimulationEntity>,
    mut updates: WorldUpdateSender,
    message_id: Res<MessageId<ServerWorldUpdate<NewPhysicsBox>>>,
) -> Result {
    for (client_entity, box_entity) in &pairs {
        let &entity = box_q.get(box_entity)?;

        updates.write(
            StreamHeader::Messages,
            client_entity,
            *message_id,
            NewPhysicsBox { entity },
        )?;
    }

    Ok(())
}
