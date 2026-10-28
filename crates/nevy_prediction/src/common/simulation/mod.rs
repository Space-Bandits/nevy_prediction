//! This module contains logic that controls the execution of a simulation instance.
//!
//! It adds a [`SimulationTime`] clock which is the generic [`Time`] resource for the [`SimulationUpdate`] schedule.
//! This schedule is run on a fixed timestep with [`SimulationTime`],
//! in the server app's world and in the client's simulation worlds, which are stepped on simulation threads.
//!
//! It also controls when [`WorldUpdate`]s are applied with a [`WorldUpdateQueue`].
//!
//! It provides schedules for extracting and resetting the simulation,
//! along with utilities for mapping entities across simulation instances in the [`simulation_entity`] module.

use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use bevy::{
    ecs::{entity::MapEntities, system::SystemParam},
    prelude::*,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::common::{
    scheme::PredictionScheme,
    simulation::{
        schedules::{ExtractSimulation, SimulationUpdate},
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
pub(crate) mod world;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExtractSimulationSystems {
    /// Where new simulation entities are extracted.
    ExtractSimulationEntities,
    /// Where components added by
    /// [`ExtractSimulationComponentPlugin`](extract_component::ExtractSimulationComponentPlugin)
    /// and relations added by [`ExtractSimulationRelationPlugin`](extract_relation::ExtractSimulationRelationPlugin)
    /// are extracted.
    ExtractComponents,
    /// Where simulation entities that no longer exist in the source world are despawned.
    DespawnSimulationEntities,
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

/// System set in the app where the app synchronizes with the simulation.
///
/// On the server the simulation is stepped in this set, in the app's own world.
/// On the client the simulation runs asynchronously to the app on separate threads,
/// and in this set its latest state is extracted into the app's world.
/// Both advance [`Time<PredictionTime>`](crate::common::time::PredictionTime) here.
///
/// Order systems that read the simulation state after this set.
#[derive(SystemSet, Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct SyncSimulationSystems;

/// A resource that exists to inform the simulation where it is running.
///
/// This should not change the behavior of the simulation,
/// but is useful for debugging and inserting different resources when building plugins based on where the simulation is running.
///
/// The simulation is only executed in [stepping](Self::steps) instances.
/// The server's is the server app's world, and the client's each have their own world on a simulation thread.
/// The other instances only receive simulation state through the [`ExtractSimulation`] schedule.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimulationInstance {
    /// The authoritative simulation, which is the server app's world.
    Server,
    /// The client's local simulation, which runs ahead of the server by the prediction interval.
    ///
    /// It is stepped on its own, and has the [`ClientPrediction`](Self::ClientPrediction) world
    /// extracted into it whenever a prediction finishes.
    ClientMain,
    /// The client's copy of the server's simulation, built from the world updates that the server sends.
    ClientTemplate,
    /// A world that is forked from the [`ClientTemplate`](Self::ClientTemplate) world and re-simulated up to the
    /// [`ClientMain`](Self::ClientMain) world's tick with the updates that the server hasn't confirmed yet.
    ClientPrediction,
    /// The client app's world, which the [`ClientMain`](Self::ClientMain) simulation is extracted into after it ticks.
    ///
    /// This is the world that is rendered.
    ClientApp,
    /// An internal buffer that a simulation thread extracts into after it ticks,
    /// so that the app can extract the latest state without waiting for the simulation thread.
    ///
    /// Don't add presentation logic to this instance, it is not the app's world.
    Snapshot,
}

impl SimulationInstance {
    pub(crate) fn format_tracing_str(self) -> &'static str {
        match self {
            SimulationInstance::Server => "Server",
            SimulationInstance::ClientMain => "ClientMain",
            SimulationInstance::ClientTemplate => "ClientTemplate",
            SimulationInstance::ClientPrediction => "ClientPrediction",
            SimulationInstance::ClientApp => "ClientApp",
            SimulationInstance::Snapshot => "Snapshot",
        }
    }

    /// Returns true if this instance executes the simulation.
    ///
    /// Other instances only receive simulation state through extraction.
    pub fn steps(self) -> bool {
        matches!(
            self,
            SimulationInstance::Server
                | SimulationInstance::ClientMain
                | SimulationInstance::ClientTemplate
                | SimulationInstance::ClientPrediction
        )
    }

    /// Returns true if this is an app's world that the simulation is extracted into.
    ///
    /// The server app's world is not one, it is the [`Server`](Self::Server) simulation itself.
    pub fn is_app(self) -> bool {
        matches!(self, SimulationInstance::ClientApp)
    }
}

/// This plugin is added to all instances of the simulation.
///
/// Adds the simulation schedules, [`SimulationTime`] and the [`PredictionScheme`]'s plugin.
/// Stepping instances are advanced with [`SimulationWorld::step`](world::SimulationWorld::step).
pub(crate) struct SimulationPlugin<S> {
    _p: PhantomData<S>,
    instance: SimulationInstance,
}

impl<S> SimulationPlugin<S> {
    pub fn new(instance: SimulationInstance) -> Self {
        SimulationPlugin {
            _p: PhantomData,
            instance,
        }
    }
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
                ExtractSimulationSystems::ExtractSimulationEntities,
                ExtractSimulationSystems::ExtractComponents,
                ExtractSimulationSystems::DespawnSimulationEntities,
            )
                .chain(),
        );

        if app.world().get_resource::<Time>().is_none() {
            app.init_resource::<Time>();
        }
        app.init_resource::<Time<SimulationTime>>();

        if let SimulationInstance::ClientPrediction = self.instance {
            crate::client::prediction::build_prediction_world(app);
        }

        app.add_plugins(S::plugin());
    }
}

pub(crate) fn build_update<T>(app: &mut App)
where
    T: Send + Sync + 'static,
{
    app.init_resource::<UpdateExecutionQueue<T>>();
    app.add_systems(schedules::ResetSimulation, clear_update_queue::<T>);
}

/// Removes queued updates when the simulation is reset, they belong to the previous simulation.
fn clear_update_queue<T>(mut queue: ResMut<UpdateExecutionQueue<T>>)
where
    T: Send + Sync + 'static,
{
    queue.clear();
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
                    "An update `{}` was returned {} ticks late in {:?}.",
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
