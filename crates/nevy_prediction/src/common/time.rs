//! Contains [`PredictionTime`], the clock that the app should use for anything that follows the simulation.

use std::time::Duration;

use bevy::prelude::*;

use crate::common::simulation::SimulationTick;

/// The context of the `Time<PredictionTime>` resource, which is inserted into apps that run a simulation.
///
/// The simulation is executed on a fixed timestep, but how fast simulation time passes compared to real time can change.
/// If the server or any client can't execute the simulation fast enough, the server slows the simulation down for everyone.
/// Use this clock for anything outside of the simulation that should slow down with it,
/// such as animations, effects and interpolating between simulation ticks.
///
/// - [`Time::elapsed`] is the simulation time that the app is presenting.
///   On the client this trails the latest simulation state that was extracted into the app by up to one tick,
///   so that it can be interpolated between the latest two ticks.
///   On the server this is the time of the latest simulation state that was extracted into the app.
/// - [`Time::delta`] is how much simulation time passed since the last frame.
///
/// See [`PredictionTimeExt`] for more information.
#[derive(Clone, Copy, Debug)]
pub struct PredictionTime {
    pub(crate) time_scale: f32,
    pub(crate) latest_tick: SimulationTick,
    pub(crate) fraction: f32,
}

impl Default for PredictionTime {
    fn default() -> Self {
        PredictionTime {
            time_scale: 1.,
            latest_tick: SimulationTick(0),
            fraction: 1.,
        }
    }
}

pub trait PredictionTimeExt {
    /// How many simulation seconds pass per real second.
    ///
    /// This is 1 unless the simulation is being slowed down because a machine can't keep up.
    fn time_scale(&self) -> f32;

    /// The [`SimulationTime`](crate::common::simulation::SimulationTime) tick of the latest simulation state in the app.
    ///
    /// The state represents the simulation at the start of this tick,
    /// so its timestamp is [`SimulationTick::time`].
    fn latest_tick(&self) -> SimulationTick;

    /// How far [`Time::elapsed`] is between the tick before [`Self::latest_tick`] and [`Self::latest_tick`], from 0 to 1.
    ///
    /// Use this to interpolate between the previous and latest simulation states.
    fn fraction(&self) -> f32;
}

impl PredictionTimeExt for Time<PredictionTime> {
    fn time_scale(&self) -> f32 {
        self.context().time_scale
    }

    fn latest_tick(&self) -> SimulationTick {
        self.context().latest_tick
    }

    fn fraction(&self) -> f32 {
        self.context().fraction
    }
}

pub(crate) trait PrivatePredictionTimeExt {
    fn from_time(time: Duration, tick: SimulationTick) -> Self;
}

impl PrivatePredictionTimeExt for Time<PredictionTime> {
    fn from_time(time: Duration, tick: SimulationTick) -> Self {
        let mut prediction_time = Time::new_with(PredictionTime {
            latest_tick: tick,
            ..default()
        });
        prediction_time.advance_to(time);
        prediction_time
    }
}
