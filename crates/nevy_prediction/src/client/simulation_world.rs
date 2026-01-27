//! Contains logic for running schedules on a world asynchronously to the main app.

use bevy::{app::PluginsState, prelude::*};

use crate::common::{
    scheme::PredictionScheme,
    simulation::{
        PrivateSimulationTimeExt, SimulationTick, SimulationTime, SourceWorld,
        schedules::{ExtractSimulation, ResetSimulation},
    },
};

/// A separate world for containing prediction logic
#[derive(Deref, DerefMut)]
pub struct SimulationWorld(World);

impl SimulationWorld {
    pub fn build(mut app: App) -> Self {
        // make sure the app finished building before updating it
        while let PluginsState::Adding = app.plugins_state() {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();

        let world = std::mem::take(app.world_mut());

        SimulationWorld(world)
    }

    /// Queues some number of ticks and runs the [`Main`] schedule.
    pub fn run(&mut self, execute_ticks: u32) {
        self.resource_mut::<Time<SimulationTime>>()
            .queue_ticks(execute_ticks);

        self.run_schedule(Main);
    }

    /// Updates the [`SimulationTime`] runs the [`ResetSimulation`] schedule.
    pub fn reset<S>(&mut self, tick: SimulationTick)
    where
        S: PredictionScheme,
    {
        self.insert_resource(Time::<SimulationTime>::from_tick::<S>(tick));

        self.run_schedule(ResetSimulation);
    }

    /// Extracts this [`SimulationWorld`] into another [`World`]
    pub fn extract(&mut self, target_world: &mut World) {
        let owned_world = std::mem::take(&mut self.0);

        // Extract the simulation time from the source world.
        owned_world
            .resource::<Time<SimulationTime>>()
            .extract_time(target_world.resource_mut::<Time<SimulationTime>>().as_mut());

        // Insert source world and run extract schedule.
        target_world.insert_resource(SourceWorld(owned_world));
        target_world.run_schedule(ExtractSimulation);
        let SourceWorld(owned_world) = target_world
            .remove_resource()
            .expect("Extract schedule removed the `SourceWorld`");

        // Swap world back and replace scratch world.
        self.0 = owned_world;
    }
}
