use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::{
    client::{
        ClientSimulationSet, parallel_app::ParallelWorld, prediction_app::PredictionInterval,
    },
    common::{
        ServerWorldUpdate, UpdateServerTime,
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationPlugin, SimulationStartup, SimulationTime,
            SimulationTimeTarget, UpdateQueue,
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
            poll_server_world.in_set(ClientSimulationSet::PollParallelApps),
            receive_time_updates.in_set(ClientSimulationSet::ReceiveTime),
            run_server_world.in_set(ClientSimulationSet::RunPredictionApps),
        ),
    );
}

pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static,
{
    app.add_systems(schedule, receive_world_updates::<T>);
}

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
    mut message_q: Query<&mut ReceivedMessages<ServerWorldUpdate<T>>>,
) where
    T: Send + Sync + 'static,
{
    let Some(world) = server_world.get() else {
        return;
    };

    for mut messages in &mut message_q {
        for ServerWorldUpdate { update } in messages.drain() {
            world.resource_mut::<UpdateQueue<T>>().insert(update);
        }
    }
}

/// Contains the most recent server time update.
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ServerWorldTime(pub Duration);

fn receive_time_updates(
    mut message_q: Query<&mut ReceivedMessages<UpdateServerTime>>,
    mut time: ResMut<ServerWorldTime>,
    mut time_target: ResMut<SimulationTimeTarget>,
    prediction_interval: Res<PredictionInterval>,
) -> Result {
    for mut messages in &mut message_q {
        for UpdateServerTime { simulation_time } in messages.drain() {
            **time = simulation_time;
            **time_target = simulation_time + **prediction_interval;
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
