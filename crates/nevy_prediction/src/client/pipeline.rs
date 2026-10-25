//! The client's simulation lanes.
//!
//! The client has three stepping simulation worlds, each stepped by its own lane:
//!
//! - **main** steps the [`ClientMain`](SimulationInstance::ClientMain) world as real time passes,
//!   so that it stays [`PredictionInterval`](crate::client::interval::PredictionInterval) ahead of the server.
//!   After every tick it extracts the main world into a snapshot world that the app extracts from.
//!   When a prediction finishes it extracts the prediction world into the main world.
//! - **template** steps the [`ClientTemplate`](SimulationInstance::ClientTemplate) world up to the latest tick the server has finished.
//! - **prediction** forks the [`ClientPrediction`](SimulationInstance::ClientPrediction) world from the template whenever the template advances,
//!   then steps it up to the main world's committed tick, which is the tick the main world is executing or will execute next.
//!   The main world extracts it once it reaches the same tick.
//!
//! The template and prediction lanes are limited by a budget that is granted for every tick of the main world,
//! see [`PredictionRates`].
//!
//! World lock order: main, prediction, template, snapshot.

use std::{marker::PhantomData, sync::Arc, time::Duration};

use bevy::{platform::time::Instant, prelude::*};
use tracing::info_span;

use crate::{
    client::{
        PredictionRates,
        prediction::{LastPredictedTick, PredictionUpdates},
    },
    common::{
        clock::SimulationClock,
        runtime::{
            CostTracker, Lane, LaneDriver, LaneMeter, LaneStats, LaneStatus, Shared,
            SimulationExecution, WorldCell, WorldJob, apply_jobs, lock_world,
        },
        scheme::PredictionScheme,
        simulation::{
            PrivateSimulationTimeExt, SimulationInstance, SimulationTick, SimulationTime,
            UpdateExecutionQueue, WorldUpdate,
            world::{SimulationWorld, SnapshotSequence, build_simulation_world},
        },
    },
};

/// Grants budget for one main world tick.
///
/// Unspent budget only carries over up to one tick's worth, so that lanes that are idle don't build up a large burst.
fn grant_budget(budget: &mut f32, rate: f32) {
    *budget = (*budget + rate).min(rate.max(1.));
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PredictionStatus {
    /// Waiting for the template to advance.
    Idle,
    /// Stepping towards the main world's committed tick.
    Running,
    /// Reached the main world's committed tick and is waiting for the main world to reach it to be extracted.
    Finished,
}

pub(crate) struct ClientState {
    /// False until the server resets the simulation.
    pub active: bool,
    /// Maps real time to the main world's target simulation time.
    pub clock: Option<SimulationClock>,
    pub rates: PredictionRates,
    pub main: MainState,
    pub template: TemplateState,
    pub prediction: PredictionState,
}

#[derive(Default)]
pub(crate) struct MainState {
    pub inbox: Vec<WorldJob>,
    /// The next tick that hasn't been decided yet.
    /// Updates for ticks before this can no longer be applied to the main world on time.
    pub committed: SimulationTick,
    /// The next tick that the main world will execute.
    pub tick: SimulationTick,
    pub meter: LaneMeter,
}

#[derive(Default)]
pub(crate) struct TemplateState {
    pub inbox: Vec<WorldJob>,
    /// The next tick that the template world will execute.
    pub tick: SimulationTick,
    /// The latest tick from the server, the template steps up to this.
    pub target: SimulationTick,
    pub budget: f32,
    pub meter: LaneMeter,
}

pub(crate) struct PredictionState {
    pub inbox: Vec<WorldJob>,
    pub status: PredictionStatus,
    /// The next tick that the prediction world will execute.
    pub tick: SimulationTick,
    /// True while the prediction lane is executing a tick.
    pub stepping: bool,
    pub budget: f32,
    pub last_fork: Option<SimulationTick>,
    pub meter: LaneMeter,
    pub fork_cost: CostTracker,
    /// Average number of ticks per prediction.
    pub run_length: Option<f32>,
    /// Number of predictions that were extracted into the main world.
    pub completed: u64,
}

impl Default for PredictionState {
    fn default() -> Self {
        PredictionState {
            inbox: Vec::new(),
            status: PredictionStatus::Idle,
            tick: SimulationTick(0),
            stepping: false,
            budget: 0.,
            last_fork: None,
            meter: LaneMeter::default(),
            fork_cost: CostTracker::default(),
            run_length: None,
            completed: 0,
        }
    }
}

impl ClientState {
    fn new(rates: PredictionRates) -> Self {
        ClientState {
            active: false,
            clock: None,
            rates,
            main: default(),
            template: default(),
            prediction: default(),
        }
    }

    /// The tick that an update created now should be applied at.
    ///
    /// This is the next tick the main world's clock will reach,
    /// unless the main world has already committed to executing it.
    fn stamp(&self, now: Instant, step: Duration) -> SimulationTick {
        let clock_tick = self
            .clock
            .map(|clock| clock.tick_at(now, step))
            .unwrap_or_default();

        clock_tick.max(self.main.committed)
    }
}

pub(crate) struct ClientShared {
    pub sync: Arc<Shared<ClientState>>,
    pub main: WorldCell,
    pub prediction: WorldCell,
    pub template: WorldCell,
    pub snapshot: WorldCell,
}

/// Statistics about the client's simulation, updated every frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct ClientSimulationStats {
    /// False until the server has reset the client's simulation.
    pub active: bool,
    /// The server's time scale, see [`PredictionTime`](crate::common::time::PredictionTime).
    pub time_scale: f32,
    /// The fastest time scale this client estimates it could sustain, reported to the server.
    pub capacity: Option<f32>,
    pub main: LaneStats,
    pub template: LaneStats,
    pub prediction: LaneStats,
    /// How many ticks the main world is behind where its clock says it should be.
    pub main_lag: u32,
    /// How many ticks the template world is behind the latest tick from the server.
    pub template_lag: u32,
    /// Number of predictions that were extracted into the main world.
    pub predictions_completed: u64,
    /// Average number of ticks per prediction.
    pub prediction_length: Option<f32>,
    /// Average time to fork the prediction world from the template world.
    pub fork_cost: Option<Duration>,
}

