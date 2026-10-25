//! The server side of prediction.
//!
//! The authoritative simulation runs on its own thread, see [`pipeline`].
//! The server app's world doesn't execute the simulation,
//! the latest state of the simulation is extracted into it during [`SyncSimulationSystems`] so that game logic can read it.
//!
//! Game logic changes the simulation by queueing world updates with [`ServerUpdateQueue`],
//! and informs clients of the changes it makes with a [`WorldUpdateSender`].

use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    platform::time::Instant,
    prelude::*,
};
use nevy::prelude::*;
use serde::Serialize;

use crate::{
    common::{
        ResetClientSimulation, ServerWorldUpdate, TickProbe, TickProbeResult, UpdateServerTick,
        runtime::SimulationExecution,
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationPlugin, SimulationTick, SimulationTime,
            SimulationTimeExt, SyncSimulationSystems, WorldUpdate,
        },
        time::PredictionTime,
    },
    server::pipeline::{ServerSimulation, ServerSimulationStats},
};

pub mod pipeline;
pub mod time_scale;

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSimulationSystems {
    /// Where new [`PredictionClient`]s are reset.
    SendResets,
    /// Where world updates should be queued with [`ServerUpdateQueue`] and sent to clients.
    ///
    /// Initial state for new clients should be sent here, for clients with [`Added<PredictionClient>`].
    QueueUpdates,
    /// Where world updates that were queued before a new client joined are sent to it.
    SendReplays,
}

pub struct NevyPredictionServerPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
    /// Where the simulation is executed.
    pub execution: SimulationExecution,
    /// The most simulation time the server will try to catch up on if it falls behind.
    /// If it falls further behind than this, the extra time is skipped.
    pub max_catch_up: Duration,
}

impl<S> Default for NevyPredictionServerPlugin<S> {
    fn default() -> Self {
        NevyPredictionServerPlugin {
            _p: PhantomData,
            schedule: Update.intern(),
            execution: default(),
            max_catch_up: Duration::from_millis(500),
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

    pub fn with_execution(mut self, execution: SimulationExecution) -> Self {
        self.execution = execution;
        self
    }
}

impl<S> Plugin for NevyPredictionServerPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        crate::common::build(app);

        app.insert_resource(ServerSimulation::new::<S>(
            self.execution,
            self.max_catch_up,
        ));
        app.init_resource::<ServerSimulationStats>();
        app.init_resource::<ReplayLog>();
        app.init_resource::<Time<PredictionTime>>();

        app.add_shared_message_sender::<SimulationUpdatesStream>(
            StreamRequirements::RELIABLE_ORDERED,
        );

        app.configure_sets(
            self.schedule,
            (
                ServerSimulationSystems::SendResets,
                ServerSimulationSystems::QueueUpdates,
                ServerSimulationSystems::SendReplays,
                SyncSimulationSystems,
            )
                .chain(),
        );

        time_scale::build(app, self.schedule);

        app.add_plugins(SimulationPlugin::<S>::new(SimulationInstance::ServerApp));

        app.add_systems(
            self.schedule,
            (
                send_simulation_resets.in_set(ServerSimulationSystems::SendResets),
                respond_to_tick_probes::<S>.in_set(ServerSimulationSystems::QueueUpdates),
                send_replays.in_set(ServerSimulationSystems::SendReplays),
                (
                    sync_simulation::<S>,
                    send_simulation_time_updates,
                    update_stats,
                )
                    .chain()
                    .in_set(SyncSimulationSystems)
                    .after(time_scale::update_time_scale),
            ),
        );
    }
}

/// Marker type for the simulation updates stream [SharedMessageSender].
pub struct SimulationUpdatesStream;

/// Insert this component onto all clients that are part of the prediction scheme.
///
/// When it is added the client's simulation is reset to the app's current [`SimulationTime`] tick.
/// Systems in [`ServerSimulationSystems::QueueUpdates`] should then send the client the current state of the simulation
/// in the app's world with [`WorldUpdateSender::write_now`].
/// Updates that were queued with [`ServerUpdateQueue`] but aren't in the app's world yet
/// are then sent to it automatically in [`ServerSimulationSystems::SendReplays`].
#[derive(Component)]
pub struct PredictionClient;

/// Inserted on new [`PredictionClient`]s until queued updates have been replayed to them.
#[derive(Component)]
struct PendingReplay {
    /// Updates queued before this sequence number are replayed.
    before: u64,
}

