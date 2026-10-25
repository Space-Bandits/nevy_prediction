//! Runs simulation work off of the app's main thread.
//!
//! Simulation work is split into lanes. A lane owns one kind of work, such as stepping one simulation world,
//! and is run on its own thread. Lanes share state through a mutex and wake each other up with a condvar.
//!
//! Lanes still run schedules with the normal multi-threaded executor,
//! so their systems are spread across the [`ComputeTaskPool`](bevy::tasks::ComputeTaskPool) like the app's own systems.
//! The lane thread itself only runs exclusive systems and waits for the rest,
//! the same as the app's main thread does.
//!
//! [`SimulationExecution::Inline`] runs lanes on the app's main thread instead, for platforms without threads and for debugging.
//!
//! Lock order: lanes never lock a world while holding the shared state lock.
//! The state lock may be taken while holding world locks.

use std::{
    panic::{AssertUnwindSafe, resume_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use bevy::{platform::time::Instant, prelude::*};
use tracing::info_span;

use crate::common::simulation::world::SimulationWorld;

/// A deferred mutation of a simulation world.
pub(crate) type WorldJob = Box<dyn FnOnce(&mut World) + Send>;

/// Applies queued jobs to a world.
pub(crate) fn apply_jobs(world: &mut World, jobs: Vec<WorldJob>) {
    for job in jobs {
        job(world);
    }
}

/// Controls where simulation work is executed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SimulationExecution {
    /// Each simulation lane runs on its own thread, asynchronously to the app.
    #[default]
    Threaded,
    /// Simulation lanes are run on the app's main thread when the app synchronizes with the simulation.
    ///
    /// This blocks the app while the simulation runs, and ticks only start when the app updates.
    /// It is meant for platforms without threads, and for debugging.
    Inline,
}

/// What a lane wants to do after it was given a chance to work.
pub(crate) enum LaneStatus {
    /// The lane did some work and should be run again immediately.
    Worked,
    /// The lane has nothing to do until it is signalled or until a deadline.
    ///
    /// `epoch` is the [`Shared`] epoch that the lane observed when it decided to wait.
    /// If it changed since, the lane is run again immediately.
    Wait {
        epoch: u64,
        until: Option<Instant>,
    },
}

/// A unit of simulation work, run repeatedly by a lane driver.
pub(crate) trait Lane: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn work(&mut self) -> LaneStatus;
}

pub(crate) struct SharedInner<T> {
    /// Incremented every time a lane should re-evaluate what to do.
    epoch: u64,
    shutdown: bool,
    pub data: T,
}

impl<T> std::ops::Deref for SharedInner<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.data
    }
}

impl<T> std::ops::DerefMut for SharedInner<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.data
    }
}

impl<T> SharedInner<T> {
    /// Wakes up lanes so that they re-evaluate the state.
    ///
    /// The condvar is notified by [`SharedGuard`] when it is dropped.
    pub fn notify(&mut self) {
        self.epoch += 1;
    }

    /// Returns a [`LaneStatus::Wait`] for the current epoch.
    pub fn wait(&self, until: Option<Instant>) -> LaneStatus {
        LaneStatus::Wait {
            epoch: self.epoch,
            until,
        }
    }
}

/// State shared between the app and its simulation lanes.
pub(crate) struct Shared<T> {
    state: Mutex<SharedInner<T>>,
    condvar: Condvar,
    /// Incremented when the simulation is reset.
    ///
    /// Lanes check this after locking a world, to make sure that a decision they made before a reset isn't applied after it.
    generation: AtomicU64,
}

/// A lock on [`Shared`] state that wakes up lanes when it is released if the state was changed with [`SharedInner::notify`].
pub(crate) struct SharedGuard<'a, T> {
    /// Only `None` after [`Self::into_inner`].
    guard: Option<MutexGuard<'a, SharedInner<T>>>,
    condvar: &'a Condvar,
    epoch: u64,
}

