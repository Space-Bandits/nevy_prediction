//! Adapts the [`PredictionInterval`] to network conditions.
//!
//! The client periodically sends a [`TickProbe`] stamped with its current tick, the same way world updates are stamped.
//! The server responds with how early it arrived, which accounts for latency, jitter and processing delays in one measurement.
//! The interval is then moved towards whatever would have made almost all recent probes arrive on time, plus a margin.

use std::{collections::VecDeque, time::Duration};

use bevy::{
    diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic},
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
};
use log::warn;
use nevy::prelude::*;

use crate::{
    client::{
        ClientPredictionSchedule, ClientSimulationSystems, PredictionInterval,
        PredictionServerConnection,
    },
    common::{
        TickProbe, TickProbeResult,
        simulation::{SimulationTick, SimulationTime, SimulationTimeExt},
    },
};

/// How long to wait for a probe result before considering it lost.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn build(app: &mut App, schedule: Interned<dyn ScheduleLabel>) {
    app.init_resource::<PredictionIntervalSettings>();
    app.init_resource::<PredictionIntervalStats>();
    app.init_resource::<IntervalController>();

    app.add_shared_message_sender::<TickProbeStream>(StreamRequirements::RELIABLE_ORDERED);

    app.add_systems(
        schedule,
        send_tick_probes.in_set(ClientSimulationSystems::QueueUpdates),
    );
}

/// Publishes [`PredictionInterval`] and [`PredictionIntervalStats`] as bevy [`Diagnostic`]s.
///
/// Must be added after [`NevyPredictionClientPlugin`](crate::client::NevyPredictionClientPlugin).
pub struct PredictionDiagnosticsPlugin;

impl PredictionDiagnosticsPlugin {
    /// The current [`PredictionInterval`] in milliseconds.
    pub const INTERVAL: DiagnosticPath = DiagnosticPath::const_new("nevy_prediction/interval");
    /// [`PredictionIntervalStats::target`] in milliseconds.
    pub const TARGET: DiagnosticPath = DiagnosticPath::const_new("nevy_prediction/target");
    /// [`PredictionIntervalStats::required`] in milliseconds.
    pub const REQUIRED: DiagnosticPath = DiagnosticPath::const_new("nevy_prediction/required");
    /// [`PredictionIntervalStats::rtt`] in milliseconds.
    pub const RTT: DiagnosticPath = DiagnosticPath::const_new("nevy_prediction/rtt");
    /// [`PredictionIntervalStats::late_probes`].
    pub const LATE_PROBES: DiagnosticPath =
        DiagnosticPath::const_new("nevy_prediction/late_probes");
    /// [`PredictionIntervalStats::lost_probes`].
    pub const LOST_PROBES: DiagnosticPath =
        DiagnosticPath::const_new("nevy_prediction/lost_probes");
}

impl Plugin for PredictionDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        for path in [Self::INTERVAL, Self::TARGET, Self::REQUIRED, Self::RTT] {
            app.register_diagnostic(Diagnostic::new(path).with_suffix("ms"));
        }

        for path in [Self::LATE_PROBES, Self::LOST_PROBES] {
            app.register_diagnostic(Diagnostic::new(path));
        }

        let schedule = **app.world().resource::<ClientPredictionSchedule>();
        app.add_systems(
            schedule,
            measure_diagnostics.after(ClientSimulationSystems::ReceiveUpdates),
        );
    }
}

fn measure_diagnostics(
    mut diagnostics: Diagnostics,
    interval: Res<PredictionInterval>,
    stats: Res<PredictionIntervalStats>,
) {
    let millis = |duration: Duration| duration.as_secs_f64() * 1000.;

    diagnostics.add_measurement(&PredictionDiagnosticsPlugin::INTERVAL, || {
        millis(**interval)
    });

    for (path, value) in [
        (&PredictionDiagnosticsPlugin::TARGET, stats.target),
        (&PredictionDiagnosticsPlugin::REQUIRED, stats.required),
        (&PredictionDiagnosticsPlugin::RTT, stats.rtt),
    ] {
        if let Some(value) = value {
            diagnostics.add_measurement(path, || millis(value));
        }
    }

    diagnostics.add_measurement(&PredictionDiagnosticsPlugin::LATE_PROBES, || {
        stats.late_probes as f64
    });
    diagnostics.add_measurement(&PredictionDiagnosticsPlugin::LOST_PROBES, || {
        stats.lost_probes as f64
    });
}

