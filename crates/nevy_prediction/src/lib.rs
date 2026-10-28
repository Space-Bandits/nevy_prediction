//! Client side prediction for [`nevy`] with a fixed timestep simulation.
//!
//! # Architecture
//!
//! Every copy of the simulation is a [`World`](bevy::prelude::World) with the
//! [`PredictionScheme`](common::scheme::PredictionScheme)'s plugin built into it.
//! See [`SimulationInstance`](common::simulation::SimulationInstance) for every kind of world.
//!
//! - The **server** app's world is the authoritative simulation.
//!   It is stepped in [`SyncSimulationSystems`](common::simulation::SyncSimulationSystems)
//!   once for every tick its clock has passed, so server game logic reads and changes the simulation directly.
//! - The **client** steps three worlds in parallel, each on its own thread:
//!   - the *template* replays the server's world updates up to the latest tick the server finished,
//!   - the *prediction* forks from the template whenever it advances and re-simulates up to the main world,
//!     including the client's own updates that the server hasn't confirmed yet,
//!   - the *main* world runs [`PredictionInterval`](client::interval::PredictionInterval) ahead of the server,
//!     and has every finished prediction extracted into it.
//!
//! After every tick, the client's main world is extracted into a snapshot,
//! which the client app extracts into its own world in [`SyncSimulationSystems`](common::simulation::SyncSimulationSystems).
//! The client app's world is never stepped, it only presents and reads the latest state.
//! The client app never waits for a simulation thread.
//!
//! Client simulation threads still run schedules with the multi-threaded executor,
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
//! - [`ClientSimulation`](client::pipeline::ClientSimulation) can run arbitrary functions on the client's simulation worlds,
//!   for configuration or changes outside of the world update flow.
//!   The server's simulation is the server app's world, so it can be changed directly.
//!
//! # Writing plugins for the simulation
//!
//! The scheme's plugin is built into every world, including the app worlds and the snapshot buffers.
//! Use [`SimulationInstance::steps`](common::simulation::SimulationInstance::steps) and
//! [`SimulationInstance::is_app`](common::simulation::SimulationInstance::is_app) to decide what to add where.
//! For example a physics plugin would step physics only in stepping worlds,
//! and in app worlds update its spatial query structures after extraction and interpolate with `Time<PredictionTime>`.
//! Everything in the simulation must be [`Send`], because the client's simulation worlds move between threads.

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
        stepping::ServerSimulationStats,
        time_scale::{
            ClientSimulationCapacity, ServerTimeScale, TimeScaleLimit, TimeScaleSettings,
        },
    };
}
