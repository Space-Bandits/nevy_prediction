use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
    scene::ScenePlugin,
};
use example::{
    networking::StreamHeader,
    simulation::{ExampleBox, PhysicsScheme, RequestUpdateExampleBox, UpdateExampleBox},
};
use nevy::*;
use nevy_prediction::server::prelude::*;

use crate::{new_pairs::NewPairs, state::JoinedClient};

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

    app.add_systems(Startup, spawn_boxes);
    app.add_systems(
        Update,
        (
            (
                initialize_boxes,
                // refresh_boxes
            )
                .chain(),
            accept_box_updates,
        )
            .in_set(ServerSimulationSystems::QueueUpdatesSystems),
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
    commands.spawn((
        ExampleBox {
            position: 0.,
            velocity: 1.,
        },
        allocator.next(),
    ));
}

fn initialize_boxes(
    pairs: NewPairs<PredictionClient, ExampleBox>,
    box_q: Query<(&SimulationEntity, &ExampleBox)>,
    mut updates: WorldUpdateSender,
    message_id: Res<MessageId<ServerWorldUpdate<UpdateExampleBox>>>,
) -> Result {
    for (client_entity, box_entity) in &pairs {
        let (&entity, example_box) = box_q.get(box_entity)?;

        updates.write_now(
            StreamHeader::Messages,
            client_entity,
            *message_id,
            true,
            UpdateExampleBox {
                entity,
                example_box: example_box.clone(),
            },
        )?;

        debug!("Initialized a new example box");
    }

    Ok(())
}

// const RECONCILE_INTERVAL: Duration = Duration::from_millis(1000);

// /// Periodically generates updates that refresh the state of example boxes.
// ///
// /// This is needed for non-deterministic simulations that would diverge without periodic updates.
// fn refresh_boxes(
//     mut last_update: Local<Duration>,
//     time: Res<Time>,
//     box_q: Query<(&SimulationEntity, &ExampleBox)>,
//     client_q: Query<Entity, With<PredictionClient>>,
//     mut updates: WorldUpdateSender,
//     message_id: Res<MessageId<ServerWorldUpdate<UpdateExampleBox>>>,
// ) -> Result {
//     if time.elapsed() < *last_update + RECONCILE_INTERVAL {
//         return Ok(());
//     }

//     *last_update = time.elapsed();

//     for (&entity, example_box) in &box_q {
//         for client_entity in &client_q {
//             updates.write_now(
//                 StreamHeader::Messages,
//                 client_entity,
//                 *message_id,
//                 false, // not a critical message, so don't waste bandwidth if not needed
//                 UpdateExampleBox {
//                     entity,
//                     example_box: example_box.clone(),
//                 },
//             )?;
//         }
//     }

//     Ok(())
// }

fn accept_box_updates(
    mut update_client_q: Query<(Entity, &mut ReceivedMessages<RequestUpdateExampleBox>)>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut queue: ResMut<WorldUpdateQueue<UpdateExampleBox>>,
    mut sender: WorldUpdateSender,
    message_id: Res<MessageId<ServerWorldUpdate<UpdateExampleBox>>>,
) -> Result {
    for (update_client_entity, mut messages) in &mut update_client_q {
        for RequestUpdateExampleBox { update } in messages.drain() {
            debug!("accepted an update from {}", update_client_entity);

            for client_entity in &client_q {
                sender.write(
                    StreamHeader::Messages,
                    client_entity,
                    *message_id,
                    true,
                    update.clone(),
                )?;
            }

            queue.insert(update);
        }
    }

    Ok(())
}
