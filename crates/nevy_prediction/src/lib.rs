//! Client side prediction for [`nevy`] with a fixed timestep simulation.
//!
//! # Architecture
//!
//! The simulation never runs on the app's main thread.
//! Every copy of the simulation that executes ticks has its own [`World`](bevy::prelude::World),
//! built from the [`PredictionScheme`](common::scheme::PredictionScheme)'s plugin, and is stepped on its own thread.
//! See [`SimulationInstance`](common::simulation::SimulationInstance) for every kind of world.
//!
//! - The **server** steps the authoritative simulation as its clock advances.
//! - The **client** steps three worlds in parallel:
//!   - the *template* replays the server's world updates up to the latest tick the server finished,
//!   - the *prediction* forks from the template whenever it advances and re-simulates up to the main world,
//!     including the client's own updates that the server hasn't confirmed yet,
//!   - the *main* world runs [`PredictionInterval`](client::interval::PredictionInterval) ahead of the server,
//!     and has every finished prediction extracted into it.
//!
//! After every tick, the server and client main worlds are extracted into a snapshot,
//! which the app extracts into its own world in [`SyncSimulationSystems`](common::simulation::SyncSimulationSystems).
//! The app's world is never stepped, it only presents and reads the latest state.
//! The app never waits for a simulation thread.
//!
//! Simulation threads still run schedules with the multi-threaded executor,
//! so the systems in them run on the [`ComputeTaskPool`](bevy::tasks::ComputeTaskPool) alongside the app's systems.
//! [`SimulationExecution::Inline`](common::runtime::SimulationExecution::Inline) runs them on the main thread instead.
//!
//! The client's template and prediction worlds are given a budget for every main world tick, see [`PredictionRates`](client::PredictionRates).
//!
//! # Time
//!
//! If the server or any client can't execute ticks fast enough, the server slows the simulation down for everyone,
//! see [`time_scale`](server::time_scale).
//! [`Time<PredictionTime>`](common::time::PredictionTime) is the clock that the app should use for anything that follows the simulation,
//! such as interpolating between ticks.
//!
//! # Changing the simulation
//!
//! - Clients create world updates with [`PredictionUpdateCreator`](client::PredictionUpdateCreator).
//! - The server queues world updates with [`ServerUpdateQueue`](server::ServerUpdateQueue)
//!   and sends them to clients with [`WorldUpdateSender`](server::WorldUpdateSender).
//! - [`ClientSimulation`](client::pipeline::ClientSimulation) and [`ServerSimulation`](server::pipeline::ServerSimulation)
//!   can run arbitrary functions on simulation worlds, for configuration or changes outside of the world update flow.
//!
//! # Writing plugins for the simulation
//!
//! The scheme's plugin is built into every world, including the app's world and the snapshot buffers.
//! Use [`SimulationInstance::steps`](common::simulation::SimulationInstance::steps) and
//! [`SimulationInstance::is_app`](common::simulation::SimulationInstance::is_app) to decide what to add where.
//! For example a physics plugin would step physics only in stepping worlds,
//! and in app worlds update its spatial query structures after extraction and interpolate with `Time<PredictionTime>`.
//! Everything in the simulation must be [`Send`], because simulation worlds move between threads.

pub mod client;
pub mod common;
pub mod server;

pub mod prelude {
    pub use crate::client::{
        ClientSimulationSystems, NevyPredictionClientPlugin, PredictionRates,
        PredictionServerConnection, PredictionUpdateCreator,
        interval::{
            PredictionDiagnosticsPlugin, PredictionInterval, PredictionIntervalSettings,
            PredictionIntervalStats,
        },
        pipeline::{ClientSimulation, ClientSimulationStats},
    };

    pub use crate::common::{
        PredictionMessages, ServerWorldUpdate,
        runtime::{LaneStats, SimulationExecution},
        scheme::{AddWorldUpdate, PredictionScheme},
        simulation::{
            ExtractSimulationSystems, ReadyUpdates, SimulationInstance, SimulationTick,
            SimulationTime, SimulationTimeExt, SourceWorld, SyncSimulationSystems,
            UpdateExecutionQueue, WorldUpdate,
            extract_component::{ExtractComponentSystems, ExtractSimulationComponentPlugin},
            extract_relation::{ExtractRelationSystems, ExtractSimulationRelationPlugin},
            extract_resource::ExtractSimulationResourcePlugin,
            schedules::{
                ExtractSimulation, ResetSimulation, SimulationPostUpdate, SimulationPreUpdate,
                SimulationStartup, SimulationUpdate,
            },
            simulation_entity::{
                DespawnSimulationEntities, DespawnSimulatonEntity, ExtractDespawnPriority,
                SimulationEntity, SimulationEntityMap,
            },
            update_component::{UpdateComponent, UpdateComponentPlugin, UpdateComponentSystems},
        },
        time::{PredictionTime, PredictionTimeExt},
    };

    pub use crate::server::{
        NevyPredictionServerPlugin, PredictionClient, ServerSimulationSystems, ServerUpdateQueue,
        WorldUpdateSender,
        pipeline::{ServerSimulation, ServerSimulationStats},
        time_scale::{
            ClientSimulationCapacity, ServerTimeScale, TimeScaleLimit, TimeScaleSettings,
        },
    };
}