type ReplaySender =
    Box<dyn for<'w, 's> Fn(&mut WorldUpdateSender<'w, 's>, Entity) -> Result<bool> + Send + Sync>;

struct ReplayEntry {
    sequence: u64,
    tick: SimulationTick,
    send: ReplaySender,
}

/// Updates that were queued with [`ServerUpdateQueue`] but haven't been extracted into the app's world yet.
///
/// A client that joins now gets the state of the app's world,
/// so it needs to be sent these updates as well.
#[derive(Resource, Default)]
struct ReplayLog {
    next_sequence: u64,
    entries: VecDeque<ReplayEntry>,
}

impl ReplayLog {
    /// Removes updates that are already part of the app's world.
    fn prune(&mut self, app_tick: SimulationTick) {
        self.entries.retain(|entry| entry.tick >= app_tick);
    }
}

/// Use this system parameter to apply world updates to the server's simulation.
///
/// Updates are applied to the server's simulation thread, so they are applied on a tick after the app's [`SimulationTime`].
/// Updates should normally also be sent to clients with [`WorldUpdateSender::write`],
/// using the update returned by [`Self::queue`] or [`Self::queue_now`] which has the tick it will actually be applied on.
///
/// Queued updates are remembered until they are part of the app's world,
/// and are sent to clients that join before then in [`ServerSimulationSystems::SendReplays`].
/// This means the server can't decide which clients are informed about an update before they join.
#[derive(SystemParam)]
pub struct ServerUpdateQueue<'w, T>
where
    T: Send + Sync + 'static,
{
    simulation: Res<'w, ServerSimulation>,
    log: ResMut<'w, ReplayLog>,
    _p: PhantomData<fn() -> T>,
}

impl<'w, T> ServerUpdateQueue<'w, T>
where
    T: Serialize + Clone + Send + Sync + 'static,
{
    /// Queues an update to be applied to the server's simulation at its tick.
    ///
    /// If the server has already executed that tick, the update is moved to the next tick that it hasn't.
    /// This happens when a client's update arrives late.
    /// The returned update has the tick it will actually be applied on,
    /// and is what should be sent to clients so that they apply it on the same tick.
    pub fn queue(&mut self, update: WorldUpdate<T>) -> WorldUpdate<T> {
        let update = self.simulation.queue(update);

        let replay = update.clone();
        let sequence = self.log.next_sequence;
        self.log.next_sequence += 1;
        self.log.entries.push_back(ReplayEntry {
            sequence,
            tick: update.tick,
            send: Box::new(move |sender, client_entity| {
                sender.write(client_entity, true, true, replay.clone())
            }),
        });

        update
    }

    /// Queues an update to be applied to the server's simulation on the next tick it executes.
    ///
    /// The returned update has the tick it will be applied on, and should be sent to clients.
    pub fn queue_now(&mut self, update: T) -> WorldUpdate<T> {
        let tick = self.simulation.committed_tick();
        self.queue(WorldUpdate { tick, update })
    }
}

fn send_simulation_resets(
    mut commands: Commands,
    new_client_q: Query<Entity, Added<PredictionClient>>,
    time: Res<Time<SimulationTime>>,
    log: Res<ReplayLog>,
    mut messages: SharedMessageSender<SimulationUpdatesStream>,
) -> Result {
    for client_entity in &new_client_q {
        messages.write(
            client_entity,
            true,
            &ResetClientSimulation {
                simulation_tick: time.current_tick(),
            },
        )?;

        commands.entity(client_entity).insert(PendingReplay {
            before: log.next_sequence,
        });
    }

    Ok(())
}

fn send_replays(
    mut commands: Commands,
    client_q: Query<(Entity, &PendingReplay), With<PredictionClient>>,
    log: Res<ReplayLog>,
    mut sender: WorldUpdateSender,
) -> Result {
    for (client_entity, pending) in &client_q {
        for entry in &log.entries {
            if entry.sequence < pending.before {
                (entry.send)(&mut sender, client_entity)?;
            }
        }

        commands.entity(client_entity).remove::<PendingReplay>();
    }

    Ok(())
}

