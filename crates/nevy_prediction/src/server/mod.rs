use std::marker::PhantomData;

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;
use serde::Serialize;

use crate::common::{
    ResetClientSimulation, ServerWorldUpdate, UpdateServerTime,
    scheme::PredictionScheme,
    simulation::{
        SimulationInstance, SimulationPlugin, SimulationTime, SimulationTimeTarget,
        SimulationUpdate, StepSimulationSystems, WorldUpdate,
    },
};

pub mod prelude {
    pub use crate::{
        common::{
            ServerWorldUpdate,
            simulation::{
                WorldUpdate, WorldUpdateQueue,
                simulation_entity::{SimulationEntity, SimulationEntityMap},
                update_component::UpdateComponent,
            },
        },
        server::{
            NevyPredictionServerPlugin, PredictionClient, ServerSimulationSystems,
            WorldUpdateSender,
        },
    };
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSimulationSystems {
    QueueUpdatesSystems,
    RunSimulationSystems,
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
                ServerSimulationSystems::QueueUpdatesSystems,
                ServerSimulationSystems::RunSimulationSystems,
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
            StepSimulationSystems.in_set(ServerSimulationSystems::RunSimulationSystems),
        );

        app.add_systems(
            self.schedule,
            (drive_simulation_time, send_simulation_resets::<S>)
                .in_set(ServerSimulationSystems::QueueUpdatesSystems),
        );

        app.add_systems(SimulationUpdate, send_simulation_time_updates::<S>);
    }
}

/// Marker type for the simulation updates stream [SharedMessageSender].
pub struct SimulationUpdatesStream;

/// Insert this component onto all clients that are part of the prediction scheme.
#[derive(Component)]
pub struct PredictionClient;

fn drive_simulation_time(mut target_time: ResMut<SimulationTimeTarget>, time: Res<Time>) {
    **target_time += time.delta();
}

fn send_simulation_time_updates<S>(
    time: Res<Time<SimulationTime>>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut messages: SharedMessageSender<SimulationUpdatesStream>,
    message_id: Res<MessageId<UpdateServerTime>>,
) -> Result
where
    S: PredictionScheme,
{
    for client_entity in &client_q {
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

fn send_simulation_resets<S>(
    new_client_q: Query<Entity, Added<PredictionClient>>,
    time: Res<Time<SimulationTime>>,
    mut messages: SharedMessageSender<SimulationUpdatesStream>,
    message_id: Res<MessageId<ResetClientSimulation>>,
) -> Result
where
    S: PredictionScheme,
{
    for client_entity in &new_client_q {
        messages.write(
            S::message_header(),
            client_entity,
            *message_id,
            true,
            &ResetClientSimulation {
                simulation_time: time.elapsed(),
            },
        )?;
    }

    Ok(())
}

/// Use this system parameter to send world updates to clients.
///
/// Which updates are sent to the clients is not controlled by this crate.
/// You must implement your own logic that sends state updates of the simulation to clients.
///
/// If your simulation is deterministic, you only need to send changes.
/// If it isn't you should periodically refresh the full state of the simulation.
///
/// Additionally, you don't need to update the entire simulation.
/// You can implement logic that only informs the client about changes that are relevant to it.
/// This could be the case for a large world, where you only send updates for the client's local area,
/// or it could be the case for a competitive game where some clients should have information that others don't.
#[derive(SystemParam)]
pub struct WorldUpdateSender<'w, 's> {
    pub sender: SharedMessageSender<'w, 's, SimulationUpdatesStream>,
    pub time: Res<'w, Time<SimulationTime>>,
}

impl<'w, 's> WorldUpdateSender<'w, 's> {
    /// Sends a [`nevy`] message containing a [`WorldUpdate`] with the current simulation time.
    ///
    /// See [`SharedMessageSender::write`].
    pub fn write_now<T>(
        &mut self,
        header: impl Into<u16>,
        client_entity: Entity,
        message_id: MessageId<ServerWorldUpdate<T>>,
        queue: bool,
        update: T,
    ) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        self.write(
            header,
            client_entity,
            message_id,
            queue,
            WorldUpdate {
                time: self.time.elapsed(),
                update,
            },
        )
    }

    /// Sends a [`nevy`] message containing a [`WorldUpdate`].
    ///
    /// If you want to send an update with the current simulation time, use [`Self::write_now`].
    ///
    /// See [`SharedMessageSender::write`].
    pub fn write<T>(
        &mut self,
        header: impl Into<u16>,
        client_entity: Entity,
        message_id: MessageId<ServerWorldUpdate<T>>,
        queue: bool,
        update: WorldUpdate<T>,
    ) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        self.sender.write(
            header,
            client_entity,
            message_id,
            queue,
            &ServerWorldUpdate { update },
        )
    }

    /// Gets the underlying [`SharedMessageSender`], for stream operations.
    pub fn sender(&mut self) -> &mut SharedMessageSender<'w, 's, SimulationUpdatesStream> {
        &mut self.sender
    }
}
