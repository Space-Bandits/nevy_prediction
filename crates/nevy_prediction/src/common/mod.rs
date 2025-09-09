use std::time::Duration;

use bevy::prelude::*;
use nevy::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::common::{
    scheme::PredictionScheme,
    simulation::{ResetSimulation, SimulationStartup, WorldUpdate},
};

pub mod scheme;
pub mod simulation;

pub mod prelude {
    pub use crate::common::{
        scheme::{PredictionScheme, SchemeWorldUpdates},
        simulation::{
            ExtractSimulation, ReadyUpdates, SimulationInstance, SimulationStartup, SimulationTime,
            SimulationUpdate, SourceWorld, WorldUpdate, WorldUpdateQueue,
            extract_component::ExtractSimulationComponentPlugin,
            extract_resource::ExtractSimulationResourcePlugin,
            simulation_entity::{
                ExtractSimulationEntitySystems, SimulationEntity, SimulationEntityMap,
            },
        },
    };
}

/// Build function run for the client and server app
pub(crate) fn build<S>(app: &mut App)
where
    S: PredictionScheme,
{
    app.add_message::<ResetClientSimulation>();
    app.add_message::<UpdateServerTime>();

    app.add_systems(Startup, startup_simulation);

    for update in S::updates().0 {
        update.build_common(app);
    }
}

/// Build function run for the client and server app per world update
pub(crate) fn build_update<T>(app: &mut App)
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    app.add_message::<ServerWorldUpdate<T>>();
}

/// run on the client and server during the [`Startup`] schedule.
fn startup_simulation(world: &mut World) {
    world.run_schedule(SimulationStartup);
    world.run_schedule(ResetSimulation);
}

/// Server -> Client message to reset the simulation.
#[derive(Serialize, Deserialize)]
pub(crate) struct ResetClientSimulation {
    pub simulation_time: Duration,
}

/// Server -> Client message to update the current simulation time on the server.
///
/// This will cause the client to advance its current copy of the server's simulation,
/// applying any [`ServerWorldUpdate`]s it received before this message.
#[derive(Serialize, Deserialize)]
pub(crate) struct UpdateServerTime {
    pub simulation_time: Duration,
}

/// Server -> Client message to apply an update to the simulation world at a certain time.
#[derive(Serialize, Deserialize)]
pub(crate) struct ServerWorldUpdate<T> {
    pub update: WorldUpdate<T>,
}
