use bevy::prelude::*;
use nevy::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::common::{
    scheme::PredictionScheme,
    simulation::{
        SimulationTick, WorldUpdate,
        schedules::{ResetSimulation, SimulationStartup},
    },
};

pub mod scheme;
pub mod simulation;

pub struct PredictionMessages;

/// Build function run for the client and server app
pub(crate) fn build<S>(app: &mut App)
where
    S: PredictionScheme,
{
    app.init_protocol::<PredictionMessages>();

    app.add_protocol_message::<PredictionMessages, ResetClientSimulation>();
    app.add_protocol_message::<PredictionMessages, UpdateServerTick>();

    app.add_systems(Startup, startup_simulation);
}

/// Build function run for the client and server app per world update
pub(crate) fn build_update<T>(app: &mut App)
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    app.add_protocol_message::<PredictionMessages, ServerWorldUpdate<T>>();
}

/// run on the client and server during the [`Startup`] schedule.
fn startup_simulation(world: &mut World) {
    world.run_schedule(SimulationStartup);
    world.run_schedule(ResetSimulation);
}

/// Server -> Client message to reset the simulation.
#[derive(Serialize, Deserialize)]
pub(crate) struct ResetClientSimulation {
    pub simulation_tick: SimulationTick,
}

/// Server -> Client message to update the current simulation time on the server.
///
/// This will cause the client to advance its current copy of the server's simulation,
/// applying any [`ServerWorldUpdate`]s it received before this message.
#[derive(Serialize, Deserialize)]
pub(crate) struct UpdateServerTick {
    pub simulation_tick: SimulationTick,
}

/// Server -> Client message to apply a [`WorldUpdate`].
///
/// This type is in the public api only so that it's message id can be retrieved.
#[derive(Serialize, Deserialize)]
pub struct ServerWorldUpdate<T> {
    pub(crate) update: WorldUpdate<T>,
    pub(crate) include_in_prediction: bool,
}