/// Marker type for the [`SharedMessageSender`] used to send [`TickProbe`]s.
struct TickProbeStream;

/// Controls how the [`PredictionInterval`] adapts to network conditions.
#[derive(Resource, Clone, Debug)]
pub struct PredictionIntervalSettings {
    /// If false the [`PredictionInterval`] is left alone and no probes are sent.
    pub adaptive: bool,
    /// Extra time added on top of the measured requirement.
    pub margin: Duration,
    /// The fraction of recent probes (0..1) that the interval should be large enough for.
    /// Higher values handle jitter better at the cost of a larger interval.
    pub percentile: f32,
    /// How long probe results are considered for.
    pub window: Duration,
    /// Minimum prediction window length
    pub min: Duration,
    /// Maximum prediction window length
    pub max: Duration,
    /// Seconds the interval may grow by per second.
    /// This is how much faster than real time the simulation may run while catching up.
    pub increase_rate: f32,
    /// Seconds the interval may shrink by per second.
    /// This is how much slower than real time the simulation may run while shrinking.
    pub decrease_rate: f32,
    /// The interval won't shrink unless it is at least this much larger than needed, to avoid oscillating.
    pub deadband: Duration,
    /// Number of ticks between probes.
    pub probe_interval: u32,
}

impl Default for PredictionIntervalSettings {
    fn default() -> Self {
        PredictionIntervalSettings {
            adaptive: true,
            margin: Duration::from_millis(50),
            percentile: 0.98,
            window: Duration::from_secs(3),
            min: Duration::ZERO,
            max: Duration::from_secs(1),
            increase_rate: 0.1,
            decrease_rate: 0.1,
            deadband: Duration::from_millis(20),
            probe_interval: 1,
        }
    }
}

/// Measurements used to adapt the [`PredictionInterval`], useful for debugging.
#[derive(Resource, Clone, Debug, Default)]
pub struct PredictionIntervalStats {
    /// Round trip time of the most recent probe.
    pub rtt: Option<Duration>,
    /// The interval that would have been just enough for [`PredictionIntervalSettings::percentile`] of recent probes.
    pub required: Option<Duration>,
    /// The interval being moved towards.
    pub target: Option<Duration>,
    /// Number of probes that arrived after the server had executed their tick.
    pub late_probes: u64,
    /// Number of probes that never got a result.
    pub lost_probes: u64,
}

/// State used to adapt the [`PredictionInterval`].
#[derive(Resource, Default)]
pub(crate) struct IntervalController {
    /// True once the simulation has been reset by the server and probes can be sent.
    active: bool,
    /// True if the interval should jump to the target instead of moving towards it.
    snap: bool,
    last_probe: Option<SimulationTick>,
    /// Probes that are waiting for a result, in the order they were sent.
    pending: VecDeque<PendingProbe>,
    /// The intervals that would have made recent probes arrive exactly on time, in seconds.
    samples: VecDeque<(Duration, f32)>,
}

struct PendingProbe {
    tick: SimulationTick,
    interval: Duration,
    sent: Duration,
}

impl IntervalController {
    /// Called when the server resets the simulation.
    pub(crate) fn reset(&mut self) {
        *self = IntervalController {
            active: true,
            snap: true,
            ..default()
        };
    }

    /// Returns true if a probe should be sent for this tick.
    fn should_probe(&self, tick: SimulationTick, probe_interval: u32) -> bool {
        self.active
            && self
                .last_probe
                .is_none_or(|last| *tick >= *last + probe_interval.max(1))
    }

    fn sent(&mut self, tick: SimulationTick, interval: Duration, now: Duration) {
        self.last_probe = Some(tick);
        self.pending.push_back(PendingProbe {
            tick,
            interval,
            sent: now,
        });
    }

    /// Records a probe result, returning its round trip time.
    fn received(&mut self, tick: SimulationTick, lead: f32, now: Duration) -> Option<Duration> {
        let index = self.pending.iter().position(|probe| probe.tick == tick)?;
        let probe = self.pending.remove(index)?;

        // the interval that would have given this probe zero lead
        let required = probe.interval.as_secs_f32() - lead;
        self.samples.push_back((now, required));

        Some(now.saturating_sub(probe.sent))
    }

