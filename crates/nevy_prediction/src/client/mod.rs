use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;

use crate::{
    client::{
        prediction::{PredictionInterval, PredictionUpdates, PredictionWorld},
        server_world::ServerWorld,
    },
    common::{
        ResetClientSimulation,
        scheme::PredictionScheme,
        simulation::{
            ResetSimulation, SimulationInstance, SimulationPlugin, SimulationTime,
            SimulationTimeTarget, StepSimulationSystems, WorldUpdate,
        },
    },
    server::prelude::WorldUpdateQueue,
};

pub(crate) mod parallel_app;
pub mod prediction;
pub(crate) mod server_world;

pub mod prelude {
    pub use crate::client::{
        ClientSimulationSystems, NevyPredictionClientPlugin, PredictionServerConnection,
        PredictionUpdateCreator, prediction::PredictionInterval,
    };
    pub use crate::common::simulation::{
        SimulationTime, StepSimulationSystems, WorldUpdate,
        simulation_entity::{SimulationEntity, SimulationEntityMap},
        update_component::UpdateComponent,
    };
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientSimulationSystems {
    /// Parallel apps are polled for completion.
    PollParallelAppsSystems,
    /// Simulations get reset by the server.
    ResetSimulationSystems,
    /// Simulation time updates are received by the server.
    ReceiveTimeSystems,
    /// The prediction app is extracted into the main world
    /// The server world app is extracted into the prediction app
    ExtractSimulationsSystems,
    /// Reapply world updates that have been added while the prediction world was running
    ReapplyNewWorldUpdatesSystems,
    /// User queues world updates.
    QueueUpdatesSystems,
    /// Predicted updates are queued to the prediction world.
    QueuePredictionAppUpdatesSystems,
    /// Prediction apps are run.
    RunParallelAppsSystems,
}

/// Used to add systems when building a world update
#[derive(Resource, Deref)]
pub(crate) struct ClientPredictionSchedule(pub Interned<dyn ScheduleLabel>);

pub struct NevyPredictionClientPlugin<S> {
    pub(crate) _p: PhantomData<S>,
    pub(crate) schedule: Interned<dyn ScheduleLabel>,
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
        app.insert_resource(ClientPredictionSchedule(self.schedule));

        app.configure_sets(
            self.schedule,
            (
                ClientSimulationSystems::PollParallelAppsSystems,
                ClientSimulationSystems::ResetSimulationSystems,
                ClientSimulationSystems::ReceiveTimeSystems,
                ClientSimulationSystems::ExtractSimulationsSystems,
                ClientSimulationSystems::ReapplyNewWorldUpdatesSystems,
                ClientSimulationSystems::QueueUpdatesSystems,
                ClientSimulationSystems::QueuePredictionAppUpdatesSystems,
                ClientSimulationSystems::RunParallelAppsSystems,
            )
                .chain()
                .before(StepSimulationSystems),
        );

        crate::common::build::<S>(app);
        server_world::build::<S>(app, self.schedule);
        prediction::build::<S>(app, self.schedule);

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
                    .in_set(ClientSimulationSystems::ResetSimulationSystems),
                drive_simulation_time.in_set(ClientSimulationSystems::ReceiveTimeSystems),
            ),
        );

        // for update in S::updates().0 {
        //     update.build_client(app);
        // }
    }
}

/// Is called on the client app for each world update message added by the prediction scheme
pub(crate) fn build_update<T>(app: &mut App)
where
    T: Send + Sync + 'static + Clone,
{
    let schedule = **app.world().resource::<ClientPredictionSchedule>();

    server_world::build_update::<T>(app, schedule);
    prediction::build_update::<T>(app, schedule);
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
pub struct PredictionUpdateCreator<'w, T>
where
    T: Send + Sync + 'static,
{
    time: Res<'w, Time<SimulationTime>>,
    simulation_queue: ResMut<'w, WorldUpdateQueue<T>>,
    prediction_updates: ResMut<'w, PredictionUpdates<T>>,
}

impl<'w, T> PredictionUpdateCreator<'w, T>
where
    T: Send + Sync + 'static + Clone,
{
    /// Creates a simulation [`WorldUpdate`], applies to the world and records it for client side prediction.
    ///
    /// It is the caller's responsibility to inform the server that the client wishes to apply this update to the server.
    /// To do this, the world update is returned and should be sent to the server.
    /// The update can then be validated and reconciled on the server.
    ///
    /// It should be noted that you do not need to use the world update returned by this function in your network message.
    /// If the server can infer what the update is based on which client sent it,
    /// then all that needs to be communicated is the timestamp of the returned update.
    pub fn create(&mut self, update: T) -> WorldUpdate<T> {
        let update = WorldUpdate {
            time: self.time.elapsed(),
            update,
        };

        self.simulation_queue.insert(update.clone());
        self.prediction_updates.push_back(update.clone());

        update
    }
}
