use std::marker::PhantomData;

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;
use serde::Serialize;

use crate::common::{
    UpdateServerTime,
    scheme::PredictionScheme,
    simulation::{
        SimulationInstance, SimulationPlugin, SimulationSchedule, SimulationTime,
        SimulationTimeTarget, StepSimulation, WorldUpdate,
    },
};

pub use crate::common::{
    ServerWorldUpdate,
    simulation::simulation_entity::{SimulationEntity, SimulationEntityMap},
};

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSimulationSet {
    UpdateSimulationTime,
    QueueUpdates,
    RunSimulation,
}

pub struct NevyPredictionServerPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
}

impl<S> Default for NevyPredictionServerPlugin<S> {
    fn default() -> Self {
        NevyPredictionServerPlugin {
            _p: PhantomData,
            schedule: Update.intern(),
        }
    }
}

impl<S> NevyPredictionServerPlugin<S> {
    pub fn new(schedule: impl ScheduleLabel) -> Self {
        NevyPredictionServerPlugin {
            schedule: schedule.intern(),
            ..default()
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
                ServerSimulationSet::UpdateSimulationTime,
                ServerSimulationSet::QueueUpdates,
                ServerSimulationSet::RunSimulation,
            )
                .chain(),
        );

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: self.schedule,
            instance: SimulationInstance::Server,
        });

        app.configure_sets(
            self.schedule,
            StepSimulation.in_set(ServerSimulationSet::RunSimulation),
        );

        app.add_systems(
            self.schedule,
            drive_simulation_time.in_set(ServerSimulationSet::UpdateSimulationTime),
        );

        app.add_systems(SimulationSchedule, send_simulation_time_updates::<S>);

        for update in S::updates().0 {
            update.build_server(app, self.schedule);
        }
    }
}

pub fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static,
{
    let _ = (app, schedule);
}

/// Marker type for the simulation updates stream [SharedMessageSender].
pub struct SimulationUpdatesStream;

/// Insert this component onto all clients that are part of the prediction scheme.
#[derive(Component)]
pub struct PredictionClient;

// fn on_interval(interval: Duration) -> impl Condition<()> {
//     IntoSystem::into_system(move |mut last_run: Local<Duration>, time: Res<Time>| {
//         if *last_run + interval <= time.elapsed() {
//             *last_run = time.elapsed();
//             true
//         } else {
//             false
//         }
//     })
// }

fn drive_simulation_time(mut target_time: ResMut<SimulationTimeTarget>, time: Res<Time>) {
    **target_time += time.delta();
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

/// Use this system parameter to send world updates to clients.
///
/// These updates will be timestamped to be applied in the next simulation step.
/// To ensure that the updates are applied properly they should be sent in [ServerSimulationSet::QueueUpdates].
#[derive(SystemParam)]
pub struct WorldUpdateSender<'w, 's> {
    messages: SharedMessageSender<'w, 's, SimulationUpdatesStream>,
    time: Res<'w, Time<SimulationTime>>,
}

impl<'w, 's> WorldUpdateSender<'w, 's> {
    pub fn write<T>(
        &mut self,
        header: impl Into<u16>,
        client_entity: Entity,
        message_id: MessageId<ServerWorldUpdate<T>>,
        update: T,
    ) -> Result
    where
        T: Serialize,
    {
        debug!(
            "Sent a world update to {} for time {}",
            client_entity,
            self.time.elapsed().as_millis()
        );

        self.messages.write(
            header,
            client_entity,
            message_id,
            true,
            &ServerWorldUpdate {
                update: WorldUpdate {
                    time: self.time.elapsed(),
                    update,
                },
            },
        )?;

        Ok(())
    }
}