    /// Removes old samples and probes that timed out, returning the number of lost probes.
    fn expire(&mut self, now: Duration, window: Duration) -> u64 {
        while self
            .samples
            .front()
            .is_some_and(|&(received, _)| now.saturating_sub(received) > window)
        {
            self.samples.pop_front();
        }

        let mut lost = 0;
        while self
            .pending
            .front()
            .is_some_and(|probe| now.saturating_sub(probe.sent) > PROBE_TIMEOUT)
        {
            self.pending.pop_front();
            lost += 1;
        }

        lost
    }

    /// The interval that would have been enough for the configured percentile of recent probes.
    fn required(&self, percentile: f32) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        let mut samples: Vec<f32> = self.samples.iter().map(|&(_, required)| required).collect();
        samples.sort_by(f32::total_cmp);

        let index = ((samples.len() - 1) as f32 * percentile.clamp(0., 1.)).ceil() as usize;

        Some(Duration::from_secs_f32(samples[index].max(0.)))
    }
}

impl PredictionIntervalSettings {
    fn target(&self, required: Duration) -> Duration {
        (required + self.margin).clamp(self.min, self.max.max(self.min))
    }

    /// Moves `current` towards `target` limited by the configured rates.
    fn approach(&self, current: Duration, target: Duration, delta: Duration) -> Duration {
        if target > current {
            let step = delta.mul_f32(self.increase_rate.max(0.));
            (current + step).min(target)
        } else if current - target > self.deadband {
            let step = delta.mul_f32(self.decrease_rate.max(0.));
            current.saturating_sub(step).max(target)
        } else {
            current
        }
    }
}

fn send_tick_probes(
    settings: Res<PredictionIntervalSettings>,
    mut controller: ResMut<IntervalController>,
    time: Res<Time<SimulationTime>>,
    interval: Res<PredictionInterval>,
    real_time: Res<Time<Real>>,
    server_q: Query<Entity, With<PredictionServerConnection>>,
    mut messages: SharedMessageSender<TickProbeStream>,
) -> Result {
    let tick = time.current_tick();

    if !settings.adaptive || !controller.should_probe(tick, settings.probe_interval) {
        return Ok(());
    }

    let Ok(server_entity) = server_q.single() else {
        return Ok(());
    };

    messages.write(
        server_entity,
        true,
        &TickProbe {
            simulation_tick: tick,
        },
    )?;

    controller.sent(tick, **interval, real_time.elapsed());

    Ok(())
}