/// The client's simulation, which runs asynchronously to the app.
///
/// Use this to modify the simulation worlds outside of the normal world update flow,
/// such as loading data slowly where the tradeoff of a moment of desync is preferable to a large hitch.
#[derive(Resource)]
pub struct ClientSimulation {
    shared: Arc<ClientShared>,
    driver: LaneDriver<ClientState>,
    last_snapshot: SnapshotSequence,
    step: Duration,
}

impl ClientSimulation {
    pub(crate) fn new<S>(execution: SimulationExecution, rates: PredictionRates) -> Self
    where
        S: PredictionScheme,
    {
        let shared = Arc::new(ClientShared {
            sync: Arc::new(Shared::new(ClientState::new(rates))),
            main: WorldCell::new(build_simulation_world::<S>(SimulationInstance::ClientMain)),
            prediction: WorldCell::new(build_simulation_world::<S>(
                SimulationInstance::ClientPrediction,
            )),
            template: WorldCell::new(build_simulation_world::<S>(
                SimulationInstance::ClientTemplate,
            )),
            snapshot: WorldCell::new(build_simulation_world::<S>(SimulationInstance::Snapshot)),
        });

        let lanes: Vec<Box<dyn Lane>> = vec![
            Box::new(MainLane::<S> {
                shared: shared.clone(),
                _p: PhantomData,
            }),
            Box::new(TemplateLane::<S> {
                shared: shared.clone(),
                _p: PhantomData,
            }),
            Box::new(PredictionLane::<S> {
                shared: shared.clone(),
                _p: PhantomData,
            }),
        ];

        ClientSimulation {
            driver: LaneDriver::new(shared.sync.clone(), execution, lanes),
            shared,
            last_snapshot: SnapshotSequence(0),
            step: S::step_interval(),
        }
    }

    /// Queues a function to run on the template world before it next ticks.
    ///
    /// Changes made to the template world will be predicted from the next time the prediction world is forked.
    pub fn run_on_template(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        let mut state = self.shared.sync.lock();
        state.template.inbox.push(Box::new(f));
        state.notify();
    }

