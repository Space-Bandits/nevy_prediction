use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;
use serde::Serialize;

use crate::common::{
    ResetClientSimulation, ServerWorldUpdate, UpdateServerTick,
    scheme::PredictionScheme,
    simulation::{
        PrivateSimulationTimeExt, SimulationInstance, SimulationPlugin, SimulationTime,
        SimulationTimeExt, StepSimulationSystems, WorldUpdate, schedules::SimulationPostUpdate,
    },
};

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSimulationSystems {
    SendResets,
    QueueUpdates,
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
                ServerSimulationSystems::SendResets,
                ServerSimulationSystems::QueueUpdates,
                StepSimulationSystems,
            )
                .chain(),
        );

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: self.schedule,
            instance: SimulationInstance::Server,
        });

        app.add_systems(
            self.schedule,
            (
                send_simulation_resets::<S>.in_set(ServerSimulationSystems::SendResets),
                drive_simulation_time::<S>.in_set(ServerSimulationSystems::QueueUpdates),
            ),
        );

        app.add_systems(SimulationPostUpdate, send_simulation_time_updates::<S>);
    }
}

/// Marker type for the simulation updates stream [SharedMessageSender].
pub struct SimulationUpdatesStream;

/// Insert this component onto all clients that are part of the prediction scheme.
#[derive(Component)]
pub struct PredictionClient;

fn drive_simulation_time<S>(
    mut time: ResMut<Time<SimulationTime>>,
    real_time: Res<Time<Real>>,
    mut overstep: Local<Duration>,
) where
    S: PredictionScheme,
{
    *overstep += real_time.delta();

    loop {
        if *overstep < S::step_interval() {
            break;
        }
        *overstep -= S::step_interval();

        time.queue_ticks(1);
    }
}

fn send_simulation_time_updates<S>(
    time: Res<Time<SimulationTime>>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut messages: SharedNetMessageSender<SimulationUpdatesStream>,
    message_id: Res<NetMessageId<UpdateServerTick>>,
) -> Result
where
    S: PredictionScheme,
{
    for client_entity in &client_q {
        messages.write(
            client_entity,
            *message_id,
            true,
            &UpdateServerTick {
                simulation_tick: time.current_tick(),
            },
        )?;
    }

    Ok(())
}

fn send_simulation_resets<S>(
    new_client_q: Query<Entity, Added<PredictionClient>>,
    time: Res<Time<SimulationTime>>,
    mut messages: SharedNetMessageSender<SimulationUpdatesStream>,
    message_id: Res<NetMessageId<ResetClientSimulation>>,
) -> Result
where
    S: PredictionScheme,
{
    for client_entity in &new_client_q {
        messages.write(
            client_entity,
            *message_id,
            true,
            &ResetClientSimulation {
                simulation_tick: time.current_tick(),
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
    pub sender: SharedNetMessageSender<'w, 's, SimulationUpdatesStream>,
    pub time: Res<'w, Time<SimulationTime>>,
}

impl<'w, 's> WorldUpdateSender<'w, 's> {
    /// Sends a [`nevy`] message containing a [`WorldUpdate`] with the current simulation time.
    ///
    /// This method would typically be used when informing clients of [`WorldUpdate`]s generated by the server.
    ///
    /// Note there is no `include_in_prediction` argument like there is in [`Self::write`],
    /// because updates sent to clients by this method will immediately be applied.
    pub fn write_now<T>(
        &mut self,
        client_entity: Entity,
        message_id: NetMessageId<ServerWorldUpdate<T>>,
        queue: bool,
        update: T,
    ) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        self.write(
            client_entity,
            message_id,
            queue,
            false,
            WorldUpdate {
                tick: self.time.current_tick(),
                update,
            },
        )
    }

    /// Sends a [`nevy`] message that instructs the client to queue a [`WorldUpdate`] to their local copy of the simulation.
    ///
    /// This method would normally be used when reconciling [`WorldUpdate`]s that a client has requested,
    /// to inform all the clients of the change that has been made.
    ///
    /// If a [`WorldUpdate`] is generated by the server you would normally use [`Self::write_now`].
    ///
    /// In addition to the normal arguments from [`SharedMessageSender::write`] there is `include_in_prediction`.
    /// If this is set to `true` then the client will include this [`WorldUpdate`] in client side prediction.
    /// When the client includes the update in prediction it means that they will see the change sooner.
    /// If the update has a diverging effect on the simulation, such as changing a velocity, then this is very important.
    /// The sooner the client sees the change the less "jerk" there will be when the update is applied to the reconciled state on the client.
    /// Care should be taken to not "double up" on an update.
    /// If a client makes a change to their copy of the simulation, requests an update be applied, and you respond with your own copy of the update then it will be included twice.
    /// You still need to inform them that the update has been reconciled, but they already have it in their prediction queue. All other clients do need to add it to their prediction queue however.
    /// In the case where latency is not important, and there is no jerk from this update being reconciled, then it may be simpler to just have this value be false.
    pub fn write<T>(
        &mut self,
        client_entity: Entity,
        message_id: NetMessageId<ServerWorldUpdate<T>>,
        queue: bool,
        include_in_prediction: bool,
        update: WorldUpdate<T>,
    ) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        self.sender.write(
            client_entity,
            message_id,
            queue,
            &ServerWorldUpdate {
                update,
                include_in_prediction,
            },
        )
    }

    /// Gets the underlying [`SharedMessageSender`], for stream operations.
    pub fn sender(&mut self) -> &mut SharedNetMessageSender<'w, 's, SimulationUpdatesStream> {
        &mut self.sender
    }
}
