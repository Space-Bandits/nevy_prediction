use std::marker::PhantomData;

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};

use crate::common::{
    scheme::PredictionScheme,
    simulation::{SimulationInstance, SimulationPlugin, SimulationTimeTarget},
};

pub mod parallel_app;
pub mod prediction_app;
pub mod server_world_app;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientPredictionSet {
    QueueUpdates,
    RunPredictionWorld,
    RunServerWorld,
}

pub struct NevyPredictionClientPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
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
        app.configure_sets(
            self.schedule,
            (
                ClientPredictionSet::QueueUpdates,
                ClientPredictionSet::RunPredictionWorld,
                ClientPredictionSet::RunServerWorld,
            )
                .chain(),
        );

        crate::common::build::<S>(app);
        server_world_app::build::<S>(app, self.schedule);
        prediction_app::build::<S>(app, self.schedule);

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: Update.intern(),
            instance: SimulationInstance::ClientMain,
        });

        app.add_systems(Update, drive_simulation_time);

        for update in S::updates().0 {
            update.build_client(app, self.schedule);
        }
    }
}

/// Is called on the client app for each world update message added by the prediction scheme
pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static,
{
    server_world_app::build_update::<T>(app, schedule);
}

fn drive_simulation_time(mut target_time: ResMut<SimulationTimeTarget>, time: Res<Time>) {
    **target_time += time.delta();
}
