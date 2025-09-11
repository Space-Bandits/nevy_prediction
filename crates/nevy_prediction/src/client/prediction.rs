use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};

use crate::{
    client::{ClientSimulationSystems, parallel_app::ParallelWorld, server_world::ServerWorld},
    common::{
        prelude::{ExtractSimulation, SourceWorld},
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationPlugin, SimulationStartup, SimulationTime,
            SimulationTimeTarget, UpdateExecutionQueue, WorldUpdateQueue,
        },
    },
};

pub(crate) fn build<S>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    S: PredictionScheme,
{
    app.insert_non_send_resource(PredictionWorld::new::<S>());

    app.add_systems(
        schedule,
        (
            poll_prediction_world.in_set(ClientSimulationSystems::PollParallelAppsSystems),
            (
                set_prediction_target,
                extract_predicton_world,
                extract_server_world,
            )
                .chain()
                .in_set(ClientSimulationSystems::ExtractSimulationsSystems),
            set_prediction_wrold_extracted
                .after(ClientSimulationSystems::ReapplyNewWorldUpdatesSystems),
            run_prediction_app.in_set(ClientSimulationSystems::RunParallelAppsSystems),
        ),
    );
}

pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static + Clone,
{
    app.init_resource::<PredictionUpdates<T>>();

    app.add_systems(
        schedule,
        (
            reapply_new_world_updates::<T>
                .in_set(ClientSimulationSystems::ReapplyNewWorldUpdatesSystems),
            (drain_prediction_updates::<T>, queue_prediction_updates::<T>)
                .in_set(ClientSimulationSystems::QueuePredictionAppUpdatesSystems),
        ),
    );
}

/// Controls how far prediction is run.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct PredictionInterval(pub Duration);

/// Contains the [`ParallelWorld`] used for prediction.
pub(crate) struct PredictionWorld {
    pub world: ParallelWorld,
    /// whether the finished predicted world has been extracted into the main app.
    pub extracted: bool,
    /// Whether the prediction app will be run this tick.
    pub prediction_needed: bool,
}

impl PredictionWorld {
    pub fn new<S>() -> Self
    where
        S: PredictionScheme,
    {
        let mut app = App::empty();

        app.add_schedule(Schedule::new(Main));
        app.main_mut().update_schedule = Some(Main.intern());

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: Main.intern(),
            instance: SimulationInstance::ClientPrediction,
        });

        app.world_mut().run_schedule(SimulationStartup);

        PredictionWorld {
            world: ParallelWorld::new(app),
            extracted: true,
            prediction_needed: false,
        }
    }
}

fn poll_prediction_world(mut prediction_world: NonSendMut<PredictionWorld>) {
    prediction_world.world.poll();
}

fn set_prediction_target(
    simulation_target: Res<SimulationTimeTarget>,
    mut prediction_world_state: NonSendMut<PredictionWorld>,
) {
    let Some(prediction_world) = prediction_world_state.world.get() else {
        return;
    };

    let mut current_target = prediction_world.resource_mut::<SimulationTimeTarget>();

    let prediction_target = **simulation_target;

    if prediction_target > **current_target {
        **current_target = prediction_target;
        prediction_world_state.prediction_needed = true;
    }
}

fn extract_predicton_world(world: &mut World, mut scratch_world: Local<Option<World>>) -> Result {
    let mut prediction_world_state = world
        .remove_non_send_resource::<PredictionWorld>()
        .ok_or("Prediction world should exist")?;

    if prediction_world_state.extracted {
        world.insert_non_send_resource(prediction_world_state);
        return Ok(());
    }

    let Some(prediction_world) = prediction_world_state.world.get() else {
        world.insert_non_send_resource(prediction_world_state);
        return Ok(());
    };

    let mut owned_prediction_world = scratch_world.take().unwrap_or_default();
    std::mem::swap(&mut owned_prediction_world, prediction_world);

    // Extract the simulation time from the source world.
    *world.resource_mut::<Time<SimulationTime>>() =
        (*owned_prediction_world.resource::<Time<SimulationTime>>()).clone();

    // Insert server world and run extract schedule.
    world.insert_resource(SourceWorld(owned_prediction_world));
    world.run_schedule(ExtractSimulation);
    let SourceWorld(mut owned_prediction_world) = world
        .remove_resource()
        .ok_or("Extract schedule removed the `SourceWorld`")?;

    // Swap server world back and replace scratch world.
    std::mem::swap(&mut owned_prediction_world, prediction_world);
    *scratch_world = Some(owned_prediction_world);

    world.insert_non_send_resource(prediction_world_state);

    Ok(())
}

