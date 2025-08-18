use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;

use crate::common::{
    UpdateServerTime,
    scheme::PredictionScheme,
    simulation::{SimulationStepPlugin, SimulationTime, StepSimulation, UpdateQueue, WorldUpdate},
};

pub struct NevyPredictionServerPlugin<S> {
    pub _p: PhantomData<S>,
    pub update_interval: Duration,
    pub schedule: Interned<dyn ScheduleLabel>,
}

impl<S> Default for NevyPredictionServerPlugin<S> {
    fn default() -> Self {
        NevyPredictionServerPlugin {
            _p: PhantomData,
            update_interval: Duration::from_millis(200),
            schedule: Update.intern(),
        }
    }
}

impl<S> Plugin for NevyPredictionServerPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        crate::common::build::<S>(app);

        app.add_shared_sender::<SimulationUpdatesStream>();

        app.configure_sets(
            self.schedule,
            (
                ServerSimulationSet::QueueUpdates,
                ServerSimulationSet::RunSimulation,
                ServerSimulationSet::SendTimeUpdate,
            )
                .chain(),
        );

        app.add_plugins(SimulationStepPlugin {
            schedule: self.schedule,
            step_interval: S::step_interval(),
        });

        app.configure_sets(
            self.schedule,
            StepSimulation.in_set(ServerSimulationSet::RunSimulation),
        );

        app.add_systems(
            self.schedule,
            send_simulation_time_updates::<S>
                .in_set(ServerSimulationSet::SendTimeUpdate)
                .run_if(on_interval(self.update_interval)),
        );

        for update in S::updates().0 {
            update.build_server(app, self.schedule);
        }
    }
}

pub fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static,
{
    app.add_systems(
        schedule,
        queue_world_updates::<T>.in_set(ServerSimulationSet::QueueUpdates),
    );
}

/// Marker type for the simulation updates stream [SharedMessageSender].
pub struct SimulationUpdatesStream;

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSimulationSet {
    QueueUpdates,
    RunSimulation,
    SendTimeUpdate,
}

/// Insert this component onto all clients that are part of the prediction scheme.
#[derive(Component)]
pub struct PredictionClient;

fn on_interval(interval: Duration) -> impl Condition<()> {
    IntoSystem::into_system(move |mut last_run: Local<Duration>, time: Res<Time>| {
        if *last_run + interval <= time.elapsed() {
            *last_run = time.elapsed();
            true
        } else {
            false
        }
    })
}

fn send_simulation_time_updates<S>(
    time: Res<Time<SimulationTime>>,
    clients: Query<Entity, With<PredictionClient>>,
    mut messages: SharedMessageSender<SimulationUpdatesStream>,
    message_id: Res<MessageId<UpdateServerTime>>,
) -> Result
where
    S: PredictionScheme,
{
    for client_entity in &clients {
        messages.write(
            S::message_header(),
            client_entity,
            *message_id,
            true,
            &UpdateServerTime {
                simulation_time: time.elapsed(),
            },
        )?;
    }

    Ok(())
}

#[derive(Resource)]
struct ApplyUpdatesQueue<T> {
    queue: VecDeque<WorldUpdate<T>>,
}

#[derive(SystemParam)]
pub struct ApplyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    time: Res<'w, Time<SimulationTime>>,
    queue: ResMut<'w, ApplyUpdatesQueue<T>>,
}

impl<'w, T> ApplyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    pub fn apply(&mut self, update: T) {
        self.queue.queue.push_back(WorldUpdate {
            time: self.time.elapsed(),
            update,
        });
    }
}

fn queue_world_updates<T>(
    mut input_queue: ResMut<ApplyUpdatesQueue<T>>,
    mut simulation_queue: ResMut<UpdateQueue<T>>,
) where
    T: Send + Sync + 'static,
{
    for update in input_queue.queue.drain(..) {
        simulation_queue.insert(update);
    }
}