/// Receives probe results and moves the [`PredictionInterval`] towards what is needed.
pub(crate) fn update_prediction_interval(
    settings: Res<PredictionIntervalSettings>,
    mut controller: ResMut<IntervalController>,
    mut stats: ResMut<PredictionIntervalStats>,
    mut interval: ResMut<PredictionInterval>,
    real_time: Res<Time<Real>>,
    mut message_q: Query<(
        Entity,
        &mut ReceivedMessages<TickProbeResult>,
        Has<PredictionServerConnection>,
    )>,
) {
    let now = real_time.elapsed();

    for (connection_entity, mut messages, is_server) in &mut message_q {
        for TickProbeResult {
            simulation_tick,
            lead,
        } in messages.drain()
        {
            if !is_server {
                warn!(
                    "Received a prediction message from a connection that isn't the server: {}",
                    connection_entity
                );

                continue;
            }

            if let Some(rtt) = controller.received(simulation_tick, lead, now) {
                stats.rtt = Some(rtt);

                if lead < 0. {
                    stats.late_probes += 1;
                }
            }
        }
    }

    stats.lost_probes += controller.expire(now, settings.window);

    if !settings.adaptive {
        return;
    }

    let Some(required) = controller.required(settings.percentile) else {
        return;
    };
    let target = settings.target(required);

    stats.required = Some(required);
    stats.target = Some(target);

    let new_interval = if controller.snap {
        controller.snap = false;
        target
    } else {
        settings.approach(**interval, target, real_time.delta())
    };

    if **interval != new_interval {
        **interval = new_interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn assert_close(actual: Duration, expected: Duration) {
        assert!(
            actual.abs_diff(expected) < Duration::from_micros(1),
            "{actual:?} != {expected:?}"
        );
    }

    /// Simulates a probe sent at `now` with `interval` that arrives with `lead`.
    fn probe(
        controller: &mut IntervalController,
        tick: u32,
        interval: Duration,
        lead: f32,
        now: Duration,
    ) {
        controller.sent(SimulationTick(tick), interval, now);
        controller
            .received(SimulationTick(tick), lead, now)
            .unwrap();
    }

    #[test]
    fn required_accounts_for_interval_at_send() {
        let mut controller = IntervalController::default();
        controller.reset();

        // sent with 100ms and arrived 30ms early, so 70ms would have been just enough
        probe(&mut controller, 0, ms(100), 0.03, ms(0));
        let required = controller.required(0.95).unwrap();
        assert!((required.as_secs_f32() - 0.07).abs() < 1e-4);

        // arriving late needs more
        probe(&mut controller, 1, ms(100), -0.05, ms(0));
        let required = controller.required(1.).unwrap();
        assert!((required.as_secs_f32() - 0.15).abs() < 1e-4);
    }

    #[test]
    fn percentile_ignores_outlier() {
        let mut controller = IntervalController::default();
        controller.reset();

        for tick in 0..99 {
            probe(&mut controller, tick, ms(100), 0.05, ms(0));
        }
        probe(&mut controller, 99, ms(100), -0.5, ms(0));

        let required = controller.required(0.95).unwrap();
        assert!((required.as_secs_f32() - 0.05).abs() < 1e-4);

        let required = controller.required(1.).unwrap();
        assert!((required.as_secs_f32() - 0.6).abs() < 1e-4);
    }

    #[test]
    fn samples_and_probes_expire() {
        let mut controller = IntervalController::default();
        controller.reset();

        probe(&mut controller, 0, ms(100), 0.05, ms(0));
        controller.sent(SimulationTick(1), ms(100), ms(0));

        assert_eq!(controller.expire(ms(1000), ms(3000)), 0);
        assert!(controller.required(0.95).is_some());

        assert_eq!(controller.expire(ms(4000), ms(3000)), 1);
        assert!(controller.required(0.95).is_none());

        // a result for an expired probe is ignored
        assert!(
            controller
                .received(SimulationTick(1), 0., ms(4000))
                .is_none()
        );
    }

    #[test]
    fn probes_respect_interval() {
        let mut controller = IntervalController::default();
        assert!(
            !controller.should_probe(SimulationTick(0), 2),
            "inactive before reset"
        );

        controller.reset();
        assert!(controller.should_probe(SimulationTick(0), 2));
        controller.sent(SimulationTick(0), ms(100), ms(0));
        assert!(!controller.should_probe(SimulationTick(1), 2));
        assert!(controller.should_probe(SimulationTick(2), 2));
    }

    #[test]
    fn target_is_clamped() {
        let settings = PredictionIntervalSettings {
            margin: ms(10),
            min: ms(20),
            max: ms(200),
            ..default()
        };

        assert_eq!(settings.target(ms(0)), ms(20));
        assert_eq!(settings.target(ms(100)), ms(110));
        assert_eq!(settings.target(ms(500)), ms(200));
    }

    #[test]
    fn increases_faster_than_decreases() {
        let settings = PredictionIntervalSettings {
            increase_rate: 0.1,
            decrease_rate: 0.02,
            deadband: ms(5),
            ..default()
        };

        // 1 second at 10% grows 100ms
        assert_close(settings.approach(ms(100), ms(500), ms(1000)), ms(200));
        // doesn't overshoot
        assert_close(settings.approach(ms(100), ms(150), ms(1000)), ms(150));

        // 1 second at 2% shrinks 20ms
        assert_close(settings.approach(ms(500), ms(100), ms(1000)), ms(480));
        assert_close(settings.approach(ms(110), ms(100), ms(1000)), ms(100));

        // within the deadband nothing changes
        assert_close(settings.approach(ms(104), ms(100), ms(1000)), ms(104));
    }

    #[test]
    fn converges_to_new_conditions() {
        let settings = PredictionIntervalSettings::default();
        let mut controller = IntervalController::default();
        controller.reset();

        // network needs 150ms, starting from 100ms
        let needed = 0.15;
        let mut interval = ms(100);
        let frame = ms(16);
        let mut now = Duration::ZERO;

        for tick in 0..300 {
            probe(
                &mut controller,
                tick,
                interval,
                interval.as_secs_f32() - needed,
                now,
            );
            controller.expire(now, settings.window);

            let target = settings.target(controller.required(settings.percentile).unwrap());
            interval = settings.approach(interval, target, frame);
            now += frame;
        }

        let expected = 0.15 + settings.margin.as_secs_f32();
        assert!(
            (interval.as_secs_f32() - expected).abs() < 0.001,
            "{interval:?}"
        );
    }
}
