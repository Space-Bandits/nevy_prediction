//! This module is responsible for receiving [`WorldUpdate`]s from the server
//! and applying them to a separate instance of the simulation in the [`ServerWorld`] resource.
//!
//! This local copy of the server's simulation is then used to predict the future state of the simulation.

use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::{
    client::{
        ClientSimulationSystems, PredictionServerConnection, parallel_app::ParallelWorld,
        prediction::PredictionInterval,
    },
    common::{
        ServerWorldUpdate, UpdateServerTime,
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationPlugin, SimulationStartup, SimulationTime,
            SimulationTimeTarget, WorldUpdateQueue,
        },
    },
};

pub(crate) fn build<S>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    S: PredictionScheme,
{
    app.insert_non_send_resource(ServerWorld::new::<S>());
    app.init_resource::<ServerWorldTime>();

    app.add_systems(
        schedule,
        (
            poll_server_world.in_set(ClientSimulationSystems::PollParallelAppsSystems),
            receive_time_updates.in_set(ClientSimulationSystems::ReceiveTimeSystems),
            run_server_world.in_set(ClientSimulationSystems::RunParallelAppsSystems),
        ),
    );
}

pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static,
{
    app.add_systems(schedule, receive_world_updates::<T>);
}

/// Contains a [`ParallelWorld`] that contains the most recently known state of the simulation according to the server.
#[derive(Deref, DerefMut)]
pub(crate) struct ServerWorld(pub ParallelWorld);

impl ServerWorld {
    pub fn new<S>() -> Self
    where
        S: PredictionScheme,
    {
        let mut app = App::empty();

        app.add_schedule(Schedule::new(Main));
        app.main_mut().update_schedule = Some(Main.intern());

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: Main.intern(),
            instance: SimulationInstance::ClientServerWorld,
        });

        app.world_mut().run_schedule(SimulationStartup);

        ServerWorld(ParallelWorld::new(app))
    }
}

fn poll_server_world(mut server_world_app: NonSendMut<ServerWorld>) {
    server_world_app.poll();
}

fn receive_world_updates<T>(
    mut server_world: NonSendMut<ServerWorld>,
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<ServerWorldUpdate<T>>,
        Has<PredictionServerConnection>,
    )>,
) where
    T: Send + Sync + 'static,
{
    let Some(world) = server_world.get() else {
        return;
    };

    for (connection_entity, mut messages, is_server) in &mut message_q {
        for ServerWorldUpdate { update } in messages.drain() {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            world.resource_mut::<WorldUpdateQueue<T>>().insert(update);
        }
    }
}

/// Contains the most recent server time update.
#[derive(Resource, Default, Deref, DerefMut)]
struct ServerWorldTime(pub Duration);

fn receive_time_updates(
    mut message_q: Query<&mut ReceivedMessages<UpdateServerTime>>,
    mut time: ResMut<ServerWorldTime>,
    mut time_target: ResMut<SimulationTimeTarget>,
    prediction_interval: Res<PredictionInterval>,
) -> Result {
    for mut messages in &mut message_q {
        for UpdateServerTime { simulation_time } in messages.drain() {
            **time = simulation_time;

            let desired_target = simulation_time + **prediction_interval;
            let actual_target = **time_target;

            **time_target = Duration::from_secs_f64(
                actual_target.as_secs_f64() * 0.95 + desired_target.as_secs_f64() * 0.05,
            );
        }
    }

    Ok(())
}

fn run_server_world(time: Res<ServerWorldTime>, mut server_world: NonSendMut<ServerWorld>) {
    let Some(world) = server_world.get() else {
        return;
    };

    let current_time = world.resource::<Time<SimulationTime>>().elapsed();
    let target_time = **time;

    if target_time > current_time {
        **world.resource_mut::<SimulationTimeTarget>() = target_time;
        server_world.update(false);
    }
}
