use crate::{
    common::simulation::{SimulationInstance, SimulationTime},
    prelude::SimulationTimeExt,
};
use bevy::{
    ecs::schedule::{ExecutorKind, ScheduleLabel},
    prelude::*,
};

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationStartupMain;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationPreStartup;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationStartup;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationPostStartup;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationMain;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationPreUpdate;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationUpdate;

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimulationPostUpdate;

/// This schedule resets the simulation instance.
/// Add systems to this schedule that remove data belonging to the simulation, as well as initialize any new data.
///
/// This is different from [`SimulationStartup`] in that it may run multiple times over the lifetime of the world.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResetSimulation;

/// Schedule that extracts the simulation state from a [`SourceWorld`](crate::common::simulation::SourceWorld) into the current (local) world.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractSimulation;

pub fn build(app: &mut App) {
    let mut startup = Schedule::new(SimulationMain);
    startup.set_executor_kind(ExecutorKind::SingleThreaded);

    let mut main = Schedule::new(SimulationMain);
    main.set_executor_kind(ExecutorKind::SingleThreaded);

    app.add_schedule(startup);
    app.add_schedule(main);

    app.add_schedule(Schedule::new(SimulationPreUpdate));
    app.add_schedule(Schedule::new(SimulationUpdate));
    app.add_schedule(Schedule::new(SimulationPostUpdate));

    app.add_schedule(Schedule::new(SimulationPreStartup));
    app.add_schedule(Schedule::new(SimulationStartup));
    app.add_schedule(Schedule::new(SimulationPostStartup));

    // Single threaded as most systems will mutably access the source world.
    let mut extract = Schedule::new(ExtractSimulation);
    extract.set_executor_kind(ExecutorKind::SingleThreaded);

    app.add_schedule(extract);
    app.add_schedule(Schedule::new(ResetSimulation));

    app.add_systems(SimulationMain, run_simulation_main);
    app.add_systems(SimulationStartupMain, run_simulation_startup_main);
}

fn run_simulation_main(world: &mut World) {
    let simulation_instance = world.resource::<SimulationInstance>().format_tracing_str();
    let simulation_tick = world.resource::<Time<SimulationTime>>().current_tick().0;

    info_span!("SimulationPreUpdate", simulation_instance, simulation_tick).in_scope(|| {
        world.run_schedule(SimulationPreUpdate);
    });

    info_span!("SimulationUpdate", simulation_instance, simulation_tick).in_scope(|| {
        world.run_schedule(SimulationUpdate);
    });

    info_span!("SimulationPostUpdate", simulation_instance, simulation_tick).in_scope(|| {
        world.run_schedule(SimulationPostUpdate);
    });
}

fn run_simulation_startup_main(world: &mut World) {
    let simulation_instance = world.resource::<SimulationInstance>().format_tracing_str();

    info_span!("SimulationPreStartup", simulation_instance).in_scope(|| {
        world.run_schedule(SimulationPreStartup);
    });

    info_span!("SimulationStartup", simulation_instance).in_scope(|| {
        world.run_schedule(SimulationStartup);
    });

    info_span!("SimulationPostStartup", simulation_instance).in_scope(|| {
        world.run_schedule(SimulationPostStartup);
    });
}
