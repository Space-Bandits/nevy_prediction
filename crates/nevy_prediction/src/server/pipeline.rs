//! The server's simulation lane.
//!
//! The authoritative [`Server`](SimulationInstance::Server) world is stepped on its own thread as its clock advances.
//! The clock runs at the server's time scale, which is lowered when the server or a client can't keep up.
//! After every tick the world is extracted into a snapshot world that the app extracts from.
//!
//! World lock order: server, snapshot.

use std::{marker::PhantomData, sync::Arc, time::Duration};

use bevy::{platform::time::Instant, prelude::*};
use tracing::{info_span, warn};

use crate::{
    common::{
        clock::SimulationClock,
        runtime::{
            Lane, LaneDriver, LaneMeter, LaneStats, LaneStatus, Shared, SimulationExecution,
            WorldCell, WorldJob, apply_jobs, lock_world,
        },
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationTick, UpdateExecutionQueue, WorldUpdate,
            world::{SimulationWorld, SnapshotSequence, build_simulation_world},
        },
    },
};

pub(crate) struct ServerState {
    /// False until the app first synchronizes with the simulation, so that the clock starts when the app does.
    active: bool,
    clock: SimulationClock,
    inbox: Vec<WorldJob>,
    /// The next tick that hasn't been decided yet.
    /// Updates for ticks before this can no longer be applied on time.
    committed: SimulationTick,
    /// The next tick that the server world will execute.
    tick: SimulationTick,
    meter: LaneMeter,
    /// The most ticks the server will execute back to back to catch up before it skips time instead.
    max_catch_up: u32,
    /// Simulation time that was skipped because the server fell too far behind.
    skipped: Duration,
    /// Updates that were queued for a tick that had already been executed.
    late_updates: u64,
}

pub(crate) struct ServerShared {
    sync: Arc<Shared<ServerState>>,
    world: WorldCell,
    snapshot: WorldCell,
}

/// Statistics about the server's simulation, updated every frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct ServerSimulationStats {
    /// How many simulation seconds pass per real second.
    pub time_scale: f32,
    /// The fastest time scale the server estimates it could sustain.
    pub capacity: Option<f32>,
    pub lane: LaneStats,
    /// How many ticks the server world is behind its clock.
    pub lag: u32,
    /// Simulation time that was skipped because the server fell too far behind.
    pub skipped: Duration,
    /// Updates that were queued for a tick that had already been executed, so were applied late.
    pub late_updates: u64,
}

/// The server's simulation, which runs asynchronously to the app.
///
/// Use [`ServerUpdateQueue`](crate::server::ServerUpdateQueue) to apply world updates to it,
/// and [`Self::run_on_world`] to modify it outside of the normal world update flow.
#[derive(Resource)]
pub struct ServerSimulation {
    shared: Arc<ServerShared>,
    driver: LaneDriver<ServerState>,
    last_snapshot: SnapshotSequence,
    step: Duration,
}

impl ServerSimulation {
    pub(crate) fn new<S>(execution: SimulationExecution, max_catch_up: Duration) -> Self
    where
        S: PredictionScheme,
    {
        let step = S::step_interval();

        let shared = Arc::new(ServerShared {
            sync: Arc::new(Shared::new(ServerState {
                active: false,
                clock: SimulationClock::new(Instant::now(), Duration::ZERO, 1.),
                inbox: Vec::new(),
                committed: SimulationTick(0),
                tick: SimulationTick(0),
                meter: LaneMeter::default(),
                max_catch_up: (max_catch_up.as_secs_f64() / step.as_secs_f64()).ceil() as u32,
                skipped: Duration::ZERO,
                late_updates: 0,
            })),
            world: WorldCell::new(build_simulation_world::<S>(SimulationInstance::Server)),
            snapshot: WorldCell::new(build_simulation_world::<S>(SimulationInstance::Snapshot)),
        });

        let lanes: Vec<Box<dyn Lane>> = vec![Box::new(ServerLane::<S> {
            shared: shared.clone(),
            _p: PhantomData,
        })];

        ServerSimulation {
            driver: LaneDriver::new(shared.sync.clone(), execution, lanes),
            shared,
            last_snapshot: SnapshotSequence(0),
            step,
        }
    }

    /// Queues a function to run on the server's simulation world before it next ticks.
    ///
    /// Changes made this way are not sent to clients.
    pub fn run_on_world(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        let mut state = self.shared.sync.lock();
        state.inbox.push(Box::new(f));
        state.notify();
    }

    /// Queues an update to be applied to the server's simulation.
    ///
    /// If its tick has already been executed, it is moved to the next tick that will be.
    pub(crate) fn queue<T>(&self, mut update: WorldUpdate<T>) -> WorldUpdate<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        let mut state = self.shared.sync.lock();

