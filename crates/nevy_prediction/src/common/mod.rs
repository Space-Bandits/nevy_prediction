use std::time::Duration;

use bevy::prelude::*;
use nevy::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::common::simulation::{SimulationTick, WorldUpdate, schedules::SimulationStartupMain};

pub(crate) mod clock;
pub mod runtime;
pub mod scheme;
pub mod simulation;
pub mod time;

pub struct PredictionMessages;

/// Build function run for the client and server app
pub(crate) fn build(app: &mut App) {
    app.init_protocol::<PredictionMessages>();

    app.add_protocol_message::<PredictionMessages, ResetClientSimulation>();
    app.add_protocol_message::<PredictionMessages, UpdateServerTick>();
    app.add_protocol_message::<PredictionMessages, TickProbe>();
    app.add_protocol_message::<PredictionMessages, TickProbeResult>();
    app.add_protocol_message::<PredictionMessages, SimulationCapacity>();

    app.add_systems(PreStartup, startup_simulation);
}

/// Build function run for the client and server app per world update
pub(crate) fn build_update<T>(app: &mut App)
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    app.add_protocol_message::<PredictionMessages, ServerWorldUpdate<T>>();
}

/// Runs the startup schedules for the app's instance of the simulation.
fn startup_simulation(world: &mut World) {
    world.run_schedule(SimulationStartupMain);
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
/// All world updates with a tick before `simulation_tick` are sent before this message.
#[derive(Serialize, Deserialize)]
pub(crate) struct UpdateServerTick {
    pub simulation_tick: SimulationTick,
    /// The server's simulation clock when this message was sent.
    pub time: Duration,
    /// How many simulation seconds pass per real second on the server.
    pub time_scale: f32,
}

/// Client -> Server message used to measure how early the client's world updates arrive.
///
/// Sent at the same point in the frame as world updates are created, stamped with the same tick.
#[derive(Serialize, Deserialize)]
pub(crate) struct TickProbe {
    pub simulation_tick: SimulationTick,
}

/// Server -> Client response to a [`TickProbe`].
#[derive(Serialize, Deserialize)]
pub(crate) struct TickProbeResult {
    pub simulation_tick: SimulationTick,
    /// Seconds before the server would have executed the probe's tick that it was received.
    /// Negative if it arrived too late.
    pub lead: f32,
}

/// Client -> Server message reporting the fastest time scale that the client can sustain.
///
/// The server slows down the simulation if any client can't keep up.
#[derive(Serialize, Deserialize)]
pub(crate) struct SimulationCapacity {
    pub time_scale: f32,
}

/// Server -> Client message to apply a [`WorldUpdate`].
///
/// This type is in the public api only so that it's message id can be retrieved.
#[derive(Serialize, Deserialize)]
pub struct ServerWorldUpdate<T> {
    pub(crate) update: WorldUpdate<T>,
    pub(crate) include_in_prediction: bool,
}
