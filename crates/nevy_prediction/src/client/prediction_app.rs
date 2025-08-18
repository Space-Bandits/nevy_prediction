use std::time::Duration;

use bevy::{ecs::schedule::ScheduleLabel, prelude::*, tasks::Task};

use crate::common::simulation::SimulationPlugin;

pub struct PredictionApp {
    state: PredictionAppState,
}

pub enum PredictionAppState {
    Idle(App),
    Running(Task<Result<App>>),
}

impl PredictionApp {
    pub fn new(step_interval: Duration) -> Self {
        let mut app = App::empty();

        app.add_schedule(Schedule::new(Main));
        app.main_mut().update_schedule = Some(Main.intern());

        app.add_plugins(SimulationPlugin {
            schedule: Main.intern(),
            step_interval,
        });

        PredictionApp {
            state: PredictionAppState::Idle(app),
        }
    }
}