        if update.tick < state.committed {
            update.tick = state.committed;
            state.late_updates += 1;
        }

        let queued = update.clone();
        state.inbox.push(Box::new(move |world: &mut World| {
            world
                .resource_mut::<UpdateExecutionQueue<T>>()
                .insert(queued);
        }));

        state.notify();

        update
    }

    /// The next tick that an update can be applied at.
    pub(crate) fn committed_tick(&self) -> SimulationTick {
        self.shared.sync.lock().committed
    }

    pub(crate) fn clock(&self) -> SimulationClock {
        self.shared.sync.lock().clock
    }

    pub(crate) fn set_time_scale(&self, now: Instant, time_scale: f32) {
        let mut state = self.shared.sync.lock();
        state.clock.rescale(now, time_scale);
        state.notify();
    }

    /// Starts the clock if it hasn't started, runs inline lanes and propagates panics from the simulation thread.
    pub(crate) fn update(&mut self) {
        {
            let mut state = self.shared.sync.lock();

            if !state.active {
                state.active = true;
                let time = self.step * *state.tick;
                state.clock = SimulationClock::new(Instant::now(), time, state.clock.scale());
                state.notify();
            }
        }

        self.driver.update();
    }

    /// Extracts the latest state of the server world into the app's world if there is a new one.
    ///
    /// Doesn't block if the simulation thread is busy with the snapshot.
    pub(crate) fn try_extract(&mut self, target: &mut World) -> bool {
        let mut snapshot = match self.shared.snapshot.try_lock() {
            Ok(snapshot) => snapshot,
            Err(std::sync::TryLockError::WouldBlock) => return false,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                panic!("a simulation thread panicked while holding a simulation world")
            }
        };

        let sequence = *snapshot.world().resource::<SnapshotSequence>();

        if sequence == self.last_snapshot {
            return false;
        }

        self.last_snapshot = sequence;
        snapshot.extract_into(target);

        true
    }

    /// The fastest time scale the server could sustain, based on how long its ticks take.
    pub(crate) fn capacity(&self) -> Option<f32> {
        let cost = self.shared.sync.lock().meter.cost.average()?;
        Some(self.step.as_secs_f32() / cost.as_secs_f32())
    }

    pub(crate) fn stats(&self) -> ServerSimulationStats {
        let state = self.shared.sync.lock();

        let lag = state
            .clock
            .tick_at(Instant::now(), self.step)
            .saturating_sub(*state.tick);

        ServerSimulationStats {
            time_scale: state.clock.scale(),
            capacity: state
                .meter
                .cost
                .average()
                .map(|cost| self.step.as_secs_f32() / cost.as_secs_f32()),
            lane: state.meter.stats(),
            lag,
            skipped: state.skipped,
            late_updates: state.late_updates,
        }
    }
}

/// Steps the server world as its clock advances.
struct ServerLane<S> {
    shared: Arc<ServerShared>,
    _p: PhantomData<fn() -> S>,
}

impl<S> ServerLane<S>
where
    S: PredictionScheme,
{
    fn update_snapshot(&self, world: &mut SimulationWorld) {
        let _span = info_span!("update snapshot").entered();

        let mut snapshot = lock_world(&self.shared.snapshot);
        snapshot.extract_from(world);
        **snapshot.world_mut().resource_mut::<SnapshotSequence>() += 1;
    }
}

impl<S> Lane for ServerLane<S>
where
    S: PredictionScheme,
{
    fn name(&self) -> &'static str {
        "server"
    }

    fn work(&mut self) -> LaneStatus {
        let jobs = {
            let mut state = self.shared.sync.lock();

            if !state.active {
                return state.wait(None);
            }

            let step = S::step_interval();
            let now = Instant::now();
            let due = state.clock.tick_at(now, step);

            if *due > *state.committed + state.max_catch_up {
                // Fell too far behind to catch up, skip time so that only the next tick is due.
                let skip = step * (*due - *state.committed - 1);
                warn!(
                    "The server simulation fell {} ticks behind, skipping {:?} of simulation time",
                    *due - *state.committed,
                    skip
                );

                state.skipped += skip;
                let scale = state.clock.scale();
                state.clock = SimulationClock::new(now, step * (*state.committed + 1), scale);
            }

            if state.committed >= state.clock.tick_at(now, step) {
                return state.wait(state.clock.tick_deadline(state.committed, step));
            }

            *state.committed += 1;
            std::mem::take(&mut state.inbox)
        };

        let start = Instant::now();
        let mut world = lock_world(&self.shared.world);

        apply_jobs(world.world_mut(), jobs);
        world.step::<S>();
        let tick = world.tick();

        self.update_snapshot(&mut world);
        drop(world);

        let mut state = self.shared.sync.lock();
        state.tick = tick;
        state.meter.record_tick(start.elapsed());
        state.notify();

        LaneStatus::Worked
    }
}
