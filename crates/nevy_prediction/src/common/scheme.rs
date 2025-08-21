use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use serde::{Serialize, de::DeserializeOwned};

/// Implement this trait on a type to define a prediction scheme.
pub trait PredictionScheme: Send + Sync + 'static {
    fn updates() -> SchemeWorldUpdates;

    /// Which nevy stream header is used for messaging.
    fn message_header() -> impl Into<u16>;

    /// The plugin that should be added to any app that runs the simulation.
    fn plugin() -> impl Plugin;

    fn step_interval() -> Duration {
        Duration::from_millis(50)
    }
}

/// Defines a list of updates that can modify the predicted world.
///
/// World updates provide a way of recording changes to the predicted world.
/// This is used for prediction, reconciliation and initializing and updating client side worlds.
pub struct SchemeWorldUpdates(pub(crate) Vec<Box<dyn SchemeUpdate>>);

pub(crate) trait SchemeUpdate {
    fn build_common(&self, app: &mut App);

    fn build_client(&self, app: &mut App, schedule: Interned<dyn ScheduleLabel>);

    fn build_server(&self, app: &mut App, schedule: Interned<dyn ScheduleLabel>);

    fn build_simulation(&self, app: &mut App);
}

impl<T> SchemeUpdate for PhantomData<T>
where
    T: Send + Sync + 'static + Serialize + DeserializeOwned + Clone,
{
    fn build_common(&self, app: &mut App) {
        crate::common::build_update::<T>(app);
    }

    fn build_client(&self, app: &mut App, schedule: Interned<dyn ScheduleLabel>) {
        crate::client::build_update::<T>(app, schedule);
    }

    fn build_server(&self, app: &mut App, schedule: Interned<dyn ScheduleLabel>) {
        crate::server::build_update::<T>(app, schedule);
    }

    fn build_simulation(&self, app: &mut App) {
        crate::common::simulation::build_update::<T>(app);
    }
}

impl Default for SchemeWorldUpdates {
    fn default() -> Self {
        SchemeWorldUpdates(Vec::new())
    }
}

impl SchemeWorldUpdates {
    /// Adds an update message to this scheme.
    ///
    /// This will "assign" this message type to this scheme,
    /// meaning logic will break if it is used in another scheme in the same app.
    pub fn add_message<T: 'static>(&mut self) -> &mut Self
    where
        T: Send + Sync + 'static + Serialize + DeserializeOwned + Clone,
    {
        self.0.push(Box::new(PhantomData::<T>));
        self
    }

    /// Adds an update message to this scheme.
    ///
    /// This will "assign" this message type to this scheme,
    /// meaning logic will break if it is used in another scheme in the same app.
    pub fn with_message<T: 'static>(mut self) -> Self
    where
        T: Send + Sync + 'static + Serialize + DeserializeOwned + Clone,
    {
        self.add_message::<T>();
        self
    }
}
