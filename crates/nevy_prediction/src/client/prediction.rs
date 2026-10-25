//! Systems that run inside the [`ClientPrediction`](crate::common::simulation::SimulationInstance::ClientPrediction) world.

use bevy::prelude::*;

use crate::common::simulation::{
    SimulationTick, SimulationTime, SimulationTimeExt, UpdateExecutionQueue, WorldUpdateQueue,
    schedules::{ResetSimulation, SimulationPreUpdate},
};

/// Called when building the prediction world.
pub(crate) fn build_prediction_world(app: &mut App) {
    app.init_resource::<LastPredictedTick>();
}

/// Called when building the prediction world for each world update.
pub(crate) fn build_update<T>(app: &mut App)
where
    T: Send + Sync + 'static + Clone,
{
    app.init_resource::<PredictionUpdates<T>>();
    app.add_systems(
        SimulationPreUpdate,
        (drain_prediction_updates::<T>, queue_prediction_updates::<T>).chain(),
    );
    app.add_systems(ResetSimulation, clear_prediction_updates::<T>);
}

/// The tick that the prediction world was last forked from the template world at.
#[derive(Resource, Default, Clone, Deref, DerefMut)]
pub(crate) struct LastPredictedTick(pub SimulationTick);

/// Contains a sorted list of world updates that haven't been reconciled with the server.
///
/// This resource is in the prediction world, and updates for the current tick are added to the [`UpdateExecutionQueue`] every tick.
#[derive(Resource, Deref, DerefMut)]
pub(crate) struct PredictionUpdates<T>(WorldUpdateQueue<T>);

impl<T> Default for PredictionUpdates<T> {
    fn default() -> Self {
        PredictionUpdates(WorldUpdateQueue::default())
    }
}

/// Removes all prediction updates when the simulation is reset, they belong to the previous simulation.
fn clear_prediction_updates<T>(mut prediction_updates: ResMut<PredictionUpdates<T>>)
where
    T: Send + Sync + 'static,
{
    prediction_updates.clear();
}

/// Removes prediction updates that have been reconciled
fn drain_prediction_updates<T>(
    mut prediction_updates: ResMut<PredictionUpdates<T>>,
    prediction_tick: Res<LastPredictedTick>,
) where
    T: Send + Sync + 'static,
{
    while let Some(front) = prediction_updates.front() {
        if front.tick < **prediction_tick {
            prediction_updates.pop_front();
        } else {
            break;
        }
    }
}

/// Runs in [`SimulationPreUpdate`] on the prediction app.
///
/// Queues any updates that should happen this tick on the prediction app.
fn queue_prediction_updates<T>(
    prediction_updates: Res<PredictionUpdates<T>>,
    mut queue: ResMut<UpdateExecutionQueue<T>>,
    time: Res<Time<SimulationTime>>,
) where
    T: Send + Sync + 'static + Clone,
{
    for update in prediction_updates.iter() {
        if update.tick == time.current_tick() {
            queue.insert(update.clone());
        }
    }
}
