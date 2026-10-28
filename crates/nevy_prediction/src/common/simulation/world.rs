//! Contains [`SimulationWorld`], a separate [`World`] that the simulation is run in.

use bevy::{app::PluginsState, prelude::*};
use tracing::info_span;

use crate::common::{
    scheme::PredictionScheme,
    simulation::{
        PrivateSimulationTimeExt, SimulationInstance, SimulationPlugin, SimulationTick,
        SimulationTime, SimulationTimeExt, SourceWorld,
        schedules::{ExtractSimulation, ResetSimulation, SimulationMain, SimulationStartupMain},
    },
};

/// Counts how many times a snapshot world has been extracted into, so the app knows when there is a new state.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Deref, DerefMut)]
pub(crate) struct SnapshotSequence(pub u64);

pub(crate) fn build_simulation_world<S>(instance: SimulationInstance) -> SimulationWorld
where
    S: PredictionScheme,
{
    let mut app = App::empty();
    app.add_plugins(SimulationPlugin::<S>::new(instance));

    if let SimulationInstance::Snapshot = instance {
        app.init_resource::<SnapshotSequence>();
    }

    SimulationWorld::build(app)
}

/// A world built from an [`App`] that contains an instance of the simulation.
///
/// These worlds are owned by simulation threads, or are buffers between simulation threads and the app.
pub(crate) struct SimulationWorld {
    world: World,
    /// Swapped in for the world while it is the [`SourceWorld`] of an extraction.
    scratch: Option<World>,
}

impl SimulationWorld {
    /// Finishes building an app and takes its world, then runs the simulation startup schedules.
    pub fn build(mut app: App) -> Self {
        // make sure the app finished building before updating it
        while let PluginsState::Adding = app.plugins_state() {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();

        let mut world = std::mem::take(app.world_mut());

        world.run_schedule(SimulationStartupMain);

        SimulationWorld {
            world,
            scratch: None,
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// The next tick that this world will execute.
    pub fn tick(&self) -> SimulationTick {
        self.world.resource::<Time<SimulationTime>>().current_tick()
    }

    /// Executes a single tick of the simulation.
    pub fn step<S>(&mut self)
    where
        S: PredictionScheme,
    {
        step_world::<S>(&mut self.world);
    }

    /// Sets the [`SimulationTime`] to a tick and runs the [`ResetSimulation`] schedule.
    pub fn reset<S>(&mut self, tick: SimulationTick)
    where
        S: PredictionScheme,
    {
        reset_world::<S>(&mut self.world, tick);
    }

    /// Extracts another simulation world into this one.
    pub fn extract_from(&mut self, source: &mut SimulationWorld) {
        source.extract_into(&mut self.world);
    }

    /// Extracts this simulation world into another world, such as the app's world.
    pub fn extract_into(&mut self, target: &mut World) {
        let placeholder = self.scratch.take().unwrap_or_default();
        let source_world = std::mem::replace(&mut self.world, placeholder);

        let source_world = extract_world(source_world, target);

        self.scratch = Some(std::mem::replace(&mut self.world, source_world));
    }
}

/// Executes a single tick of the simulation in a world.
pub(crate) fn step_world<S>(world: &mut World)
where
    S: PredictionScheme,
{
    let mut time = world.resource_mut::<Time<SimulationTime>>();
    time.clear_target();
    time.queue_ticks(1);

    // Save the current generic time to replace it after overwriting it with `SimulationTime`.
    let old_time = *world.resource::<Time>();
    *world.resource_mut::<Time>() = world.resource::<Time<SimulationTime>>().as_generic();

    world.run_schedule(SimulationMain);

    // `SimulationTime` contains the timestamp of the *next* update, so we advance it after executing `SimulationUpdate`.
    world.resource_mut::<Time<SimulationTime>>().step::<S>();

    *world.resource_mut::<Time>() = old_time;
}

/// Sets the [`SimulationTime`] of a world to a tick and runs the [`ResetSimulation`] schedule.
pub(crate) fn reset_world<S>(world: &mut World, tick: SimulationTick)
where
    S: PredictionScheme,
{
    world.insert_resource(Time::<SimulationTime>::from_tick::<S>(tick));

    info_span!(
        "ResetSimulation",
        simulation_instance = world.resource::<SimulationInstance>().format_tracing_str()
    )
    .in_scope(|| {
        world.run_schedule(ResetSimulation);
    });
}

/// Inserts the source world into the target as a [`SourceWorld`], runs [`ExtractSimulation`] and returns the source world.
fn extract_world(source_world: World, target_world: &mut World) -> World {
    // Extract the simulation time from the source world.
    source_world
        .resource::<Time<SimulationTime>>()
        .extract_time(target_world.resource_mut::<Time<SimulationTime>>().as_mut());

    target_world.insert_resource(SourceWorld(source_world));

    info_span!(
        "ExtractSimulation",
        simulation_instance = target_world
            .resource::<SimulationInstance>()
            .format_tracing_str()
    )
    .in_scope(|| {
        target_world.run_schedule(ExtractSimulation);
    });

    let SourceWorld(source_world) = target_world
        .remove_resource()
        .expect("Extract schedule removed the `SourceWorld`");

    source_world
}