impl<T> std::ops::Deref for SharedGuard<'_, T> {
    type Target = SharedInner<T>;

    fn deref(&self) -> &SharedInner<T> {
        self.guard.as_ref().unwrap()
    }
}

impl<T> std::ops::DerefMut for SharedGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut SharedInner<T> {
        self.guard.as_mut().unwrap()
    }
}

impl<T> Drop for SharedGuard<'_, T> {
    fn drop(&mut self) {
        if let Some(guard) = &self.guard
            && guard.epoch != self.epoch
        {
            self.condvar.notify_all();
        }
    }
}

impl<'a, T> SharedGuard<'a, T> {
    /// Takes the mutex guard to wait on the condvar. Must not be used after changing the epoch.
    fn into_inner(mut self) -> MutexGuard<'a, SharedInner<T>> {
        let guard = self.guard.take().unwrap();
        debug_assert_eq!(guard.epoch, self.epoch);
        guard
    }
}

impl<T> Shared<T> {
    pub fn new(data: T) -> Self {
        Shared {
            state: Mutex::new(SharedInner {
                epoch: 0,
                shutdown: false,
                data,
            }),
            condvar: Condvar::new(),
            generation: AtomicU64::new(0),
        }
    }

    pub fn lock(&self) -> SharedGuard<'_, T> {
        let guard = self
            .state
            .lock()
            .expect("a simulation thread panicked while holding the simulation state");
        let epoch = guard.epoch;

        SharedGuard {
            guard: Some(guard),
            condvar: &self.condvar,
            epoch,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Must be called while holding the state lock and every world lock.
    pub fn increment_generation(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Waits until the epoch changes, the deadline passes or the runtime shuts down.
    ///
    /// Returns false if the runtime is shutting down.
    fn wait(&self, epoch: u64, until: Option<Instant>) -> bool {
        // Condvar timeouts are only as precise as the OS timer, which is ~15ms on windows.
        // Precise sleeps are used for the end of a deadline, waking every so often to check for signals.
        const COARSE_MARGIN: Duration = Duration::from_millis(20);
        const FINE_SLICE: Duration = Duration::from_millis(1);

        loop {
            let guard = self.lock();

            if guard.shutdown {
                return false;
            }

            if guard.epoch != epoch {
                return true;
            }

            let Some(until) = until else {
                let guard = self
                    .condvar
                    .wait(guard.into_inner())
                    .expect("a simulation thread panicked while holding the simulation state");
                drop(guard);
                continue;
            };

            let now = Instant::now();
            if now >= until {
                return true;
            }

            let remaining = until - now;

            if remaining > COARSE_MARGIN {
                let (guard, _) = self
                    .condvar
                    .wait_timeout(guard.into_inner(), remaining - COARSE_MARGIN)
                    .expect("a simulation thread panicked while holding the simulation state");
                drop(guard);
            } else {
                drop(guard);
                std::thread::sleep(remaining.min(FINE_SLICE));
            }
        }
    }

    fn shutdown(&self) {
        let mut guard = self.lock();
        guard.shutdown = true;
        guard.notify();
    }
}

/// Owns the threads that run lanes, or runs them inline.
pub(crate) struct LaneDriver<T> {
    shared: Arc<Shared<T>>,
    mode: LaneDriverMode,
}

enum LaneDriverMode {
    Threaded(Vec<JoinHandle<()>>),
    Inline(Vec<Box<dyn Lane>>),
}

impl<T> LaneDriver<T>
where
    T: Send + 'static,
{
    pub fn new(
        shared: Arc<Shared<T>>,
        execution: SimulationExecution,
        lanes: Vec<Box<dyn Lane>>,
    ) -> Self {
        let mode = match execution {
            SimulationExecution::Inline => LaneDriverMode::Inline(lanes),
            SimulationExecution::Threaded => LaneDriverMode::Threaded(
                lanes
                    .into_iter()
                    .map(|lane| {
                        let shared = shared.clone();

                        std::thread::Builder::new()
                            .name(format!("nevy_prediction {}", lane.name()))
                            .spawn(move || run_lane(shared, lane))
                            .expect("failed to spawn a simulation thread")
                    })
                    .collect(),
            ),
        };

        LaneDriver { shared, mode }
    }

    /// Runs inline lanes until they have nothing left to do,
    /// and propagates panics from threaded lanes.
    pub fn update(&mut self) {
        match &mut self.mode {
            LaneDriverMode::Threaded(threads) => {
                if let Some(index) = threads.iter().position(JoinHandle::is_finished) {
                    let thread = threads.swap_remove(index);

                    if let Err(payload) = thread.join() {
                        resume_unwind(payload);
                    }

                    panic!("a simulation thread exited unexpectedly");
                }
            }
            LaneDriverMode::Inline(lanes) => {
                // Limit how much work is done in one update in case a lane never waits.
                for _ in 0..10_000 {
                    let mut worked = false;

                    for lane in lanes.iter_mut() {
                        if let LaneStatus::Worked = lane.work() {
                            worked = true;
                        }
                    }

                    if !worked {
                        break;
                    }
                }
            }
        }
    }
}

impl<T> Drop for LaneDriver<T> {
    fn drop(&mut self) {
        self.shared.shutdown();

        if let LaneDriverMode::Threaded(threads) = &mut self.mode {
            for thread in threads.drain(..) {
                // Panics have already been reported by the panic hook.
                let _ = thread.join();
            }
        }
    }
}

fn run_lane<T>(shared: Arc<Shared<T>>, mut lane: Box<dyn Lane>) {
    let name = lane.name();
    let _span = info_span!("simulation lane", name).entered();

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        loop {
            match lane.work() {
                LaneStatus::Worked => {}
                LaneStatus::Wait { epoch, until } => {
                    if !shared.wait(epoch, until) {
                        break;
                    }
                }
            }
        }
    }));

    if let Err(payload) = result {
        // Wake up any other lanes so they notice the poisoned state, then report the panic to the app.
        shared.condvar.notify_all();
        resume_unwind(payload);
    }
}

