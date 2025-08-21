use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use nevy::*;

use crate::{
    client::{prediction_app::PredictionApp, server_world_app::ServerWorldApp},
    common::{
        ResetClientSimulation,
        scheme::PredictionScheme,
        simulation::{
            ResetSimulation, SimulationInstance, SimulationPlugin, SimulationTime,
            SimulationTimeTarget,
        },
    },
};

pub mod parallel_app;
pub mod prediction_app;
pub mod server_world_app;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientPredictionSet {
    ResetSimulation,
    QueueUpdates,
    RunPredictionWorld,
    RunServerWorld,
}

pub struct NevyPredictionClientPlugin<S> {
    pub _p: PhantomData<S>,
    pub schedule: Interned<dyn ScheduleLabel>,
}

impl<S> Default for NevyPredictionClientPlugin<S> {
    fn default() -> Self {
        NevyPredictionClientPlugin {
            _p: PhantomData,
            schedule: Update.intern(),
        }
    }
}

impl<S> Plugin for NevyPredictionClientPlugin<S>
where
    S: PredictionScheme,
{
    fn build(&self, app: &mut App) {
        app.configure_sets(
            self.schedule,
            (
                ClientPredictionSet::ResetSimulation,
                ClientPredictionSet::QueueUpdates,
                ClientPredictionSet::RunPredictionWorld,
                ClientPredictionSet::RunServerWorld,
            )
                .chain(),
        );

        crate::common::build::<S>(app);
        server_world_app::build::<S>(app, self.schedule);
        prediction_app::build::<S>(app, self.schedule);

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: Update.intern(),
            instance: SimulationInstance::ClientMain,
        });

        app.add_systems(
            Update,
            (
                receive_reset_simulations
                    .pipe(reset_simulations)
                    .in_set(ClientPredictionSet::ResetSimulation),
                drive_simulation_time.in_set(ClientPredictionSet::QueueUpdates),
            ),
        );

        for update in S::updates().0 {
            update.build_client(app, self.schedule);
        }
    }
}

/// Is called on the client app for each world update message added by the prediction scheme
pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static,
{
    server_world_app::build_update::<T>(app, schedule);
}

fn drive_simulation_time(mut target_time: ResMut<SimulationTimeTarget>, time: Res<Time>) {
    **target_time += time.delta();
}

fn receive_reset_simulations(
    mut message_q: Query<&mut ReceivedMessages<ResetClientSimulation>>,
) -> Option<Duration> {
    let mut reset = None;

    for mut messages in &mut message_q {
        for ResetClientSimulation { simulation_time } in messages.drain() {
            reset = Some(simulation_time);
        }
    }

    reset
}

fn reset_simulations(In(reset): In<Option<Duration>>, world: &mut World) {
    let Some(elapsed) = reset else {
        return;
    };

    debug!("resetting simulation to {:?}", elapsed);

    world
        .non_send_resource_mut::<ServerWorldApp>()
        .reset(elapsed);
    world
        .non_send_resource_mut::<PredictionApp>()
        .app
        .reset(elapsed);

    let mut time = Time::new_with(SimulationTime);
    time.advance_to(elapsed);
    world.insert_resource(time);
    world.insert_resource(SimulationTimeTarget(elapsed));

    world.run_schedule(ResetSimulation);
}
