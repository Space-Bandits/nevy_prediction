use std::{marker::PhantomData, ops::DerefMut};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};

use crate::{
    client::{
        ClientSimulationSystems, PredictionBudget, simulation_world::SimulationWorld,
        template_world::TemplateWorld,
    },
    common::{
        scheme::PredictionScheme,
        simulation::{
            PrivateSimulationTimeExt, SimulationInstance, SimulationPlugin, SimulationTick,
            SimulationTime, SimulationTimeExt, UpdateExecutionQueue, WorldUpdateQueue,
            schedules::{SimulationPreUpdate, SimulationStartupMain},
        },
    },
};

pub(crate) fn build<S>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    S: PredictionScheme,
{
    app.insert_resource(PredictionWorld::new::<S>());
    app.init_resource::<LastPredictedTick>();

    app.add_systems(
        schedule,
        run_prediction_world.in_set(ClientSimulationSystems::RunPredictionWorld),
    );
}

pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static + Clone,
{
    let mut prediction_world = app.world_mut().resource_mut::<PredictionWorld>();

    prediction_world.init_resource::<PredictionUpdates<T>>();
    prediction_world
        .resource_mut::<Schedules>()
        .add_systems(SimulationPreUpdate, queue_prediction_updates::<T>);

    app.add_systems(
        schedule,
        drain_prediction_updates::<T>.in_set(ClientSimulationSystems::QueuePredictionUpdates),
    );
}

/// The last tick that the prediction world predicted from.
#[derive(Resource, Default, Deref, DerefMut)]
struct LastPredictedTick(SimulationTick);

/// Contains the [`ParallelWorld`] used for prediction.
#[derive(Resource, Deref, DerefMut)]
pub(crate) struct PredictionWorld {
    #[deref]
    pub world: SimulationWorld,
    state: PredictionWorldState,
}

#[derive(Clone, Copy)]
enum PredictionWorldState {
    Idle,
    Running,
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

        app.world_mut().run_schedule(SimulationStartupMain);

        PredictionWorld {
            world: SimulationWorld::build(app),
            state: PredictionWorldState::Idle,
        }
    }
}

fn run_prediction_world(world: &mut World) {
    let mut prediction_world = world.remove_resource::<PredictionWorld>().unwrap();

    loop {
        match prediction_world.state {
            PredictionWorldState::Idle => {
                let current_template_tick = world
                    .resource::<TemplateWorld>()
                    .resource::<Time<SimulationTime>>()
                    .current_tick();

                let last_predicted_tick = **world.resource::<LastPredictedTick>();

                if current_template_tick == last_predicted_tick {
                    break;
                }

                world
                    .resource_mut::<TemplateWorld>()
                    .extract(prediction_world.deref_mut());

                prediction_world
                    .resource_mut::<Time<SimulationTime>>()
                    .clear_target();

                prediction_world.state = PredictionWorldState::Running;

                debug!("Started prediction app")
            }
            PredictionWorldState::Running => {
                let current_tick = prediction_world
                    .resource::<Time<SimulationTime>>()
                    .current_tick();
                let desired_tick = world.resource::<Time<SimulationTime>>().current_tick();

                let mut budget = world.resource_mut::<PredictionBudget>();

                if budget.prediction == 0 {
                    // not enough prediction budget
                    break;
                }

                let desired_ticks = desired_tick.saturating_sub(*current_tick);
                let execute_ticks = desired_ticks.min(budget.prediction);

                debug!("Running prediction app for {} ticks", execute_ticks);

                budget.prediction -= execute_ticks;
                prediction_world.run(execute_ticks);

                if desired_tick
                    == prediction_world
                        .resource::<Time<SimulationTime>>()
                        .current_tick()
                {
                    debug!("Finished prediction and extracting");

                    prediction_world.extract(world);
                    prediction_world.state = PredictionWorldState::Idle;
                }
            }
        }
    }

    world.insert_resource(prediction_world);
}

/// Contains a sorted list of world updates that haven't been reconciled with the server.
///
/// This resource is inserted into the [`PredictionWorld`] and updates of the matching frame add added to the [`UpdateExecutionQueue`] every update.
#[derive(Resource, Deref, DerefMut)]
pub(crate) struct PredictionUpdates<T>(WorldUpdateQueue<T>);

impl<T> Default for PredictionUpdates<T> {
    fn default() -> Self {
        PredictionUpdates(WorldUpdateQueue::default())
    }
}

/// Removes prediction updates that have been reconciled
fn drain_prediction_updates<T>(
    mut prediction_world: ResMut<PredictionWorld>,
    template_world: ResMut<TemplateWorld>,
) where
    T: Send + Sync + 'static,
{
    let mut updates = prediction_world.resource_mut::<PredictionUpdates<T>>();

    let reconciled_tick = template_world
        .resource::<Time<SimulationTime>>()
        .current_tick();

    while let Some(front) = updates.front() {
        if front.tick < reconciled_tick {
            updates.pop_front();
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

            debug!(
                "queued an update {} for prediction",
                std::any::type_name::<T>()
            );
        }
    }
}

// /// When the prediction world finishes, it will not contain any predicted world upates that were added while it was running.
// /// This system adds any updates that are newer than it's current simulation time, which will then be "predicted" on the main app.
// fn reapply_new_world_updates<T>(
//     mut prediction_world: ResMut<PredictionWorld>,
//     predicted_time: Res<Time<SimulationTime>>,
//     prediction_updates: Res<PredictionUpdates<T>>,
//     mut prediction_queue: ResMut<UpdateExecutionQueue<T>>,
// ) where
//     T: Send + Sync + 'static + Clone,
// {
//     if prediction_world.extracted {
//         return;
//     }

//     if prediction_world.world.get().is_none() {
//         return;
//     };

//     for update in prediction_updates.iter() {
//         if update.time >= predicted_time.elapsed() {
//             prediction_queue.insert(update.clone());
//         }
//     }
// }
