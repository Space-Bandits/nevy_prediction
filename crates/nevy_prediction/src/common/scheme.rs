use std::time::Duration;

use bevy::prelude::*;
use serde::{Serialize, de::DeserializeOwned};

use crate::common::simulation::SimulationInstance;

/// This trait defines a prediction scheme that controls how the client and server interact.
///
/// Implement this trait on a marker type to define a prediction scheme.
pub trait PredictionScheme: Send + Sync + 'static {
    /// Which nevy stream header is used for messaging.
    fn message_header() -> impl Into<u16>;

    /// The plugin that should be added to any app that runs the simulation.
    fn plugin() -> impl Plugin;

    fn step_interval() -> Duration {
        Duration::from_millis(50)
    }
}

pub trait AddWorldUpdate {
    /// Adds a simulation world update to the app.
    ///
    /// This should be called by the plugin provided by [`PredictionScheme`].
    /// The order that updates are added in should be the same for all instances of the plugin.
    /// See [`nevy::messages::AddMessage::add_message`].
    fn add_world_update<T>(&mut self) -> &mut Self
    where
        T: Send + Sync + 'static + Serialize + DeserializeOwned + Clone;
}

impl AddWorldUpdate for App {
    fn add_world_update<T>(&mut self) -> &mut Self
    where
        T: Send + Sync + 'static + Serialize + DeserializeOwned + Clone,
    {
        let instance = self.world().resource::<SimulationInstance>();

        match instance {
            SimulationInstance::Server => {
                crate::common::build_update::<T>(self);
                crate::common::simulation::build_update::<T>(self);
            }
            SimulationInstance::ClientMain => {
                crate::client::build_update::<T>(self);
                crate::common::build_update::<T>(self);
                crate::common::simulation::build_update::<T>(self);
            }
            SimulationInstance::ClientTemplate => {
                crate::common::simulation::build_update::<T>(self);
            }
            SimulationInstance::ClientPrediction => {
                crate::common::simulation::build_update::<T>(self);
            }
        }

        self
    }
}
