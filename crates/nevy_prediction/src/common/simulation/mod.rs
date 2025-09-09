//! This module contains logic that controls the execution of a simulation instance.
//!
//! It adds a [`SimulationTime`] clock which is the generic [`Time`] resource for the [`SimulationUpdate`] schedule.
//! This schedule is run on a fixed timestep with [`SimulationTime`], and is advanced up to [`SimulationTimeTarget`].
//!
//! It also controls when [`WorldUpdate`]s are applied with a [`WorldUpdateQueue`].
//!
//! It provides schedules for extracting and resetting the simulation,
//! along with utilities for mapping entities across simulation instances in the [`simulation_entity`] module.

use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use bevy::{
    ecs::{entity::MapEntities, intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::common::scheme::PredictionScheme;

pub mod extract_component;
pub mod extract_resource;
pub mod simulation_entity;

/// This is the first schedule to run on all simulation instances and only ever runs once.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationStartup;

/// This schedule resets the simulation instance.
/// Add systems to this schedule that remove data belonging to the simulation, as well as initialize any new data.
///
/// This is different from [`SimulationStartup`] in that it may run multiple times over the lifetime of the world.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResetSimulation;

/// This schedule runs on a fixed timestep with [`SimulationTime`].
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationUpdate;

/// Schedule that extracts the simulation state from a [`SourceWorld`] into the current (local) world.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractSimulation;

/// Holds the source world during the [`ExtractSimulation`] schedule.
#[derive(Resource, Deref, DerefMut)]
pub struct SourceWorld(pub World);

/// The [`Time`] resource for the [`SimulationUpdate`].
///
/// This time resource is advanced on a fixed timestep.
///
/// Wherever the simulation runs this is the generic time resource.
///
/// When outside of [`SimulationUpdate`] this time resource will contain the time of the next simulation step.
#[derive(Default, Clone)]
pub struct SimulationTime;

/// System set where [`SimulationUpdate`] is run.
#[derive(SystemSet, Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct StepSimulationSystems;

/// This resource is used to control how far [`SimulationTime`] is advanced.
///
/// [`SimulationPlugin`] will advance [`SimulationTime`] up to this point whenever it's schedule runs.
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct SimulationTimeTarget(pub Duration);

/// A resource that exists to inform the simulation where it is running.
///
/// This should not change the behavior of the simulation,
/// but is useful for debugging and inserting different resources when building plugins based on where the simulation is running.
#[derive(Resource, Clone, Copy, Debug)]
pub enum SimulationInstance {
    Server,
    ClientMain,
    ClientServerWorld,
    ClientPrediction,
}

/// This plugin is added to all instances of the simulation.
///
/// Controls the execution of the [SimulationUpdate] and [SimulationTime].
pub(crate) struct SimulationPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
    pub instance: SimulationInstance,
}

impl<S> Plugin for SimulationPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        app.insert_resource(self.instance);

        app.add_schedule(Schedule::new(SimulationStartup));
        app.add_schedule(Schedule::new(SimulationUpdate));
        app.add_schedule(Schedule::new(ResetSimulation));

        simulation_entity::build(app);

        app.init_resource::<Time>();
        debug!("inserting simulation time");
        app.init_resource::<Time<SimulationTime>>();
        app.init_resource::<SimulationTimeTarget>();

        app.add_systems(
            self.schedule,
            run_simulation_schedule::<S>.in_set(StepSimulationSystems),
        );

        app.add_plugins(S::plugin());

        for update in S::updates().0 {
            update.build_simulation(app);
        }
    }
}

pub(crate) fn build_update<T>(app: &mut App)
where
    T: Send + Sync + 'static,
{
    app.init_resource::<WorldUpdateQueue<T>>();
}

/// Advances [SimulationTime] and the [SimulationUpdate].
fn run_simulation_schedule<S>(world: &mut World)
where
    S: PredictionScheme,
{
    // Save the current generic time to replace it after overwriting it with `SimulationTime`.
    let old_time = world.resource::<Time>().clone();

    let &SimulationTimeTarget(target_time) = world.resource::<SimulationTimeTarget>();
    let step_interval = S::step_interval();

    loop {
        let simulation_time = world.resource::<Time<SimulationTime>>();

        if simulation_time.elapsed() + step_interval > target_time {
            break;
        }

        *world.resource_mut::<Time>() = simulation_time.as_generic();
        world.run_schedule(SimulationUpdate);

        // `SimulationTime` contains the timestamp of the *next* update, so we advance it after executing `SimulationUpdate`.
        world
            .resource_mut::<Time<SimulationTime>>()
            .advance_by(step_interval);
    }

    *world.resource_mut::<Time>() = old_time;
}

/// A world update with a simulation timestamp.
#[derive(Serialize, Deserialize, Clone, MapEntities)]
pub struct WorldUpdate<T> {
    pub time: Duration,
    pub update: T,
}

/// This contains [`WorldUpdate`]s that will be applied when their simulation timetamp is reached.
///
/// You insert world updates into this queue to apply them to the simulation.
#[derive(Resource)]
pub struct WorldUpdateQueue<T> {
    updates: VecDeque<WorldUpdate<T>>,
}

impl<T> Default for WorldUpdateQueue<T> {
    fn default() -> Self {
        Self {
            updates: VecDeque::new(),
        }
    }
}

impl<T> WorldUpdateQueue<T> {
    /// Inserts an update into the queue, which will be applied when its simulation timestamp is reached.
    pub fn insert(&mut self, update: WorldUpdate<T>) {
        let (Ok(index) | Err(index)) = self.updates.binary_search_by(|e| e.time.cmp(&update.time));

        self.updates.insert(index, update);
    }
}

/// This system parameter will return simulation updates from their [`WorldUpdateQueue`] that are ready to be applied.
///
/// For every world update there should be a system in [`SimulationUpdate`] that calls [`ReadyUpdates::drain`] and applies the updates to the world.
#[derive(SystemParam)]
pub struct ReadyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    updates: ResMut<'w, WorldUpdateQueue<T>>,
    time: ResMut<'w, Time<SimulationTime>>,
}

impl<'w, T> ReadyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    /// Returns an iterator over the updates that should be applied this simulation step.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(move || {
            let front = self.updates.updates.front()?;

            if front.time > self.time.elapsed() {
                return None;
            }

            if front.time != self.time.elapsed() {
                warn!(
                    "Returned an update for {} late at {}",
                    front.time.as_millis(),
                    self.time.elapsed().as_millis()
                )
            }

            self.updates.updates.pop_front().map(|update| update.update)
        })
    }
}
