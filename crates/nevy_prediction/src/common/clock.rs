//! A linear model of simulation time against real time.
//!
//! Clocks are computed on the app thread and shared with the simulation threads,
//! so that simulation threads can wake up exactly when a tick is due regardless of the app's frame rate.

use std::time::Duration;

use bevy::platform::time::Instant;

use crate::common::simulation::SimulationTick;

/// Maps real time to simulation time.
///
/// Simulation time advances at `scale` simulation seconds per real second from an anchor point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SimulationClock {
    anchor: Instant,
    /// Simulation seconds at the anchor.
    anchor_time: f64,
    scale: f64,
}

impl SimulationClock {
    pub fn new(anchor: Instant, time: Duration, scale: f32) -> Self {
        SimulationClock {
            anchor,
            anchor_time: time.as_secs_f64(),
            scale: scale.max(0.) as f64,
        }
    }

    pub fn scale(&self) -> f32 {
        self.scale as f32
    }

    fn seconds_at(&self, instant: Instant) -> f64 {
        let elapsed = if instant >= self.anchor {
            (instant - self.anchor).as_secs_f64()
        } else {
            -(self.anchor - instant).as_secs_f64()
        };

        self.anchor_time + elapsed * self.scale
    }

    /// The simulation time at a real instant.
    pub fn time_at(&self, instant: Instant) -> Duration {
        Duration::from_secs_f64(self.seconds_at(instant).max(0.))
    }

    /// The real instant that the clock reaches a simulation time.
    ///
    /// Returns `None` if the clock is stopped and hasn't reached it.
    pub fn instant_at(&self, time: Duration) -> Option<Instant> {
        let remaining = time.as_secs_f64() - self.anchor_time;

        if remaining <= 0. {
            return Some(self.anchor);
        }

        if self.scale <= 0. {
            return None;
        }

        Some(self.anchor + Duration::from_secs_f64(remaining / self.scale))
    }

    /// Changes the scale at `now` without changing the current time.
    pub fn rescale(&mut self, now: Instant, scale: f32) {
        *self = SimulationClock::new(now, self.time_at(now), scale);
    }

    /// The next tick to execute at a real instant.
    ///
    /// Every tick before this one should have been executed.
    /// A tick is executed once simulation time passes its end, see [`Self::tick_deadline`].
    pub fn tick_at(&self, instant: Instant, step: Duration) -> SimulationTick {
        let ticks = self.seconds_at(instant) / step.as_secs_f64();

        SimulationTick(ticks.max(0.).floor() as u32)
    }

    /// The real instant that a tick should be executed.
    pub fn tick_deadline(&self, tick: SimulationTick, step: Duration) -> Option<Instant> {
        self.instant_at(step * (*tick + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: Duration = Duration::from_millis(50);

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn ticks_are_due_at_their_end() {
        let start = Instant::now();
        let clock = SimulationClock::new(start, ms(0), 1.);

        assert_eq!(clock.tick_at(start, STEP), SimulationTick(0));
        assert_eq!(clock.tick_at(start + ms(49), STEP), SimulationTick(0));
        assert_eq!(clock.tick_at(start + ms(50), STEP), SimulationTick(1));

        let deadline = clock.tick_deadline(SimulationTick(0), STEP).unwrap();
        assert!((deadline - start).abs_diff(ms(50)) < Duration::from_micros(1));
    }

    #[test]
    fn scale_slows_time() {
        let start = Instant::now();
        let mut clock = SimulationClock::new(start, ms(1000), 0.5);

        assert!(clock.time_at(start + ms(100)).abs_diff(ms(1050)) < Duration::from_micros(1));

        // rescaling keeps the current time
        clock.rescale(start + ms(100), 2.);
        assert!(clock.time_at(start + ms(100)).abs_diff(ms(1050)) < Duration::from_micros(1));
        assert!(clock.time_at(start + ms(200)).abs_diff(ms(1250)) < Duration::from_micros(1));

        let deadline = clock.instant_at(ms(1450)).unwrap();
        assert!((deadline - start).abs_diff(ms(300)) < Duration::from_micros(1));
    }

    #[test]
    fn stopped_clock_never_reaches_the_future() {
        let start = Instant::now();
        let clock = SimulationClock::new(start, ms(100), 0.);

        assert!(clock.instant_at(ms(200)).is_none());
        assert_eq!(clock.instant_at(ms(50)), Some(start));
    }
}
