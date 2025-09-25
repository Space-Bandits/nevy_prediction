use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use nevy::*;

use crate::{
    client::{
        prediction::{PredictionUpdates, PredictionWorld},
        template_world::{ServerTickSamples, TemplateWorld},
    },
    common::{
        ResetClientSimulation,
        scheme::PredictionScheme,
        simulation::{
            PrivateSimulationTimeExt, SimulationInstance, SimulationPlugin, SimulationTick,
            SimulationTime, StepSimulationSystems, WorldUpdate, schedules::ResetSimulation,
        },
    },
    server::prelude::{SimulationTimeExt, UpdateExecutionQueue},
};

pub mod prediction;
pub(crate) mod simulation_world;
pub(crate) mod template_world;

pub mod prelude {
    pub use crate::client::{
        ClientSimulationSystems, NevyPredictionClientPlugin, PredictionInterval, PredictionRates,
        PredictionServerConnection, PredictionUpdateCreator,
    };
    pub use crate::common::simulation::{
        SimulationTime, StepSimulationSystems, WorldUpdate,
        simulation_entity::{SimulationEntity, SimulationEntityMap},
        update_component::UpdateComponent,
    };
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientSimulationSystems {
    /// Runs first, where the [`ResetSimulation`] schedule is run if needed.
    ResetSimulation,
    /// Simulation time updates are received by the server.
    ReceiveTime,
    /// User queues world updates.
    QueueUpdates,
    RunTemplateWorld,
    /// Any updates than should be included in prediction are queued.
    QueuePredictionUpdates,
    RunPredictionWorld,
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

        app.init_resource::<PredictionRates>();
        app.init_resource::<PredictionBudget>();

        app.configure_sets(
            self.schedule,
            (
                ClientSimulationSystems::ResetSimulation,
                ClientSimulationSystems::ReceiveTime,
                ClientSimulationSystems::QueueUpdates,
                StepSimulationSystems,
                ClientSimulationSystems::RunTemplateWorld,
                ClientSimulationSystems::QueuePredictionUpdates,
                ClientSimulationSystems::RunPredictionWorld,
            )
                .chain(),
        );

        crate::common::build::<S>(app);
        template_world::build::<S>(app, self.schedule);
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
                    .pipe(reset_simulations::<S>)
                    .in_set(ClientSimulationSystems::ResetSimulation),
                drive_simulation_time::<S>.in_set(ClientSimulationSystems::ReceiveTime),
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

    template_world::build_update::<T>(app, schedule);
    prediction::build_update::<T>(app, schedule);
}

/// Controls how many updates prediction logic is allowed to relative to the main app.
///
/// These values should be greater than one to allow prediction logic to catch up,
/// but if they are too high, too many updates may run in a single frame which cause hitching.
#[derive(Resource)]
pub struct PredictionRates {
    pub template: f32,
    pub prediction: f32,
}

impl Default for PredictionRates {
    fn default() -> Self {
        PredictionRates {
            template: 3.,
            prediction: 5.,
        }
    }
}

/// Controls how many updates the template and prediction worlds are allowed to execute.
#[derive(Resource, Default)]
struct PredictionBudget {
    pub template: u32,
    pub prediction: u32,

    template_overstep: f32,
    prediction_overstep: f32,
}

/// Controls how far prediction is run.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct PredictionInterval(pub Duration);

/// Marker component that must be inserted onto the server connection entity for prediction.
#[derive(Component)]
pub struct PredictionServerConnection;

fn drive_simulation_time<S>(
    server_time: Res<ServerTickSamples>,
    interval: Res<PredictionInterval>,
    mut time: ResMut<Time<SimulationTime>>,
    real_time: Res<Time<Real>>,
    rates: Res<PredictionRates>,
    mut budget: ResMut<PredictionBudget>,
) where
    S: PredictionScheme,
{
    loop {
        let target_time = server_time.estimated_time::<S>(real_time.elapsed()) + **interval;
        let current_time = time.target_tick().time::<S>();

        if current_time + S::step_interval() > target_time {
            break;
        }

        time.queue_ticks(1);

        budget.template_overstep += rates.template;
        budget.prediction_overstep += rates.prediction;
    }

    budget.template = 0;
    budget.prediction = 0;

    while budget.template_overstep > 1. {
        budget.template_overstep -= 1.;
        budget.template += 1;
    }

    while budget.prediction_overstep > 1. {
        budget.prediction_overstep -= 1.;
        budget.prediction += 1;
    }
}

fn receive_reset_simulations(
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<ResetClientSimulation>,
        Has<PredictionServerConnection>,
    )>,
) -> Option<SimulationTick> {
    let mut reset = None;

    for (connection_entity, mut messages, is_server) in &mut message_q {
        for ResetClientSimulation {
            simulation_tick: simulation_time,
        } in messages.drain()
        {
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

fn reset_simulations<S>(In(reset_tick): In<Option<SimulationTick>>, world: &mut World)
where
    S: PredictionScheme,
{
    let Some(reset_tick) = reset_tick else {
        return;
    };

    debug!("resetting simulation to {:?}", reset_tick);

    world.resource_mut::<TemplateWorld>().reset::<S>(reset_tick);

    world
        .resource_mut::<PredictionWorld>()
        .reset::<S>(reset_tick);

    world.insert_resource(Time::<SimulationTime>::from_tick::<S>(reset_tick));
    world.run_schedule(ResetSimulation);

    world.init_resource::<PredictionBudget>();

    let real_time = world.resource::<Time<Real>>().elapsed();
    world
        .resource_mut::<ServerTickSamples>()
        .reset::<S>(real_time, reset_tick);
}

#[derive(SystemParam)]
pub struct PredictionUpdateCreator<'w, T>
where
    T: Send + Sync + 'static,
{
    time: Res<'w, Time<SimulationTime>>,
    simulation_queue: ResMut<'w, UpdateExecutionQueue<T>>,
    prediction_world: ResMut<'w, PredictionWorld>,
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
            tick: self.time.current_tick(),
            update,
        };

        self.simulation_queue.insert(update.clone());
        self.prediction_world
            .resource_mut::<PredictionUpdates<T>>()
            .push_back(update.clone());

        update
    }
}
