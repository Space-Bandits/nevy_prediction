//! Estimates the server's simulation time from the [`UpdateServerTick`] messages it sends.

use std::{collections::VecDeque, time::Duration};

use bevy::{platform::time::Instant, prelude::*};
use nevy::prelude::*;
use tracing::warn;

use crate::{
    client::PredictionServerConnection,
    common::{UpdateServerTick, simulation::SimulationTick},
};

/// Estimates the server's simulation time.
///
/// The server's clock can run slower than real time when the simulation is slowed down.
/// To account for this the estimate is made on a local clock that runs at the server's latest time scale,
/// so that the time between receiving a sample and now is scaled the same way that the server's clock was.
#[derive(Resource)]
pub(crate) struct ServerTickSamples {
    latest: SimulationTick,
    time_scale: f32,
    /// A local clock that advances at `time_scale`, in seconds.
    scaled_clock: f64,
    last_update: Option<Instant>,
    /// The server's clock minus the scaled clock when each sample was received, in seconds.
    offsets: VecDeque<f64>,
}

impl Default for ServerTickSamples {
    fn default() -> Self {
        ServerTickSamples {
            latest: SimulationTick(0),
            time_scale: 1.,
            scaled_clock: 0.,
            last_update: None,
            offsets: VecDeque::new(),
        }
    }
}

impl ServerTickSamples {
    const SERVER_TIME_ESTIMATE_SAMPLES: usize = 32;

    /// Advances the scaled clock to `now`.
    fn advance(&mut self, now: Instant) {
        if let Some(last_update) = self.last_update
            && now > last_update
        {
            self.scaled_clock += (now - last_update).as_secs_f64() * self.time_scale as f64;
        }

        self.last_update = Some(now);
    }

    pub fn push(&mut self, now: Instant, tick: SimulationTick, time: Duration, time_scale: f32) {
        self.advance(now);

        self.latest = self.latest.max(tick);
        self.time_scale = time_scale;

        self.offsets
            .push_back(time.as_secs_f64() - self.scaled_clock);

        while self.offsets.len() > Self::SERVER_TIME_ESTIMATE_SAMPLES {
            self.offsets.pop_front();
        }
    }

    pub fn reset(&mut self, now: Instant, tick: SimulationTick, time: Duration) {
        let time_scale = self.time_scale;

        *self = default();

        self.push(now, tick, time, time_scale);
    }

    /// The latest tick that the server has executed up to.
    pub fn latest(&self) -> SimulationTick {
        self.latest
    }

    /// The server's latest time scale.
    pub fn time_scale(&self) -> f32 {
        self.time_scale
    }

    /// The estimated simulation time of the server at `now`.
    ///
    /// This should be called with increasing values of `now`.
    pub fn estimated_time(&mut self, now: Instant) -> Duration {
        self.advance(now);

        if self.offsets.is_empty() {
            return Duration::ZERO;
        }

        let offset = self.offsets.iter().sum::<f64>() / self.offsets.len() as f64;

        Duration::from_secs_f64((self.scaled_clock + offset).max(0.))
    }
}

/// Responsible for receiving [`UpdateServerTick`]s.
pub(crate) fn receive_time_updates(
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<UpdateServerTick>,
        Has<PredictionServerConnection>,
    )>,
    mut tick_samples: ResMut<ServerTickSamples>,
) {
    let now = Instant::now();

    for (connection_entity, mut messages, is_server) in &mut message_q {
        for UpdateServerTick {
            simulation_tick,
            time,
            time_scale,
        } in messages.drain()
        {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            tick_samples.push(now, simulation_tick, time, time_scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn estimate_follows_time_scale() {
        let start = Instant::now();
        let mut samples = ServerTickSamples::default();

        samples.push(start, SimulationTick(20), ms(1000), 0.5);

        // half a second of simulation time per real second
        let estimate = samples.estimated_time(start + ms(1000));
        assert!(estimate.abs_diff(ms(1500)) < ms(1), "{estimate:?}");

        // another sample at a different scale
        samples.push(start + ms(1000), SimulationTick(30), ms(1500), 1.);
        let estimate = samples.estimated_time(start + ms(2000));
        assert!(estimate.abs_diff(ms(2500)) < ms(1), "{estimate:?}");

        assert_eq!(samples.latest(), SimulationTick(30));
    }
}