/// Tells clients how early their probes arrived,
/// which they use to adjust how far ahead they predict.
///
/// Runs where world updates from clients are queued so that it measures the same thing.
fn respond_to_tick_probes<S>(
    simulation: Res<ServerSimulation>,
    mut client_q: Query<(Entity, &mut ReceivedMessages<TickProbe>), With<PredictionClient>>,
    mut messages: SharedMessageSender<SimulationUpdatesStream>,
) -> Result
where
    S: PredictionScheme,
{
    let now = simulation.clock().time_at(Instant::now());

    for (client_entity, mut probes) in &mut client_q {
        for TickProbe { simulation_tick } in probes.drain() {
            // a tick is executed once the simulation time passes its end
            let deadline = SimulationTick(*simulation_tick + 1).time::<S>();
            let lead = if deadline >= now {
                (deadline - now).as_secs_f32()
            } else {
                -(now - deadline).as_secs_f32()
            };

            messages.write(
                client_entity,
                true,
                &TickProbeResult {
                    simulation_tick,
                    lead,
                },
            )?;
        }
    }

    Ok(())
}

/// The tick of the app's world when [`UpdateServerTick`] was last sent.
#[derive(Resource, Default)]
struct LastSentTick(Option<SimulationTick>);

/// Extracts the latest state of the simulation into the app and advances [`PredictionTime`].
fn sync_simulation<S>(world: &mut World)
where
    S: PredictionScheme,
{
    world.resource_scope(|world, mut simulation: Mut<ServerSimulation>| {
        simulation.update();

        if !simulation.try_extract(world) {
            return;
        }

        let tick = world.resource::<Time<SimulationTime>>().current_tick();
        world.resource_mut::<ReplayLog>().prune(tick);

        let time_scale = simulation.clock().scale();
        let mut time = world.resource_mut::<Time<PredictionTime>>();
        let latest = tick.time::<S>();
        if latest > time.elapsed() {
            time.advance_to(latest);
        }
        let context = time.context_mut();
        context.time_scale = time_scale;
        context.latest_tick = tick;
        context.fraction = 1.;
    });
}

/// Tells clients the server's simulation has advanced every time a new tick is extracted into the app.
///
/// Every update with an earlier tick has already been sent,
/// because updates are stamped with the app's tick or later.
fn send_simulation_time_updates(
    time: Res<Time<SimulationTime>>,
    simulation: Res<ServerSimulation>,
    mut last_sent: Local<LastSentTick>,
    client_q: Query<Entity, With<PredictionClient>>,
    mut messages: SharedMessageSender<SimulationUpdatesStream>,
) -> Result {
    let tick = time.current_tick();

    if last_sent.0 == Some(tick) {
        return Ok(());
    }
    last_sent.0 = Some(tick);

    let clock = simulation.clock();
    let message = UpdateServerTick {
        simulation_tick: tick,
        time: clock.time_at(Instant::now()),
        time_scale: clock.scale(),
    };

    for client_entity in &client_q {
        messages.write(client_entity, true, &message)?;
    }

    Ok(())
}

fn update_stats(simulation: Res<ServerSimulation>, mut stats: ResMut<ServerSimulationStats>) {
    *stats = simulation.stats();
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
    /// Sends a [`nevy`] message containing a [`WorldUpdate`] with the app's current simulation tick.
    ///
    /// The app's world holds the state of the simulation at the start of this tick,
    /// so this is used for informing clients of the state of the simulation that the app can see,
    /// such as sending the initial state to a new client or periodically correcting state.
    ///
    /// This does not apply the update to the server's simulation, use [`ServerUpdateQueue`] for that.
    ///
    /// Note there is no `include_in_prediction` argument like there is in [`Self::write`],
    /// because updates sent to clients by this method will immediately be applied.
    pub fn write_now<T>(&mut self, client_entity: Entity, queue: bool, update: T) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        self.write(
            client_entity,
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
    /// This method would normally be used to inform clients of updates queued with [`ServerUpdateQueue`],
    /// or of updates that a client requested which were reconciled.
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
        queue: bool,
        include_in_prediction: bool,
        update: WorldUpdate<T>,
    ) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        self.sender.write(
            client_entity,
            queue,
            &ServerWorldUpdate {
                update,
                include_in_prediction,
            },
        )
    }

    /// Gets the underlying [`SharedMessageSender`], for stream operations.
    pub fn sender(&mut self) -> &mut SharedMessageSender<'w, 's, SimulationUpdatesStream> {
        &mut self.sender
    }
}