/// A simulation world that is shared between lanes.
pub(crate) type WorldCell = Mutex<SimulationWorld>;

pub(crate) fn lock_world(cell: &WorldCell) -> std::sync::MutexGuard<'_, SimulationWorld> {
    cell.lock()
        .expect("a simulation thread panicked while holding a simulation world")
}

/// An exponential moving average of how long something takes.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct CostTracker {
    average: Option<f64>,
}

impl CostTracker {
    /// The weight of each new sample.
    const ALPHA: f64 = 0.05;

    pub fn record(&mut self, duration: Duration) {
        let sample = duration.as_secs_f64();

        self.average = Some(match self.average {
            Some(average) => average + (sample - average) * Self::ALPHA,
            None => sample,
        });
    }

    pub fn average(&self) -> Option<Duration> {
        self.average.map(Duration::from_secs_f64)
    }
}

/// Statistics about a simulation lane.
#[derive(Clone, Copy, Default, Debug)]
pub struct LaneStats {
    /// Number of ticks executed since the last reset.
    pub ticks: u64,
    /// Average time to execute one tick.
    pub tick_cost: Option<Duration>,
    /// Total time spent working.
    pub busy: Duration,
}

/// Measures work done on a lane.
#[derive(Default)]
pub(crate) struct LaneMeter {
    pub ticks: u64,
    pub cost: CostTracker,
    pub busy: Duration,
}

impl LaneMeter {
    pub fn record_tick(&mut self, duration: Duration) {
        self.ticks += 1;
        self.cost.record(duration);
        self.busy += duration;
    }

    pub fn record_busy(&mut self, duration: Duration) {
        self.busy += duration;
    }

    pub fn stats(&self) -> LaneStats {
        LaneStats {
            ticks: self.ticks,
            tick_cost: self.cost.average(),
            busy: self.busy,
        }
    }
}
