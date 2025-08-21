use std::{marker::PhantomData, time::Duration};

use bevy::{
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};

use crate::{
    client::{
        ClientPredictionSet,
        parallel_app::{ExtractSimulation, ParallelApp, SourceWorld},
        server_world_app::ServerWorldApp,
    },
    common::{
        scheme::PredictionScheme,
        simulation::{SimulationInstance, SimulationPlugin, SimulationTime, SimulationTimeTarget},
    },
};

pub(crate) fn build<S>(app: &mut App, schedule: Interned<dyn ScheduleLabel>)
where
    S: PredictionScheme,
{
    app.insert_non_send_resource(PredictionApp::new::<S>());
    app.insert_resource(PredictionInterval(Duration::from_millis(1000)));

    app.add_systems(
        schedule,
        (extract_predicted_app, run_prediction_app)
            .chain()
            .in_set(ClientPredictionSet::RunPredictionWorld),
    );
}

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct PredictionInterval(pub Duration);

pub(crate) struct PredictionApp {
    pub app: ParallelApp,
    /// whether the finished predicted world has been extracted into the main app.
    pub applied: bool,
}

impl PredictionApp {
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

        PredictionApp {
            app: ParallelApp::new(app),
            applied: true,
        }
    }
}

fn run_prediction_app(
    mut server_world_app_state: NonSendMut<ServerWorldApp>,
    mut prediction_app_state: NonSendMut<PredictionApp>,
    prediction_interval: Res<PredictionInterval>,
    mut scratch_world: Local<Option<World>>,
) -> Result {
    if !prediction_app_state.applied {
        return Ok(());
    }

    // Check that both the server world app and prediction app are idle.
    let Some(server_world_app) = server_world_app_state.poll()? else {
        return Ok(());
    };

    let Some(prediction_app) = prediction_app_state.app.poll()? else {
        return Ok(());
    };

    // Check if there are new ticks to predict.
    let server_time = **server_world_app.world().resource::<SimulationTimeTarget>();
    let mut current_target = prediction_app
        .world_mut()
        .resource_mut::<SimulationTimeTarget>();

    let target = server_time + **prediction_interval;
    if **current_target >= target {
        return Ok(());
    }

    **current_target = target;

    // Swap server world with scratch world.
    let mut server_world = scratch_world.take().unwrap_or_default();
    std::mem::swap(&mut server_world, server_world_app.world_mut());

    // Extract the simulation time from the source world.
    *prediction_app
        .world_mut()
        .resource_mut::<Time<SimulationTime>>() =
        (*server_world.resource::<Time<SimulationTime>>()).clone();

    // Insert server world and run extract schedule.
    prediction_app.insert_resource(SourceWorld(server_world));
    prediction_app.world_mut().run_schedule(ExtractSimulation);
    let SourceWorld(mut server_world) = prediction_app
        .world_mut()
        .remove_resource()
        .ok_or("Extract schedule removed the `SourceWorld`")?;

    // Swap server world back and replace scratch world.
    std::mem::swap(&mut server_world, server_world_app.world_mut());
    *scratch_world = Some(server_world);

    // Run prediction app.
    prediction_app_state.applied = false;
    prediction_app_state.app.update();

    Ok(())
}

fn extract_predicted_app(world: &mut World, mut scratch_world: Local<Option<World>>) -> Result {
    let mut prediction_app_state = world
        .remove_non_send_resource::<PredictionApp>()
        .ok_or("Prediction app should exist")?;

    if prediction_app_state.applied {
        world.insert_non_send_resource(prediction_app_state);
        return Ok(());
    }

    let Some(prediction_app) = prediction_app_state.app.poll()? else {
        world.insert_non_send_resource(prediction_app_state);
        return Ok(());
    };

    let mut prediction_world = scratch_world.take().unwrap_or_default();
    std::mem::swap(&mut prediction_world, prediction_app.world_mut());

    // Extract the simulation time from the source world.
    *world.resource_mut::<Time<SimulationTime>>() =
        (*prediction_world.resource::<Time<SimulationTime>>()).clone();

    // Insert server world and run extract schedule.
    world.insert_resource(SourceWorld(prediction_world));
    world.run_schedule(ExtractSimulation);
    let SourceWorld(mut prediction_world) = world
        .remove_resource()
        .ok_or("Extract schedule removed the `SourceWorld`")?;

    // Swap server world back and replace scratch world.
    std::mem::swap(&mut prediction_world, prediction_app.world_mut());
    *scratch_world = Some(prediction_world);

    prediction_app_state.applied = true;
    world.insert_non_send_resource(prediction_app_state);

    Ok(())
}
