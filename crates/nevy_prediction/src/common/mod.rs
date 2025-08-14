use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Implement this trait on a type to define a prediction scheme.
///
/// This type will be used as a marker component for generic components, resources and systems specific to this scheme.
pub trait PredictionScheme: Send + Sync + 'static {
    fn updates() -> SchemeWorldUpdates<Self>;

    fn message_header() -> impl Into<u16>;
}

pub fn build<S>(app: &mut App)
where
    S: PredictionScheme,
{
    app.add_message::<WorldUpdateSync<S>>();

    for update in S::updates().0 {
        update.build_common(app);
    }
}

/// Defines a list of updates used by a [PredictionScheme]
pub struct SchemeWorldUpdates<S>(pub(crate) Vec<Box<dyn SchemeUpdate<S>>>)
where
    S: ?Sized;

pub(crate) trait SchemeUpdate<S> {
    fn build_common(&self, app: &mut App);

    fn build_client(&self, app: &mut App, schedule: Interned<dyn ScheduleLabel>);
}

impl<S, T> SchemeUpdate<S> for PhantomData<T>
where
    T: Send + Sync + 'static,
    S: Send + Sync + 'static,
    WorldUpdate<S, T>: Serialize + DeserializeOwned,
{
    fn build_common(&self, app: &mut App) {
        app.add_message::<WorldUpdate<S, T>>();
    }

    fn build_client(&self, app: &mut App, schedule: Interned<dyn ScheduleLabel>) {
        crate::client::build_message::<S, T>(app, schedule);
    }
}

impl<S> Default for SchemeWorldUpdates<S> {
    fn default() -> Self {
        SchemeWorldUpdates(Vec::new())
    }
}

impl<S> SchemeWorldUpdates<S> {
    /// Adds an update message to this scheme.
    ///
    /// This will "assign" this message type to this scheme,
    /// meaning logic will break if it is used in another scheme in the same app.
    pub fn add_message<T: 'static>(&mut self) -> &mut Self
    where
        T: Send + Sync + 'static,
        S: Send + Sync + 'static,
        WorldUpdate<S, T>: Serialize + DeserializeOwned,
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
        T: Send + Sync + 'static,
        S: Send + Sync + 'static,
        WorldUpdate<S, T>: Serialize + DeserializeOwned,
    {
        self.add_message();
        self
    }
}

/// Sent after a batch of [WorldStateUpdate]s
#[derive(Serialize, Deserialize)]
pub(crate) struct WorldUpdateSync<S> {
    pub _p: PhantomData<S>,
    pub update_time: Duration,
}

/// This is a nevy message that contains a world update for a prediction scheme.
///
/// `WorldUpdate`s are added to the app by specifying them in [PredictionScheme::updates]
/// and can then be used to
#[derive(Serialize, Deserialize)]
pub struct WorldUpdate<S, T> {
    pub(crate) _p: PhantomData<S>,
    pub(crate) update: T,
}

impl<S, T> WorldUpdate<S, T> {
    pub fn new(update: T) -> Self {
        WorldUpdate {
            _p: PhantomData,
            update,
        }
    }
}