fn set_prediction_wrold_extracted(mut prediction_world: NonSendMut<PredictionWorld>) {
    if prediction_world.world.get().is_none() {
        return;
    }

    prediction_world.extracted = true;
}

fn extract_server_world(
    mut server_world_state: NonSendMut<ServerWorld>,
    mut prediction_world_state: NonSendMut<PredictionWorld>,
    mut scratch_world: Local<Option<World>>,
) -> Result {
    if !prediction_world_state.prediction_needed {
        return Ok(());
    }

    // Check that both the server world app and prediction app are idle.
    let Some(server_world_app) = server_world_state.get() else {
        return Ok(());
    };

    let Some(prediction_app) = prediction_world_state.world.get() else {
        return Ok(());
    };

    // Swap server world with scratch world.
    let mut owned_server_world = scratch_world.take().unwrap_or_default();
    std::mem::swap(&mut owned_server_world, server_world_app);

    // Extract the simulation time from the source world.
    *prediction_app.resource_mut::<Time<SimulationTime>>() =
        (*owned_server_world.resource::<Time<SimulationTime>>()).clone();

    // Insert server world and run extract schedule.
    prediction_app.insert_resource(SourceWorld(owned_server_world));
    prediction_app.run_schedule(ExtractSimulation);
    let SourceWorld(mut owned_server_world) = prediction_app
        .remove_resource()
        .ok_or("Extract schedule removed the `SourceWorld`")?;

    // Swap server world back and replace scratch world.
    std::mem::swap(&mut owned_server_world, server_world_app);
    *scratch_world = Some(owned_server_world);

    Ok(())
}

fn run_prediction_app(mut prediction_app: NonSendMut<PredictionWorld>) {
    if !prediction_app.prediction_needed {
        return;
    }

    prediction_app.world.update(true);
    prediction_app.extracted = false;
}

/// Contains a sorted list of world updates that haven't been reconciled with the server.
///
/// These updates get added to the update queue of the prediction app before it is run.
#[derive(Resource, Deref, DerefMut)]
pub(crate) struct PredictionUpdates<T>(WorldUpdateQueue<T>);

impl<T> Default for PredictionUpdates<T> {
    fn default() -> Self {
        PredictionUpdates(WorldUpdateQueue::default())
    }
}

/// Removes prediction updates that have been reconciled
fn drain_prediction_updates<T>(
    mut updates: ResMut<PredictionUpdates<T>>,
    mut prediction_world: NonSendMut<PredictionWorld>,
) where
    T: Send + Sync + 'static,
{
    let Some(prediction_world) = prediction_world.world.get() else {
        return;
    };

    let reconciled_time = prediction_world
        .resource::<Time<SimulationTime>>()
        .elapsed();

    while let Some(front) = updates.front() {
        if front.time < reconciled_time {
            updates.pop_front();
        } else {
            break;
        }
    }
}

fn queue_prediction_updates<T>(
    prediction_updates: Res<PredictionUpdates<T>>,
    mut prediction_world: NonSendMut<PredictionWorld>,
) where
    T: Send + Sync + 'static + Clone,
{
    if !prediction_world.prediction_needed {
        return;
    }

    let Some(world) = prediction_world.world.get() else {
        return;
    };

    let mut queue = world.resource_mut::<UpdateExecutionQueue<T>>();

    for update in prediction_updates.iter().cloned() {
        queue.insert(update);
    }
}

/// When the prediction world finishes, it will not contain any predicted world upates that were added while it was running.
/// This system adds any updates that are newer than it's current simulation time, which will then be "predicted" on the main app.
fn reapply_new_world_updates<T>(
    mut prediction_world: NonSendMut<PredictionWorld>,
    predicted_time: Res<Time<SimulationTime>>,
    prediction_updates: Res<PredictionUpdates<T>>,
    mut prediction_queue: ResMut<UpdateExecutionQueue<T>>,
) where
    T: Send + Sync + 'static + Clone,
{
    if prediction_world.extracted {
        return;
    }

    if prediction_world.world.get().is_none() {
        return;
    };

    for update in prediction_updates.iter() {
        if update.time >= predicted_time.elapsed() {
            prediction_queue.insert(update.clone());
        }
    }
}
