use std::marker::PhantomData;

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::common::scheme::PredictionScheme;

pub mod prediction_app;

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
        crate::common::build::<S>(app);

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
}
