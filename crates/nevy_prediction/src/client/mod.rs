//! The client side of prediction.
//!
//! The client simulates three copies of the simulation on separate threads, see [`pipeline`].
//! The app's own world doesn't execute the simulation,
//! the latest state of the client's main simulation world is extracted into it during [`SyncSimulationSystems`].

use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    platform::time::Instant,
    prelude::*,
};
use nevy::prelude::*;
use tracing::{debug, warn};

use crate::{
    client::{
        interval::{IntervalController, PredictionInterval},
        pipeline::{ClientSimulation, ClientSimulationStats},
        server_time::ServerTickSamples,
    },
    common::{
        ResetClientSimulation, ServerWorldUpdate, SimulationCapacity,
        clock::SimulationClock,
        runtime::SimulationExecution,
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationPlugin, SimulationTick, SimulationTime,
            SimulationTimeExt, SyncSimulationSystems, WorldUpdate, world::reset_world,
        },
        time::{PredictionTime, PrivatePredictionTimeExt},
    },
};

pub mod interval;
pub mod pipeline;
pub(crate) mod prediction;
pub(crate) mod server_time;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientSimulationSystems {
    /// Runs first, where the simulation is reset if the server requested it.
    ResetSimulation,
    /// Updates from the server are received and the simulation clock is updated.
    ReceiveUpdates,
    /// User queues world updates with [`PredictionUpdateCreator`].
    QueueUpdates,
}

/// Used to add systems when building a world update
#[derive(Resource, Deref)]
pub(crate) struct ClientPredictionSchedule(pub Interned<dyn ScheduleLabel>);

/// Marker type for the [`SharedMessageSender`] used to send prediction messages to the server.
pub(crate) struct ClientPredictionStream;

pub struct NevyPredictionClientPlugin<S> {
    pub(crate) _p: PhantomData<S>,
    pub(crate) schedule: Interned<dyn ScheduleLabel>,
    /// Where the simulation is executed.
    pub execution: SimulationExecution,
}

impl<S> Default for NevyPredictionClientPlugin<S> {
    fn default() -> Self {
        NevyPredictionClientPlugin {
            _p: PhantomData,
            schedule: Update.intern(),
            execution: default(),
        }
    }
}

impl<S> NevyPredictionClientPlugin<S> {
    pub fn new(schedule: impl ScheduleLabel) -> Self {
        NevyPredictionClientPlugin {
            schedule: schedule.intern(),
            ..default()
        }
    }

    pub fn with_execution(mut self, execution: SimulationExecution) -> Self {
        self.execution = execution;
        self
    }
}

impl<S> Plugin for NevyPredictionClientPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        app.insert_resource(ClientPredictionSchedule(self.schedule));

        app.init_resource::<PredictionRates>();
        app.init_resource::<ServerTickSamples>();
        app.init_resource::<ClientSimulationStats>();
        app.init_resource::<Time<PredictionTime>>();
        app.init_resource::<PresentationMargin>();

        let rates = app.world().resource::<PredictionRates>().clone();
        app.insert_resource(ClientSimulation::new::<S>(self.execution, rates));

        app.add_shared_message_sender::<ClientPredictionStream>(StreamRequirements::RELIABLE_ORDERED);

        app.configure_sets(
            self.schedule,
            (
                ClientSimulationSystems::ResetSimulation,
                ClientSimulationSystems::ReceiveUpdates,
                ClientSimulationSystems::QueueUpdates,
                SyncSimulationSystems,
            )
                .chain(),
        );

        crate::common::build(app);
        interval::build(app, self.schedule);

        app.add_plugins(SimulationPlugin::<S>::new(SimulationInstance::ClientApp));

        app.add_systems(
            self.schedule,
            (
                receive_reset_simulations
                    .pipe(reset_simulations::<S>)
                    .in_set(ClientSimulationSystems::ResetSimulation),
                (
                    server_time::receive_time_updates,
                    interval::update_prediction_interval,
                    drive_simulation_time,
                )
                    .chain()
                    .in_set(ClientSimulationSystems::ReceiveUpdates),
                send_simulation_capacity.in_set(ClientSimulationSystems::QueueUpdates),
                (sync_simulation::<S>, update_stats)
                    .chain()
                    .in_set(SyncSimulationSystems),
            ),
        );
    }
}

