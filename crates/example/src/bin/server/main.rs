use std::time::Duration;

use bevy::{
    log::{Level, LogPlugin},
    prelude::*,
    scene::ScenePlugin,
};
use example::{
    networking::StreamHeader,
    scheme::{NewPhysicsBox, PhysicsScheme},
    simulation::PhysicsBox,
};
use nevy_prediction::{common::simulation::UpdateQueue, server::*};

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

    app.add_plugins((AssetPlugin::default(), ScenePlugin));
    app.init_asset::<Mesh>();

    example::build(&mut app);

    networking::build(&mut app);
    state::build(&mut app);

    app.add_plugins(NevyPredictionServerPlugin::<PhysicsScheme>::new(Update));

    app.add_observer(insert_prediction_clients);

    app.init_resource::<SimulationEntityAllocator>();

    app.add_systems(Startup, spawn_boxes);
    app.add_systems(
        Update,
        (
            (
                replicate_new_boxes,
                initialize_physics_bodies,
                reconcile_physics_bodies,
            )
                .chain(),
            accept_body_updates,
        )
            .in_set(ServerSimulationSet::QueueUpdates),
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
        PhysicsBox,
        allocator.next(),
        ReplicatePhysicsBody,
        Position(Vec3::new(0., 3., 0.)),
    ));
}

fn replicate_new_boxes(
    pairs: NewPairs<PredictionClient, PhysicsBox>,
    box_q: Query<&SimulationEntity>,
    mut updates: WorldUpdateSender<NewPhysicsBox>,
) -> Result {
    for (client_entity, box_entity) in &pairs {
        let &entity = box_q.get(box_entity)?;

        updates.write_now(
            StreamHeader::Messages,
            client_entity,
            NewPhysicsBox { entity },
        )?;
    }

    Ok(())
}

#[derive(Component)]
pub struct ReplicatePhysicsBody;

fn initialize_physics_bodies(
    pairs: NewPairs<PredictionClient, ReplicatePhysicsBody>,
    body_q: Query<(
        &SimulationEntity,
        &Position,
        &Rotation,
        &LinearVelocity,
        &AngularVelocity,
    )>,
    mut updates: WorldUpdateSender<UpdatePhysicsBody>,
) -> Result {
    for (client_entity, body_entity) in &pairs {
        let (&entity, &position, &rotation, &linear_velocity, &angular_velocity) =
            body_q.get(body_entity)?;

        updates.write_now(
            StreamHeader::Messages,
            client_entity,
            UpdatePhysicsBody {
                entity,
                position,
                rotation,
                linear_velocity,
                angular_velocity,
            },
        )?;
    }

    Ok(())
}

const RECINCILE_BODIES_INTERVAL: Duration = Duration::from_millis(1000);

fn reconcile_physics_bodies(
    mut last_update: Local<Duration>,
    time: Res<Time>,
    body_q: Query<(
        &SimulationEntity,
        &Position,
        &Rotation,
        &LinearVelocity,
        &AngularVelocity,
    )>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut updates: WorldUpdateSender<UpdatePhysicsBody>,
) -> Result {
    if time.elapsed() < *last_update + RECINCILE_BODIES_INTERVAL {
        return Ok(());
    }

    *last_update = time.elapsed();

    for (&entity, &position, &rotation, &linear_velocity, &angular_velocity) in &body_q {
        for client_entity in &client_q {
            updates.write_now(
                StreamHeader::Messages,
                client_entity,
                UpdatePhysicsBody {
                    entity,
                    position,
                    rotation,
                    linear_velocity,
                    angular_velocity,
                },
            )?;
        }
    }

    Ok(())
}

fn accept_body_updates(
    mut updates: UpdateRequests<UpdatePhysicsBody>,
    mut queue: ResMut<UpdateQueue<UpdatePhysicsBody>>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut sender: WorldUpdateSender<UpdatePhysicsBody>,
) -> Result {
    for (client_entity, update) in updates.drain() {
        debug!("client {} updated a physics body", client_entity);

        for client_entity in &client_q {
            sender.write(StreamHeader::Messages, client_entity, update.clone())?;
        }

        queue.insert(update);
    }

    Ok(())
}
