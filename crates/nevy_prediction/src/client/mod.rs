use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;
use serde::Serialize;

use crate::{
    client::{
        prediction_app::{PredictionInterval, PredictionUpdates, PredictionWorld},
        server_world_app::ServerWorld,
    },
    common::{
        RequestWorldUpdate, ResetClientSimulation,
        scheme::PredictionScheme,
        simulation::{
            ResetSimulation, SimulationInstance, SimulationPlugin, SimulationTime,
            SimulationTimeTarget, StepSimulation, UpdateQueue, WorldUpdate,
        },
    },
};

pub mod parallel_app;
pub mod prediction_app;
pub mod server_world_app;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientSimulationSet {
    /// Parallel apps are polled for completion.
    PollParallelApps,
    /// Simulations get reset by the server.
    ResetSimulations,
    /// Simulation time updates are received by the server.
    ReceiveTime,
    /// The prediction app is extracted into the main world
    /// The server world app is extracted into the prediction app
    ExtractPredictionWorlds,
    /// Reapply world updates that have been added while the prediction world was running
    ReapplyNewWorldUpdates,
    /// User queues world updates.
    QueueUpdates,
    /// Predicted updates are queued to the prediction world.
    QueuePredictionAppUpdates,
    /// Prediction apps are run.
    RunPredictionApps,
}

pub struct NevyPredictionClientPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
}

impl<S> Default for NevyPredictionClientPlugin<S> {
    fn default() -> Self {
        NevyPredictionClientPlugin {
            _p: PhantomData,
            schedule: Update.intern(),
        }
    }
}

impl<S> Plugin for NevyPredictionClientPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        app.configure_sets(
            self.schedule,
            (
                ClientSimulationSet::PollParallelApps,
                ClientSimulationSet::ResetSimulations,
                ClientSimulationSet::ReceiveTime,
                ClientSimulationSet::ExtractPredictionWorlds,
                ClientSimulationSet::ReapplyNewWorldUpdates,
                ClientSimulationSet::QueueUpdates,
                ClientSimulationSet::QueuePredictionAppUpdates,
                ClientSimulationSet::RunPredictionApps,
            )
                .chain()
                .before(StepSimulation),
        );

        crate::common::build::<S>(app);
        server_world_app::build::<S>(app, self.schedule);
        prediction_app::build::<S>(app, self.schedule);

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: self.schedule,
            instance: SimulationInstance::ClientMain,
        });

        app.add_systems(
            self.schedule,
            (
                receive_reset_simulations
                    .pipe(reset_simulations)
                    .in_set(ClientSimulationSet::ResetSimulations),
                drive_simulation_time.in_set(ClientSimulationSet::ReceiveTime),
            ),
        );

        for update in S::updates().0 {
            update.build_client(app, self.schedule);
        }
    }
}

/// Is called on the client app for each world update message added by the prediction scheme
pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static + Clone,
{
    server_world_app::build_update::<T>(app, schedule);
    prediction_app::build_update::<T>(app, schedule);
}

/// Marker component that must be inserted onto the server connection entity for prediction.
#[derive(Component)]
pub struct PredictionServerConnection;

fn drive_simulation_time(mut target_time: ResMut<SimulationTimeTarget>, time: Res<Time>) {
    **target_time += time.delta();
}

fn receive_reset_simulations(
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<ResetClientSimulation>,
        Has<PredictionServerConnection>,
    )>,
) -> Option<Duration> {
    let mut reset = None;

    for (connection_entity, mut messages, is_server) in &mut message_q {
        for ResetClientSimulation { simulation_time } in messages.drain() {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            reset = Some(simulation_time);
        }
    }

    reset
}

fn reset_simulations(In(reset): In<Option<Duration>>, world: &mut World) {
    let Some(elapsed) = reset else {
        return;
    };

    debug!("resetting simulation to {:?}", elapsed);

    world.non_send_resource_mut::<ServerWorld>().reset(elapsed);
    world
        .non_send_resource_mut::<PredictionWorld>()
        .world
        .reset(elapsed);

    let prediction_interval = **world.resource::<PredictionInterval>();

    let mut time = Time::new_with(SimulationTime);
    time.advance_to(elapsed + prediction_interval);
    world.insert_resource(time);
    world.insert_resource(SimulationTimeTarget(elapsed + prediction_interval));

    world.run_schedule(ResetSimulation);
}

#[derive(SystemParam)]
pub struct PredictionUpdateSender<'w, 's, T>
where
    T: Send + Sync + 'static,
{
    time: Res<'w, Time<SimulationTime>>,
    simulation_queue: ResMut<'w, UpdateQueue<T>>,
    prediction_updates: ResMut<'w, PredictionUpdates<T>>,
    connection_q: Query<'w, 's, Entity, With<PredictionServerConnection>>,
    message_sender: LocalMessageSender<'w, 's>,
    message_id: Res<'w, MessageId<RequestWorldUpdate<T>>>,
}

impl<'w, 's, T> PredictionUpdateSender<'w, 's, T>
where
    T: Send + Sync + 'static + Clone + Serialize,
{
    pub fn flush(&mut self) -> Result {
        self.message_sender.flush()?;

        Ok(())
    }

    pub fn finish_if_uncongested(&mut self) -> Result {
        self.message_sender.finish_all_if_uncongested()?;

        Ok(())
    }

    /// Creates a simulation world update for the next simulation step and does three things with it:
    ///
    /// - Sends it to the server
    /// - Queues it in the main app simulation
    /// - Records it to be used in prediction
    ///
    /// For `header` and `queue` see [LocalMessageSender::write].
    pub fn write(&mut self, header: impl Into<u16>, queue: bool, update: T) -> Result {
        let connection_entity = self.connection_q.single()?;

        let update = WorldUpdate {
            time: self.time.elapsed(),
            update,
        };

        self.message_sender.write(
            header,
            connection_entity,
            *self.message_id,
            queue,
            &RequestWorldUpdate {
                update: update.clone(),
            },
        )?;

        self.simulation_queue.insert(update.clone());
        self.prediction_updates.push_back(update);

        Ok(())
    }
}
