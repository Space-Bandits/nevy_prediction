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
        run_prediction_app.in_set(ClientPredictionSet::RunPredictionWorld),
    );
}

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct PredictionInterval(pub Duration);

#[derive(Deref, DerefMut)]
pub(crate) struct PredictionApp(pub ParallelApp);

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

        PredictionApp(ParallelApp::new(app))
    }
}

fn run_prediction_app(
    mut server_world_app_state: NonSendMut<ServerWorldApp>,
    mut prediction_app_state: NonSendMut<PredictionApp>,
    prediction_interval: Res<PredictionInterval>,
    mut scratch_world: Local<Option<World>>,
) -> Result {
    // Check that both the server world app and prediction app are idle.
    let Some(server_world_app) = server_world_app_state.poll()? else {
        return Ok(());
    };

    let Some(prediction_app) = prediction_app_state.poll()? else {
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
        .remove_resource::<SourceWorld>()
        .ok_or("Extract schedule removed the source world")?;

    // Swap server world back and replace scratch world.
    std::mem::swap(&mut server_world, server_world_app.world_mut());
    *scratch_world = Some(server_world);

    // Run prediction app.
    prediction_app_state.update();

    Ok(())
}
