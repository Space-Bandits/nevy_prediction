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

use crate::common::{
    scheme::PredictionScheme,
    simulation::{
        schedules::{ExtractSimulation, SimulationMain, SimulationUpdate},
        simulation_entity::DespawnSimulationEntities,
        update_component::UpdateComponentSystems,
    },
};

pub mod extract_component;
pub mod extract_relation;
pub mod extract_resource;
pub mod schedules;
pub mod simulation_entity;
pub mod update_component;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExtractSimulationSystems {
    ExtractEntities,
    /// Where components and relations added by
    /// [`ExtractSimulationComponentPlugin`](extract_component::ExtractSimulationComponentPlugin)
    /// and relations added by [`ExtractSimulationRelationPlugin`](extract_relation::ExtractSimulationRelationPlugin)
    /// are extracted.
    ExtractComponents,
}

/// Holds the source world during the [`ExtractSimulation`] schedule.
#[derive(Resource, Deref, DerefMut)]
pub struct SourceWorld(pub World);

#[derive(
    Clone,
    Copy,
    Default,
    Debug,
    Hash,
    Deref,
    DerefMut,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
pub struct SimulationTick(pub u32);

/// The [`Time`] resource for the [`SimulationUpdate`].
///
/// This time resource is advanced on a fixed timestep.
///
/// Wherever the simulation runs this is the generic time resource.
///
/// When outside of [`SimulationUpdate`] this time resource will contain the time of the next simulation step.
#[derive(Default, Clone)]
pub struct SimulationTime {
    current_tick: SimulationTick,
    target_tick: SimulationTick,
}

/// System set where [`SimulationUpdate`] is run.
#[derive(SystemSet, Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct StepSimulationSystems;

/// A resource that exists to inform the simulation where it is running.
///
/// This should not change the behavior of the simulation,
/// but is useful for debugging and inserting different resources when building plugins based on where the simulation is running.
#[derive(Resource, Clone, Copy, Debug)]
pub enum SimulationInstance {
    Server,
    ClientMain,
    ClientTemplate,
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
        schedules::build(app);

        app.insert_resource(self.instance);

        simulation_entity::build(app);

        app.configure_sets(
            SimulationUpdate,
            (UpdateComponentSystems, DespawnSimulationEntities).chain(),
        );

        app.configure_sets(
            ExtractSimulation,
            (
                ExtractSimulationSystems::ExtractEntities,
                ExtractSimulationSystems::ExtractComponents,
            )
                .chain(),
        );

        if app.world().get_resource::<Time>().is_none() {
            app.init_resource::<Time>();
        }
        app.init_resource::<Time<SimulationTime>>();

        app.add_systems(
            self.schedule,
            run_simulation_schedule::<S>.in_set(StepSimulationSystems),
        );

        app.add_plugins(S::plugin());

        // for update in S::updates().0 {
        //     update.build_simulation(app);
        // }
    }
}

pub(crate) fn build_update<T>(app: &mut App)
where
    T: Send + Sync + 'static,
{
    app.init_resource::<UpdateExecutionQueue<T>>();
}

/// Advances [SimulationTime] and the [SimulationUpdate].
fn run_simulation_schedule<S>(world: &mut World)
where
    S: PredictionScheme,
{
    // Save the current generic time to replace it after overwriting it with `SimulationTime`.
    let old_time = world.resource::<Time>().clone();

    loop {
        let simulation_time = world.resource::<Time<SimulationTime>>();

        if simulation_time.current_tick() >= simulation_time.target_tick() {
            break;
        }

        *world.resource_mut::<Time>() = simulation_time.as_generic();
        world.run_schedule(SimulationMain);

        // `SimulationTime` contains the timestamp of the *next* update, so we advance it after executing `SimulationUpdate`.
        world.resource_mut::<Time<SimulationTime>>().step::<S>();
    }

    *world.resource_mut::<Time>() = old_time;
}

impl SimulationTick {
    /// Calculates what timestamp the simulation should be at given a prediction scheme
    pub fn time<S>(self) -> Duration
    where
        S: PredictionScheme,
    {
        S::step_interval() * *self
    }
}

/// A world update with a simulation timestamp.
#[derive(Serialize, Deserialize, Clone, MapEntities)]
pub struct WorldUpdate<T> {
    pub tick: SimulationTick,
    pub update: T,
}

