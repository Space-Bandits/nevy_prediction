use std::time::Duration;

use bevy::{
    app::PluginsState,
    ecs::schedule::ScheduleLabel,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};

use crate::common::simulation::{ResetSimulation, SimulationTime, SimulationTimeTarget};

pub(crate) struct ParallelApp {
    state: ParallelAppState,
    reset: Option<Duration>,
}

enum ParallelAppState {
    Idle(App),
    Running(Task<Result<App>>),
}

impl ParallelApp {
    pub fn new(mut app: App) -> Self {
        // make sure the app finished building before updating it
        while let PluginsState::Adding = app.plugins_state() {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();

        ParallelApp {
            state: ParallelAppState::Idle(app),
            reset: Some(Duration::ZERO),
        }
    }

    pub fn update(&mut self) {
        let ParallelAppState::Idle(app) = &mut self.state else {
            return;
        };

        let app = std::mem::replace(app, App::empty());

        let task = AsyncComputeTaskPool::get().spawn_local(run_parallel_app(app));

        self.state = ParallelAppState::Running(task);
    }

    pub fn poll(&mut self) -> Result<Option<&mut App>> {
        Ok(self.state.poll()?.map(|app| {
            if let Some(elapsed) = self.reset.take() {
                self.reset = None;

                let mut time = Time::new_with(SimulationTime);
                time.advance_to(elapsed);
                app.insert_resource(time);
                app.insert_resource(SimulationTimeTarget(elapsed));

                app.world_mut().run_schedule(ResetSimulation);
            }

            app
        }))
    }

    /// Sets a flag so that before the app is returned on the next `Self::poll()` the [ResetSimulation] schedule will be run.
    pub fn reset(&mut self, time: Duration) {
        self.reset = Some(time);
    }
}

impl ParallelAppState {
    pub fn poll(&mut self) -> Result<Option<&mut App>> {
        let task = match self {
            ParallelAppState::Idle(app) => return Ok(Some(app)),
            ParallelAppState::Running(task) => task,
        };

        let Some(result) = block_on(poll_once(task)) else {
            return Ok(None);
        };

        *self = ParallelAppState::Idle(result?);

        let ParallelAppState::Idle(app) = self else {
            unreachable!();
        };

        Ok(Some(app))
    }
}

async fn run_parallel_app(mut app: App) -> Result<App> {
    app.update();

    if let Some(app_exit) = app.should_exit() {
        return Err(format!("Parallel app exited, which souldn't happen: {:?}", app_exit).into());
    }

    Ok(app)
}

/// Schedule that extracts the simulation state from a [SourceWorld] into the current world.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractSimulation;

/// Holds the source world during the [ExtractSimulation] schedule.
#[derive(Resource, Deref, DerefMut)]
pub struct SourceWorld(pub World);
