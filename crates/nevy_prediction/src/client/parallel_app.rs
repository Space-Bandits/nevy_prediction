use bevy::{
    app::PluginsState,
    ecs::schedule::ScheduleLabel,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};

pub(crate) enum ParallelApp {
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

        ParallelApp::Idle(app)
    }

    pub fn run(&mut self) {
        let ParallelApp::Idle(app) = self else {
            return;
        };

        let app = std::mem::replace(app, App::empty());

        let task = AsyncComputeTaskPool::get().spawn_local(run_parallel_app(app));

        *self = ParallelApp::Running(task);
    }

    pub fn poll(&mut self) -> Result<Option<&mut App>> {
        let task = match self {
            ParallelApp::Idle(app) => return Ok(Some(app)),
            ParallelApp::Running(task) => task,
        };

        let Some(result) = block_on(poll_once(task)) else {
            return Ok(None);
        };

        *self = ParallelApp::Idle(result?);

        let ParallelApp::Idle(app) = self else {
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