/// An ordered queue of [`WorldUpdate`]s
#[derive(Deref, DerefMut)]
pub struct WorldUpdateQueue<T>(VecDeque<WorldUpdate<T>>);

impl<T> Default for WorldUpdateQueue<T> {
    fn default() -> Self {
        WorldUpdateQueue(VecDeque::new())
    }
}

impl<T> WorldUpdateQueue<T> {
    /// Inserts a world update into the queue maintaining order.
    ///
    /// If there are updates with the same timestamp this update will be inserted *after* the existing ones.
    pub fn insert(&mut self, update: WorldUpdate<T>) {
        let index = self.0.partition_point(|e| e.tick <= update.tick);

        self.0.insert(index, update);
    }

    /// Returns the next world update in the queue that is timestamped at or before the given tick.
    pub fn next(&mut self, tick: SimulationTick) -> Option<WorldUpdate<T>> {
        let front = self.0.front()?;

        if front.tick > tick {
            return None;
        }

        self.0.pop_front()
    }
}

/// This contains [`WorldUpdate`]s that will be applied when their simulation timetamp is reached.
///
/// You insert world updates into this queue to apply them to the simulation.
#[derive(Resource, Deref, DerefMut)]
pub struct UpdateExecutionQueue<T>(WorldUpdateQueue<T>);

impl<T> Default for UpdateExecutionQueue<T> {
    fn default() -> Self {
        UpdateExecutionQueue(WorldUpdateQueue::default())
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
    instance: Res<'w, SimulationInstance>,
    updates: ResMut<'w, UpdateExecutionQueue<T>>,
    time: Res<'w, Time<SimulationTime>>,
}

impl<'w, T> ReadyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    /// Returns an iterator over the updates that should be applied this simulation step.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(move || {
            let Some(update) = self.updates.next(self.time.current_tick()) else {
                return None;
            };

            if update.tick != self.time.current_tick() {
                warn!(
                    "Returned an update `{}` late by {} ticks in instance {:?}",
                    std::any::type_name::<T>(),
                    (*self.time.current_tick()).saturating_sub(*update.tick),
                    *self.instance,
                )
            }

            Some(update.update)
        })
    }
}

pub(crate) trait PrivateSimulationTimeExt {
    fn from_tick<S>(tick: SimulationTick) -> Self
    where
        S: PredictionScheme;

    fn extract_time(&self, other: &mut Self);

    fn step<S>(&mut self)
    where
        S: PredictionScheme;

    fn queue_ticks(&mut self, ticks: u32);

    fn clear_target(&mut self);
}

impl PrivateSimulationTimeExt for Time<SimulationTime> {
    fn from_tick<S>(tick: SimulationTick) -> Self
    where
        S: PredictionScheme,
    {
        let mut time = Time::new_with(SimulationTime {
            current_tick: tick,
            target_tick: tick,
        });

        let target_time = time.current_tick().time::<S>();
        time.advance_to(target_time.saturating_sub(S::step_interval())); // ensures delta is set correctly
        time.advance_to(target_time);

        time
    }

    /// Extracts the clock without copying the target time.
    fn extract_time(&self, other: &mut Self) {
        let target_tick = other.context_mut().target_tick;
        *other = self.clone();
        other.context_mut().target_tick = target_tick;
    }

    fn step<S>(&mut self)
    where
        S: PredictionScheme,
    {
        *self.context_mut().current_tick += 1;
        self.advance_to(self.current_tick().time::<S>());
    }

    fn queue_ticks(&mut self, ticks: u32) {
        *self.context_mut().target_tick += ticks;
    }

    fn clear_target(&mut self) {
        self.context_mut().target_tick = self.context().current_tick;
    }
}

pub trait SimulationTimeExt {
    fn current_tick(&self) -> SimulationTick;

    fn target_tick(&self) -> SimulationTick;
}

impl SimulationTimeExt for Time<SimulationTime> {
    /// Returns the simulation tick of the current simulation tick.
    ///
    /// When not in [`SimulationUpdate`] this is the next tick,
    /// not the one that was just executed.
    fn current_tick(&self) -> SimulationTick {
        self.context().current_tick
    }

    fn target_tick(&self) -> SimulationTick {
        self.context().target_tick
    }
}