/// Is called on the client app for each world update message added by the prediction scheme
pub(crate) fn build_update<T>(app: &mut App)
where
    T: Send + Sync + 'static + Clone,
{
    let schedule = **app.world().resource::<ClientPredictionSchedule>();

    app.add_systems(
        schedule,
        receive_world_updates::<T>.in_set(ClientSimulationSystems::ReceiveUpdates),
    );
}

/// Controls how many ticks the template and prediction worlds are allowed to execute per tick of the main world.
///
/// These values should be greater than one to allow prediction logic to catch up,
/// but if they are too high the template and prediction worlds may use a lot of processing time when they fall behind.
///
/// Budget that isn't spent carries over to the next main world tick, but only up to one tick's worth.
#[derive(Resource, Clone, Debug)]
pub struct PredictionRates {
    pub template: f32,
    pub prediction: f32,
    /// How many times faster than real time the prediction world must be able to execute ticks
    /// for this client to be considered to keep up with the simulation.
    ///
    /// A prediction has to catch up to the main world while the main world keeps advancing,
    /// so it must run faster than real time to ever finish.
    /// The closer to real time it runs, the longer each prediction takes and the later corrections are seen.
    /// At 1.5 a prediction takes three times as many ticks to catch up as its length.
    ///
    /// If this client can't predict this fast, it asks the server to slow down the simulation.
    pub min_prediction_speed: f32,
}

impl Default for PredictionRates {
    fn default() -> Self {
        PredictionRates {
            template: 3.,
            prediction: 5.,
            min_prediction_speed: 1.5,
        }
    }
}

/// Marker component that must be inserted onto the server connection entity for prediction.
#[derive(Component)]
pub struct PredictionServerConnection;

/// Updates the clock that the main world is driven by, so that it runs [`PredictionInterval`] ahead of the server.
fn drive_simulation_time(
    mut server_time: ResMut<ServerTickSamples>,
    interval: Res<PredictionInterval>,
    simulation: Res<ClientSimulation>,
    rates: Res<PredictionRates>,
) {
    let now = Instant::now();
    let target_time = server_time.estimated_time(now) + **interval;

    simulation.set_targets(
        SimulationClock::new(now, target_time, server_time.time_scale()),
        server_time.latest(),
        &rates,
    );
}

/// How quickly [`PredictionTime`] is steered towards where it should be, per second.
const PREDICTION_TIME_CORRECTION: f64 = 2.;

/// How quickly the presentation margin shrinks, in steps per second.
const PRESENTATION_MARGIN_DECAY: f64 = 0.05;

/// Extra delay for [`PredictionTime`], so that it doesn't run ahead of the latest simulation state.
///
/// Simulation states reach the app some time after their tick is due, because ticks take time to execute.
/// Whenever presented time would pass the latest state the margin grows by how far it would have,
/// and otherwise it slowly shrinks.
#[derive(Resource, Default)]
pub(crate) struct PresentationMargin(f64);

/// Extracts the latest state of the main world into the app and advances [`PredictionTime`].
fn sync_simulation<S>(world: &mut World)
where
    S: PredictionScheme,
{
    world.resource_scope(|world, mut simulation: Mut<ClientSimulation>| {
        simulation.update();
        simulation.try_extract(world);

        let Some(clock) = simulation.clock() else {
            return;
        };

        let real_delta = world.resource::<Time<Real>>().delta_secs_f64();
        let latest_tick = world.resource::<Time<SimulationTime>>().current_tick();

        let step = S::step_interval().as_secs_f64();
        let latest = latest_tick.time::<S>().as_secs_f64();
        let scale = clock.scale() as f64;

        let margin = world.resource::<PresentationMargin>().0;

        // Present one tick behind the main world's clock, so that there are two ticks to interpolate between.
        let target = clock.time_at(Instant::now()).as_secs_f64() - step - margin;

        let mut time = world.resource_mut::<Time<PredictionTime>>();
        let previous = time.elapsed_secs_f64();

        let mut presented = previous + real_delta * scale;
        let error = target - presented;

        if error.abs() > step * 2. {
            presented = target;
        } else {
            presented += error * (real_delta * PREDICTION_TIME_CORRECTION).min(1.);
        }

        let overshoot = presented - latest;

        // Never present a time without a simulation state, and never fall more than a tick behind.
        let presented = presented
            .min(latest)
            .max(latest - step)
            .max(previous);

        time.advance_to(Duration::from_secs_f64(presented));

        let context = time.context_mut();
        context.time_scale = clock.scale();
        context.latest_tick = latest_tick;
        context.fraction = ((presented - (latest - step)) / step).clamp(0., 1.) as f32;

        let mut margin = world.resource_mut::<PresentationMargin>();
        margin.0 = if overshoot > 0. {
            margin.0 + overshoot
        } else {
            margin.0 - real_delta * scale * step * PRESENTATION_MARGIN_DECAY
        }
        .clamp(0., step);
    });
}