    /// Queues a function to run on the main world before it next ticks.
    ///
    /// Changes made to the main world will be overwritten the next time a prediction finishes.
    pub fn run_on_main(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        let mut state = self.shared.sync.lock();
        state.main.inbox.push(Box::new(f));
        state.notify();
    }

    /// Queues a function to run on the prediction world before it next ticks or forks.
    pub fn run_on_prediction(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        let mut state = self.shared.sync.lock();
        state.prediction.inbox.push(Box::new(f));
        state.notify();
    }

    /// Queues a function to run on every stepping simulation world on the client.
    ///
    /// This is useful for configuring resources that every world needs.
    pub fn run_on_all(&self, f: impl Fn(&mut World) + Send + Sync + 'static) {
        let f = Arc::new(f);
        let job = |f: Arc<dyn Fn(&mut World) + Send + Sync>| -> WorldJob {
            Box::new(move |world: &mut World| f(world))
        };

        let mut state = self.shared.sync.lock();
        state.main.inbox.push(job(f.clone()));
        state.template.inbox.push(job(f.clone()));
        state.prediction.inbox.push(job(f));
        state.notify();
    }

    /// The tick that an update created now should be applied at.
    pub(crate) fn stamp(&self) -> SimulationTick {
        self.shared.sync.lock().stamp(Instant::now(), self.step)
    }

    /// Stamps a locally created update, and queues it for the main and prediction worlds.
    pub(crate) fn queue_local_update<T>(&self, update: T) -> WorldUpdate<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        let mut state = self.shared.sync.lock();

        let update = WorldUpdate {
            tick: state.stamp(Instant::now(), self.step),
            update,
        };

        let main_update = update.clone();
        state.main.inbox.push(Box::new(move |world: &mut World| {
            world
                .resource_mut::<UpdateExecutionQueue<T>>()
                .insert(main_update);
        }));

        let prediction_update = update.clone();
        state.prediction.inbox.push(Box::new(move |world: &mut World| {
            world
                .resource_mut::<PredictionUpdates<T>>()
                .insert(prediction_update);
        }));

        state.notify();

