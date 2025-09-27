pub mod client;
pub mod common;
pub mod server;

pub mod prelude {
    pub use crate::client::{
        ClientSimulationSystems, NevyPredictionClientPlugin, PredictionInterval, PredictionRates,
        PredictionServerConnection, PredictionUpdateCreator,
    };

    pub use crate::common::{
        ServerWorldUpdate,
        scheme::{AddWorldUpdate, PredictionScheme},
        simulation::{
            ExtractSimulationSystems, ReadyUpdates, SimulationInstance, SimulationTick,
            SimulationTime, SimulationTimeExt, SourceWorld, StepSimulationSystems,
            UpdateExecutionQueue, WorldUpdate,
            extract_component::ExtractSimulationComponentPlugin,
            extract_relation::ExtractSimulationRelationPlugin,
            extract_resource::ExtractSimulationResourcePlugin,
            schedules::{
                ExtractSimulation, SimulationPostUpdate, SimulationPreUpdate, SimulationStartup,
                SimulationUpdate,
            },
            simulation_entity::{
                DespawnSimulationEntities, DespawnSimulatonEntity, SimulationEntity,
                SimulationEntityMap,
            },
            update_component::{UpdateComponent, UpdateComponentPlugin, UpdateComponentSystems},
        },
    };

    pub use crate::server::{
        NevyPredictionServerPlugin, PredictionClient, ServerSimulationSystems, WorldUpdateSender,
    };
}
