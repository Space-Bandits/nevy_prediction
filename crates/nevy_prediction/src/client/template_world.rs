//! This module is responsible for receiving [`WorldUpdate`]s from the server
//! and applying them to a separate instance of the simulation in the [`ServerWorld`] resource.
//!
//! This local copy of the server's simulation is then used to predict the future state of the simulation.

use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use log::warn;
use nevy::prelude::*;

use crate::{
    client::{
        ClientSimulationSystems, PredictionBudget, PredictionServerConnection,
        prediction::{PredictionUpdates, PredictionWorld},
        simulation_world::SimulationWorld,
    },
    common::{
        ServerWorldUpdate, UpdateServerTick,
        scheme::PredictionScheme,
        simulation::{
            SimulationInstance, SimulationPlugin, SimulationTick, SimulationTime,
            SimulationTimeExt, UpdateExecutionQueue, schedules::SimulationStartupMain,
        },
    },
};

pub(crate) fn build<S>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    S: PredictionScheme,
{
    app.insert_resource(TemplateWorld::build::<S>());
    app.init_resource::<ServerTickSamples>();

    app.add_systems(
        schedule,
        (
            receive_time_updates::<S>.in_set(ClientSimulationSystems::ReceiveUpdates),
            run_template_world.in_set(ClientSimulationSystems::RunTemplateWorld),
        ),
    );
}

pub(crate) fn build_update<T>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    T: Send + Sync + 'static + Clone,
{
    app.add_systems(
        schedule,
        receive_world_updates::<T>.in_set(ClientSimulationSystems::ReceiveUpdates),
    );
}

/// Contains a [`SimulationWorld`] that holds the most recently known state of the simulation according to the server.
#[derive(Resource, Deref, DerefMut)]
pub(crate) struct TemplateWorld(pub SimulationWorld);

impl TemplateWorld {
    pub fn build<S>() -> Self
    where
        S: PredictionScheme,
    {
        let mut app = App::empty();

        app.add_schedule(Schedule::new(Main));

        app.add_plugins(SimulationPlugin::<S> {
            _p: PhantomData,
            schedule: Main.intern(),
            instance: SimulationInstance::ClientTemplate,
        });

        app.world_mut().run_schedule(SimulationStartupMain);

        TemplateWorld(SimulationWorld::build(app))
    }
}

fn receive_world_updates<T>(
    mut server_world: ResMut<TemplateWorld>,
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<ServerWorldUpdate<T>>,
        Has<PredictionServerConnection>,
    )>,
    mut prediction_world: ResMut<PredictionWorld>,
) where
    T: Send + Sync + 'static + Clone,
{
    for (connection_entity, mut messages, is_server) in &mut message_q {
        for ServerWorldUpdate {
            update,
            include_in_prediction,
        } in messages.drain()
        {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            if include_in_prediction {
                let mut prediction_updates =
                    prediction_world.resource_mut::<PredictionUpdates<T>>();
                prediction_updates.insert(update.clone());
            }

            server_world
                .resource_mut::<UpdateExecutionQueue<T>>()
                .insert(update);
        }
    }
}

/// Contains the most recent server time update.
#[derive(Resource, Default)]
pub struct ServerTickSamples {
    latest: SimulationTick,
    samples: VecDeque<(Duration, SimulationTick)>,
}

impl ServerTickSamples {
    const SERVER_TIME_ESTIMATE_SAMPLES: usize = 32;

    pub fn push<S>(&mut self, real_time: Duration, tick: SimulationTick)
    where
        S: PredictionScheme,
    {
        self.latest = tick;

        self.samples.push_back((real_time, tick));

        while self.samples.len() > Self::SERVER_TIME_ESTIMATE_SAMPLES {
            self.samples.pop_front();
        }
    }

    pub fn reset<S>(&mut self, current_time: Duration, tick: SimulationTick)
    where
        S: PredictionScheme,
    {
        *self = default();

        self.push::<S>(current_time, tick);
    }

    pub fn latest(&self) -> SimulationTick {
        self.latest
    }

    pub fn estimated_time<S>(&self, real_time: Duration) -> Duration
    where
        S: PredictionScheme,
    {
        self.samples
            .iter()
            .map(|&(received_time, sample)| {
                let elapsed = real_time - received_time;
                let sample_time = sample.time::<S>();

                sample_time + elapsed
            })
            .sum::<Duration>()
            .checked_div(self.samples.len() as u32)
            .unwrap_or_default()
    }
}

/// Responsible for receiving [`UpdateServerTick`]s.
fn receive_time_updates<S>(
    mut message_q: Query<&mut ReceivedMessages<UpdateServerTick>>,
    mut tick_samples: ResMut<ServerTickSamples>,
    real_time: Res<Time<Real>>,
    // mut time: ResMut<Time<SimulationTime>>,
    // prediction_interval: Res<PredictionInterval>,
) -> Result
where
    S: PredictionScheme,
{
    for mut messages in &mut message_q {
        for UpdateServerTick { simulation_tick } in messages.drain() {
            tick_samples.push::<S>(real_time.elapsed(), simulation_tick);
        }
    }

    Ok(())
}

fn run_template_world(
    mut budget: ResMut<PredictionBudget>,
    time: Res<ServerTickSamples>,
    mut template_world: ResMut<TemplateWorld>,
) {
    let current_tick = template_world
        .resource::<Time<SimulationTime>>()
        .current_tick();
    let desired_tick = time.latest();

    let desired_ticks = *desired_tick - *current_tick;

    let execute_ticks = desired_ticks.min(budget.template);

    if execute_ticks == 0 {
        return;
    }

    budget.template -= execute_ticks;
    template_world.run(execute_ticks);
}