        update
    }

    /// Queues updates from the server for the template world, and the prediction world if `include_in_prediction` is set.
    pub(crate) fn queue_server_updates<T>(&self, updates: Vec<(WorldUpdate<T>, bool)>)
    where
        T: Clone + Send + Sync + 'static,
    {
        if updates.is_empty() {
            return;
        }

        let mut prediction_updates = Vec::new();
        let mut template_updates = Vec::with_capacity(updates.len());

        for (update, include_in_prediction) in updates {
            if include_in_prediction {
                prediction_updates.push(update.clone());
            }

            template_updates.push(update);
        }

        let mut state = self.shared.sync.lock();

        state.template.inbox.push(Box::new(move |world: &mut World| {
            let mut queue = world.resource_mut::<UpdateExecutionQueue<T>>();
            for update in template_updates {
                queue.insert(update);
            }
        }));

        if !prediction_updates.is_empty() {
            state.prediction.inbox.push(Box::new(move |world: &mut World| {
                let mut queue = world.resource_mut::<PredictionUpdates<T>>();
                for update in prediction_updates {
                    queue.insert(update);
                }
            }));
        }

        state.notify();
    }

    /// Updates the main world's clock, the template's target and the prediction rates.
    pub(crate) fn set_targets(
        &self,
        clock: SimulationClock,
        template_target: SimulationTick,
        rates: &PredictionRates,
    ) {
        let mut state = self.shared.sync.lock();

        state.clock = Some(clock);
        state.template.target = template_target;
        state.rates = rates.clone();

        state.notify();
    }

    pub(crate) fn clock(&self) -> Option<SimulationClock> {
        self.shared.sync.lock().clock
    }

    /// Resets every simulation world to a tick.
    ///
    /// Jobs queued with `run_on_*` before the reset are applied first, so they aren't lost.
    ///
    /// This blocks until the simulation lanes finish what they are doing.
    pub(crate) fn reset<S>(&mut self, tick: SimulationTick)
    where
        S: PredictionScheme,
    {
        let shared = &*self.shared;

        let mut main = lock_world(&shared.main);
        let mut prediction = lock_world(&shared.prediction);
        let mut template = lock_world(&shared.template);
        let mut snapshot = lock_world(&shared.snapshot);

        // Apply jobs queued before the reset, such as configuration from `run_on_main`.
        // World updates in them are cleared by the reset.
        {
            let mut state = shared.sync.lock();
            apply_jobs(main.world_mut(), std::mem::take(&mut state.main.inbox));
            apply_jobs(
                prediction.world_mut(),
                std::mem::take(&mut state.prediction.inbox),
            );
            apply_jobs(
                template.world_mut(),
                std::mem::take(&mut state.template.inbox),
            );
        }

        main.reset::<S>(tick);
        prediction.reset::<S>(tick);
        prediction.world_mut().insert_resource(LastPredictedTick(tick));
        template.reset::<S>(tick);
        snapshot.reset::<S>(tick);

        self.last_snapshot = *snapshot.world().resource::<SnapshotSequence>();

        let mut state = shared.sync.lock();
        shared.sync.increment_generation();

        let rates = state.rates.clone();
        state.data = ClientState {
            active: true,
            main: MainState {
                committed: tick,
                tick,
                ..default()
            },
            template: TemplateState {
                tick,
                target: tick,
                ..default()
            },
            prediction: PredictionState {
                tick,
                ..default()
            },
            ..ClientState::new(rates)
        };

        state.notify();
    }

    /// Runs inline lanes, and propagates panics from simulation threads.
    pub(crate) fn update(&mut self) {
        self.driver.update();
    }

    /// Extracts the latest state of the main world into the app's world if there is a new one.
    ///
    /// Doesn't block if a simulation thread is busy with the snapshot.
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

    /// The fastest time scale that this client could sustain, based on how long the simulation lanes take to execute ticks.
    ///
    /// The main and template worlds need to execute ticks as fast as the simulation advances,
    /// and the prediction world needs to execute them [`PredictionRates::min_prediction_speed`] times faster.
    fn capacity(state: &ClientState, step: Duration) -> Option<f32> {
        let step = step.as_secs_f32();

        let main = state.main.meter.cost.average()?.as_secs_f32();
        let mut capacity = step / main;

        if let Some(template) = state.template.meter.cost.average() {
            capacity = capacity.min(step / template.as_secs_f32());
        }

        if let Some(prediction) = state.prediction.meter.cost.average() {
            // forking is part of the cost of every prediction
            let fork = state
                .prediction
                .fork_cost
                .average()
                .unwrap_or_default()
                .as_secs_f32();
            let length = state.prediction.run_length.unwrap_or(1.).max(1.);
            let cost = prediction.as_secs_f32() + fork / length;

            capacity = capacity.min(step / (cost * state.rates.min_prediction_speed.max(1.)));
        }

        Some(capacity)
    }

    pub(crate) fn stats(&self, template_target: SimulationTick) -> ClientSimulationStats {
        let state = self.shared.sync.lock();
        let now = Instant::now();

        let main_lag = state
            .clock
            .map(|clock| clock.tick_at(now, self.step))
            .map(|tick| tick.saturating_sub(*state.main.tick))
            .unwrap_or(0);

        ClientSimulationStats {
            active: state.active,
            time_scale: state.clock.map(|clock| clock.scale()).unwrap_or(1.),
            capacity: Self::capacity(&state, self.step),
            main: state.main.meter.stats(),
            template: state.template.meter.stats(),
            prediction: state.prediction.meter.stats(),
            main_lag,
            template_lag: template_target.saturating_sub(*state.template.tick),
            predictions_completed: state.prediction.completed,
            prediction_length: state.prediction.run_length,
            fork_cost: state.prediction.fork_cost.average(),
        }
    }
}

/// Steps the main world as real time passes, and extracts finished predictions into it.
struct MainLane<S> {
    shared: Arc<ClientShared>,
    _p: PhantomData<fn() -> S>,
}

enum MainDecision {
    ExtractPrediction,
    Tick(Vec<WorldJob>),
}

