use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use bevy::{
    ecs::{entity::MapEntities, intern::Interned, schedule::ScheduleLabel, system::SystemParam},
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::common::scheme::PredictionScheme;

pub mod simulation_entity;

/// This schedule runs on a fixed timestep with [SimulationTime].
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationSchedule;

/// The [Time] resource for the [SimulationSchedule].
///
/// This time resource is advanced on a fixed timestep.
///
/// Wherever the simulatin runs this is the generic time resource.
///
/// When outside of [SimulationSchedule] this time resource will contain the time of the next simulation step.
#[derive(Default, Clone)]
pub struct SimulationTime;

/// System set where [SimulationSchedule] is run.
#[derive(SystemSet, Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct StepSimulation;

/// This resource is used to control how far [SimulationTime] is advanced to.
///
/// [SimulationPlugin] will advance [SimulationTime] up to this point whenever it's schedule runs.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct SimulationTimeTarget(pub Duration);

/// Controls the fixed timestep of [SimulationTime]
#[derive(Resource, Deref)]
struct SimulationStepInterval(Duration);

/// A resource that exists to inform the simulation where it is running.
#[derive(Resource, Clone, Copy, Debug)]
pub enum SimulationInstance {
    Server,
    ClientMain,
    ClientServerWorld,
    ClientPrediction,
}

/// This plugin is added to all instances of the simulation.
///
/// Controls the execution of the [SimulationSchedule] and [SimulationTime].
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

        app.add_schedule(Schedule::new(SimulationSchedule));

        simulation_entity::build(app);

        app.init_resource::<Time>();
        debug!("inserting simulation time");
        app.init_resource::<Time<SimulationTime>>();
        app.init_resource::<SimulationTimeTarget>();
        app.insert_resource(SimulationStepInterval(S::step_interval()));

        app.add_systems(
            self.schedule,
            run_simulation_schedule.in_set(StepSimulation),
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
    app.init_resource::<UpdateQueue<T>>();
}

/// Advances [SimulationTime] and the [SimulationSchedule].
fn run_simulation_schedule(world: &mut World) {
    // Save the current generic time to replace it after using it for the
    let old_time = world.resource::<Time>().clone();

    let &SimulationTimeTarget(target_time) = world.resource::<SimulationTimeTarget>();
    let &SimulationStepInterval(step_interval) = world.resource::<SimulationStepInterval>();

    loop {
        let simulation_time = world.resource::<Time<SimulationTime>>();

        if simulation_time.elapsed() + step_interval > target_time {
            break;
        }

        *world.resource_mut::<Time>() = simulation_time.as_generic();
        world.run_schedule(SimulationSchedule);

        world
            .resource_mut::<Time<SimulationTime>>()
            .advance_by(step_interval);
    }

    *world.resource_mut::<Time>() = old_time;
}

/// A world update with a simulation timestamp
#[derive(Serialize, Deserialize, Clone, MapEntities)]
pub(crate) struct WorldUpdate<T> {
    pub(crate) time: Duration,
    pub(crate) update: T,
}

/// Contains a queue of world updates to be applied when their time is reached
#[derive(Resource)]
pub(crate) struct UpdateQueue<T> {
    pub updates: VecDeque<WorldUpdate<T>>,
}

impl<T> Default for UpdateQueue<T> {
    fn default() -> Self {
        Self {
            updates: VecDeque::new(),
        }
    }
}

impl<T> UpdateQueue<T> {
    /// Inserts an update into the queue maintaining the order of the queued updates.
    pub fn insert(&mut self, update: WorldUpdate<T>) {
        let (Ok(index) | Err(index)) = self.updates.binary_search_by(|e| e.time.cmp(&update.time));

        self.updates.insert(index, update);
    }
}

/// Use this system parameter to read updates that need to be applied to the simulation now
#[derive(SystemParam)]
pub struct ReadyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    updates: ResMut<'w, UpdateQueue<T>>,
    time: ResMut<'w, Time<SimulationTime>>,
}

impl<'w, T> ReadyUpdates<'w, T>
where
    T: Send + Sync + 'static,
{
    pub fn read(&mut self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(move || {
            let front = self.updates.updates.front()?;

            if front.time > self.time.elapsed() {
                return None;
            }

            self.updates.updates.pop_front().map(|update| update.update)
        })
    }
}
