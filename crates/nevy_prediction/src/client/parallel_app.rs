use std::time::Duration;

use bevy::{
    app::PluginsState,
    ecs::schedule::ScheduleLabel,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};

use crate::common::simulation::{ResetSimulation, SimulationTime, SimulationTimeTarget};

pub(crate) struct ParallelWorld {
    state: ParallelWorldState,
    reset: Option<Duration>,
}

enum ParallelWorldState {
    Idle(World),
    Running(Task<World>),
}

impl ParallelWorld {
    pub fn new(mut app: App) -> Self {
        // make sure the app finished building before updating it
        while let PluginsState::Adding = app.plugins_state() {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();

        let world = std::mem::take(app.world_mut());

        ParallelWorld {
            state: ParallelWorldState::Idle(world),
            reset: Some(Duration::ZERO),
        }
    }

    pub fn update(&mut self, log_time: bool) {
        let ParallelWorldState::Idle(world) = &mut self.state else {
            return;
        };

        let app = std::mem::take(world);

        let task = AsyncComputeTaskPool::get().spawn(run_parallel_world(app, log_time));

        self.state = ParallelWorldState::Running(task);
    }

    pub fn poll(&mut self) -> bool {
        self.state.poll()
    }

    pub fn get(&mut self) -> Option<&mut World> {
        self.state.get().map(|world| {
            if let Some(elapsed) = self.reset.take() {
                self.reset = None;

                let mut time = Time::new_with(SimulationTime);
                time.advance_to(elapsed);
                world.insert_resource(time);
                world.insert_resource(SimulationTimeTarget(elapsed));

                world.run_schedule(ResetSimulation);
            }

            world
        })
    }

    /// Sets a flag so that before the app is returned on the next `Self::get()` the [ResetSimulation] schedule will be run.
    pub fn reset(&mut self, time: Duration) {
        self.reset = Some(time);
    }
}

impl ParallelWorldState {
    fn poll(&mut self) -> bool {
        let ParallelWorldState::Running(task) = self else {
            return true;
        };

        let Some(result) = block_on(poll_once(task)) else {
            return false;
        };

        *self = ParallelWorldState::Idle(result);

        true
    }

    fn get(&mut self) -> Option<&mut World> {
        let ParallelWorldState::Idle(world) = self else {
            return None;
        };

        Some(world)
    }
}

async fn run_parallel_world(mut world: World, _log_time: bool) -> World {
    // let start = std::time::Instant::now();
    // let simulation_start = world.resource::<Time<SimulationTime>>().elapsed();

    world.run_schedule(Main);

    // let simulated_time = world.resource::<Time<SimulationTime>>().elapsed() - simulation_start;
    // if _log_time {
    //     debug!(
    //         "Parallel world advanced simulation {:?} in {:?}",
    //         simulated_time,
    //         start.elapsed()
    //     );
    // }

    world
}

/// Schedule that extracts the simulation state from a [SourceWorld] into the current world.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractSimulation;

/// Holds the source world during the [ExtractSimulation] schedule.
#[derive(Resource, Deref, DerefMut)]
pub struct SourceWorld(pub World);