impl<S> MainLane<S>
where
    S: PredictionScheme,
{
    /// Extracts the main world into the snapshot world for the app to extract.
    fn update_snapshot(&self, main: &mut SimulationWorld) {
        let _span = info_span!("update snapshot").entered();

        let mut snapshot = lock_world(&self.shared.snapshot);
        snapshot.extract_from(main);
        **snapshot.world_mut().resource_mut::<SnapshotSequence>() += 1;
    }
}

impl<S> Lane for MainLane<S>
where
    S: PredictionScheme,
{
    fn name(&self) -> &'static str {
        "client main"
    }

    fn work(&mut self) -> LaneStatus {
        let (decision, generation) = {
            let mut state = self.shared.sync.lock();

            if !state.active {
                return state.wait(None);
            }

            let generation = self.shared.sync.generation();

            let decision = 'decision: {
                if let PredictionStatus::Finished = state.prediction.status {
                    if state.prediction.tick == state.main.tick {
                        break 'decision MainDecision::ExtractPrediction;
                    }

                    if state.prediction.tick < state.main.tick {
                        // The main world advanced while the prediction was finishing.
                        state.prediction.status = PredictionStatus::Running;
                        state.notify();
                    }
                }

                let Some(clock) = state.clock else {
                    return state.wait(None);
                };

                let step = S::step_interval();
                let due = clock.tick_at(Instant::now(), step);

                if state.main.committed >= due {
                    return state.wait(clock.tick_deadline(state.main.committed, step));
                }

                // If the main world is behind its clock it ticks back to back,
                // and a prediction that ticks as fast as it does would never catch up.
                // So if the prediction is executing the tick that catches it up, wait for it instead of moving the target.
                // When the main world is on time the prediction catches up while it waits for its next tick.
                let catching_up = *due > *state.main.committed + 1;
                if catching_up
                    && state.prediction.stepping
                    && *state.prediction.tick + 1 == *state.main.tick
                {
                    return state.wait(None);
                }

                *state.main.committed += 1;
                MainDecision::Tick(std::mem::take(&mut state.main.inbox))
            };

            (decision, generation)
        };

        let start = Instant::now();
        let mut main = lock_world(&self.shared.main);

        if self.shared.sync.generation() != generation {
            // The simulation was reset since this decision was made.
            return LaneStatus::Worked;
        }

        match decision {
            MainDecision::ExtractPrediction => {
                {
                    let _span = info_span!("extract prediction").entered();
                    let mut prediction = lock_world(&self.shared.prediction);
                    main.extract_from(&mut prediction);
                }

                self.update_snapshot(&mut main);
                drop(main);

                let mut state = self.shared.sync.lock();
                state.prediction.status = PredictionStatus::Idle;
                state.prediction.completed += 1;
                state.main.meter.record_busy(start.elapsed());
                state.notify();
            }
            MainDecision::Tick(jobs) => {
                apply_jobs(main.world_mut(), jobs);
                main.step::<S>();
                let tick = main.tick();

                self.update_snapshot(&mut main);
                drop(main);

                let mut state = self.shared.sync.lock();
                state.main.tick = tick;
                state.main.meter.record_tick(start.elapsed());

                let rates = state.rates.clone();
                grant_budget(&mut state.template.budget, rates.template);
                grant_budget(&mut state.prediction.budget, rates.prediction);

                state.notify();
            }
        }

        LaneStatus::Worked
    }
}

/// Steps the template world up to the latest tick from the server.
struct TemplateLane<S> {
    shared: Arc<ClientShared>,
    _p: PhantomData<fn() -> S>,
}

impl<S> Lane for TemplateLane<S>
where
    S: PredictionScheme,
{
    fn name(&self) -> &'static str {
        "client template"
    }

    fn work(&mut self) -> LaneStatus {
        let (jobs, step, generation) = {
            let mut state = self.shared.sync.lock();

            if !state.active {
                return state.wait(None);
            }

            let jobs = std::mem::take(&mut state.template.inbox);

            let step =
                state.template.tick < state.template.target && state.template.budget >= 1.;

            if step {
                state.template.budget -= 1.;
            }

            if jobs.is_empty() && !step {
                return state.wait(None);
            }

            (jobs, step, self.shared.sync.generation())
        };

        let start = Instant::now();
        let mut template = lock_world(&self.shared.template);

        if self.shared.sync.generation() != generation {
            return LaneStatus::Worked;
        }

        apply_jobs(template.world_mut(), jobs);

        if step {
            template.step::<S>();
        }

        let tick = template.tick();
        drop(template);

        let mut state = self.shared.sync.lock();
        state.template.tick = tick;

        if step {
            state.template.meter.record_tick(start.elapsed());
        } else {
            state.template.meter.record_busy(start.elapsed());
        }

        state.notify();

        LaneStatus::Worked
    }
}