fn update_stats(
    simulation: Res<ClientSimulation>,
    server_time: Res<ServerTickSamples>,
    mut stats: ResMut<ClientSimulationStats>,
) {
    *stats = simulation.stats(server_time.latest());
}

/// How often the client reports its capacity to the server.
const CAPACITY_INTERVAL: Duration = Duration::from_millis(250);

fn send_simulation_capacity(
    stats: Res<ClientSimulationStats>,
    real_time: Res<Time<Real>>,
    mut last_sent: Local<Option<Duration>>,
    server_q: Query<Entity, With<PredictionServerConnection>>,
    mut messages: SharedMessageSender<ClientPredictionStream>,
) -> Result {
    let now = real_time.elapsed();

    if !stats.active || last_sent.is_some_and(|last| now - last < CAPACITY_INTERVAL) {
        return Ok(());
    }

    let (Some(capacity), Ok(server_entity)) = (stats.capacity, server_q.single()) else {
        return Ok(());
    };

    messages.write(
        server_entity,
        true,
        &SimulationCapacity {
            time_scale: capacity,
        },
    )?;

    *last_sent = Some(now);

    Ok(())
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
        for ResetClientSimulation { simulation_tick } in messages.drain() {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            reset = Some(simulation_tick);
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

    world.resource_mut::<ClientSimulation>().reset::<S>(reset_tick);

    reset_world::<S>(world, reset_tick);

    world.resource_mut::<IntervalController>().reset();

    world.resource_mut::<ServerTickSamples>().reset(
        Instant::now(),
        reset_tick,
        reset_tick.time::<S>(),
    );

    world.insert_resource(Time::<PredictionTime>::from_time(
        reset_tick.time::<S>(),
        reset_tick,
    ));
    world.insert_resource(PresentationMargin::default());
}

fn receive_world_updates<T>(
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<ServerWorldUpdate<T>>,
        Has<PredictionServerConnection>,
    )>,
    simulation: Res<ClientSimulation>,
) where
    T: Send + Sync + 'static + Clone,
{
    let mut updates = Vec::new();

    for (connection_entity, mut messages, is_server) in &mut message_q {
        for ServerWorldUpdate {
            update,
            include_in_prediction,
        } in messages.drain()
        {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            updates.push((update, include_in_prediction));
        }
    }

    simulation.queue_server_updates(updates);
}

#[derive(SystemParam)]
pub struct PredictionUpdateCreator<'w, T>
where
    T: Send + Sync + 'static,
{
    simulation: Res<'w, ClientSimulation>,
    _p: PhantomData<fn() -> T>,
}

impl<'w, T> PredictionUpdateCreator<'w, T>
where
    T: Send + Sync + 'static + Clone,
{
    /// Creates a simulation [`WorldUpdate`], applies it to the client's simulation and records it for client side prediction.
    ///
    /// The update is stamped with the next tick that the client's main simulation world will execute.
    ///
    /// It is the caller's responsibility to inform the server that the client wishes to apply this update to the server.
    /// To do this, the world update is returned and should be sent to the server.
    /// The update can then be validated and reconciled on the server.
    ///
    /// It should be noted that you do not need to use the world update returned by this function in your network message.
    /// If the server can infer what the update is based on which client sent it,
    /// then all that needs to be communicated is the timestamp of the returned update.
    pub fn create(&mut self, update: T) -> WorldUpdate<T> {
        self.simulation.queue_local_update(update)
    }
}
