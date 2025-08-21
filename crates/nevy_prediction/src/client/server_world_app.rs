use std::marker::PhantomData;

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::{
    client::{ClientPredictionSet, parallel_app::ParallelApp},
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
    app.insert_non_send_resource(ServerWorldApp::new::<S>());

    app.add_systems(
        schedule,
        (
            receive_time_updates.in_set(ClientPredictionSet::QueueUpdates),
            run_server_world.in_set(ClientPredictionSet::RunServerWorld),
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
pub(crate) struct ServerWorldApp(pub ParallelApp);

impl ServerWorldApp {
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

        ServerWorldApp(ParallelApp::new(app))
    }
}

fn receive_world_updates<T>(
    mut server_app: NonSendMut<ServerWorldApp>,
    mut message_q: Query<&mut ReceivedMessages<ServerWorldUpdate<T>>>,
) -> Result
where
    T: Send + Sync + 'static,
{
    let Some(app) = server_app.poll()? else {
        return Ok(());
    };

    for mut messages in &mut message_q {
        for ServerWorldUpdate { update } in messages.drain() {
            app.world_mut()
                .resource_mut::<UpdateQueue<T>>()
                .insert(update);
        }
    }

    Ok(())
}

fn receive_time_updates(
    mut server_app: NonSendMut<ServerWorldApp>,
    mut message_q: Query<&mut ReceivedMessages<UpdateServerTime>>,
) -> Result {
    let Some(app) = server_app.poll()? else {
        return Ok(());
    };

    for mut messages in &mut message_q {
        for UpdateServerTime { simulation_time } in messages.drain() {
            **app.world_mut().resource_mut::<SimulationTimeTarget>() = simulation_time;
        }
    }

    Ok(())
}

fn run_server_world(mut server_app: NonSendMut<ServerWorldApp>) -> Result {
    let Some(app) = server_app.poll()? else {
        return Ok(());
    };

    let current_time = app.world().resource::<Time<SimulationTime>>().elapsed();
    let target_time = **app.world().resource::<SimulationTimeTarget>();

    if target_time > current_time {
        server_app.update();
    }

    Ok(())
}