/// Forks the prediction world from the template world and steps it up to the main world.
struct PredictionLane<S> {
    shared: Arc<ClientShared>,
    _p: PhantomData<fn() -> S>,
}

enum PredictionDecision {
    None,
    Fork,
    Step,
}

impl<S> Lane for PredictionLane<S>
where
    S: PredictionScheme,
{
    fn name(&self) -> &'static str {
        "client prediction"
    }

    fn work(&mut self) -> LaneStatus {
        let (jobs, decision, generation) = {
            let mut state = self.shared.sync.lock();

            if !state.active {
                return state.wait(None);
            }

            let jobs = std::mem::take(&mut state.prediction.inbox);

            let decision = match state.prediction.status {
                PredictionStatus::Idle => {
                    if state.prediction.last_fork != Some(state.template.tick) {
                        state.prediction.status = PredictionStatus::Running;
                        PredictionDecision::Fork
                    } else {
                        PredictionDecision::None
                    }
                }
                PredictionStatus::Running => {
                    // Ticks before the main world's committed tick won't get any more local updates,
                    // so the prediction can execute the tick that the main world is executing right now.
                    // It then finishes just before the main world gets there, instead of racing it.
                    if state.prediction.budget >= 1.
                        && state.prediction.tick < state.main.committed
                    {
                        state.prediction.budget -= 1.;
                        state.prediction.stepping = true;
                        PredictionDecision::Step
                    } else {
                        PredictionDecision::None
                    }
                }
                PredictionStatus::Finished => PredictionDecision::None,
            };

            if jobs.is_empty() && matches!(decision, PredictionDecision::None) {
                return state.wait(None);
            }

            (jobs, decision, self.shared.sync.generation())
        };

        let start = Instant::now();
        let mut prediction = lock_world(&self.shared.prediction);

        if self.shared.sync.generation() != generation {
            return LaneStatus::Worked;
        }

        apply_jobs(prediction.world_mut(), jobs);


        match decision {
            PredictionDecision::None => {}
            PredictionDecision::Fork => {
                let _span = info_span!("fork prediction").entered();

                {
                    let mut template = lock_world(&self.shared.template);
                    prediction.extract_from(&mut template);
                }

                let fork_tick = prediction.tick();
                let world = prediction.world_mut();
                world.insert_resource(LastPredictedTick(fork_tick));
                world
                    .resource_mut::<Time<SimulationTime>>()
                    .clear_target();
            }
            PredictionDecision::Step => {
                prediction.step::<S>();
            }
        }

        let tick = prediction.tick();
        drop(prediction);

        let mut state = self.shared.sync.lock();
        state.prediction.tick = tick;
        state.prediction.stepping = false;

        match decision {
            PredictionDecision::None => state.prediction.meter.record_busy(start.elapsed()),
            PredictionDecision::Fork => {
                state.prediction.last_fork = Some(tick);
                state.prediction.fork_cost.record(start.elapsed());
                state.prediction.meter.record_busy(start.elapsed());
            }
            PredictionDecision::Step => state.prediction.meter.record_tick(start.elapsed()),
        }

        if state.prediction.status == PredictionStatus::Running && tick >= state.main.committed {
            state.prediction.status = PredictionStatus::Finished;

            if let Some(fork) = state.prediction.last_fork {
                let length = tick.saturating_sub(*fork) as f32;
                state.prediction.run_length = Some(match state.prediction.run_length {
                    Some(average) => average + (length - average) * 0.05,
                    None => length,
                });
            }
        }

        state.notify();

        LaneStatus::Worked
    }
}
