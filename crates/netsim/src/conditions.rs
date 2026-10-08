use std::time::{Duration, Instant};

/// Packets that would wait longer than this behind a bandwidth limit are dropped (tail drop).
const MAX_QUEUE_DELAY: Duration = Duration::from_secs(1);

/// Network conditions applied to a single direction of traffic.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkConditions {
    /// One way base delay in milliseconds.
    pub latency: f64,
    /// Standard deviation of a normally distributed delay added to `latency`, in milliseconds.
    pub jitter: f64,
    /// If false, jitter will never cause packets to be delivered out of order.
    pub reorder: bool,
    /// Probability that any packet is dropped.
    pub loss: f64,
    /// Per packet probability of entering a burst loss state. Zero disables burst loss.
    pub burst_enter: f64,
    /// Per packet probability of leaving the burst loss state. All packets are dropped while in a burst.
    pub burst_exit: f64,
    /// Probability that a packet is delivered twice.
    pub duplicate: f64,
    /// Bandwidth limit in kilobits per second. Zero is unlimited.
    pub bandwidth: f64,
}

impl Default for LinkConditions {
    fn default() -> Self {
        LinkConditions {
            latency: 0.,
            jitter: 0.,
            reorder: false,
            loss: 0.,
            burst_enter: 0.,
            burst_exit: 0.5,
            duplicate: 0.,
            bandwidth: 0.,
        }
    }
}

impl LinkConditions {
    pub const FIELDS: &[&str] = &[
        "latency",
        "jitter",
        "reorder",
        "loss",
        "burst_enter",
        "burst_exit",
        "duplicate",
        "bandwidth",
    ];

    pub fn set(&mut self, field: &str, value: &str) -> Result<(), String> {
        fn number(value: &str, min: f64, max: f64) -> Result<f64, String> {
            let parsed: f64 = value
                .parse()
                .map_err(|_| format!("'{value}' is not a number"))?;

            if !(min..=max).contains(&parsed) {
                return Err(format!("{value} is outside of the range {min}..={max}"));
            }

            Ok(parsed)
        }

        match field {
            "latency" => self.latency = number(value, 0., f64::MAX)?,
            "jitter" => self.jitter = number(value, 0., f64::MAX)?,
            "reorder" => {
                self.reorder = match value {
                    "true" | "on" | "1" => true,
                    "false" | "off" | "0" => false,
                    _ => return Err(format!("'{value}' is not a boolean")),
                }
            }
            "loss" => self.loss = number(value, 0., 1.)?,
            "burst_enter" => self.burst_enter = number(value, 0., 1.)?,
            "burst_exit" => self.burst_exit = number(value, 0., 1.)?,
            "duplicate" => self.duplicate = number(value, 0., 1.)?,
            "bandwidth" => self.bandwidth = number(value, 0., f64::MAX)?,
            _ => {
                return Err(format!(
                    "unknown field '{field}', expected one of {}",
                    Self::FIELDS.join(", ")
                ));
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for LinkConditions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "latency {}ms, jitter {}ms, reorder {}, loss {}, burst_enter {}, burst_exit {}, duplicate {}, bandwidth {}",
            self.latency,
            self.jitter,
            self.reorder,
            self.loss,
            self.burst_enter,
            self.burst_exit,
            self.duplicate,
            if self.bandwidth > 0. {
                format!("{}kbps", self.bandwidth)
            } else {
                "unlimited".to_string()
            }
        )
    }
}

/// Conditions for both directions of traffic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conditions {
    /// Client -> server.
    pub up: LinkConditions,
    /// Server -> client.
    pub down: LinkConditions,
    /// While paused all packets are dropped.
    pub paused: bool,
}

impl std::fmt::Display for Conditions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "up:   {}
down: {}",
            self.up, self.down
        )?;
        if self.paused {
            write!(
                f,
                "
(paused)"
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl Conditions {
    pub fn link(&self, direction: Direction) -> &LinkConditions {
        match direction {
            Direction::Up => &self.up,
            Direction::Down => &self.down,
        }
    }
}

/// Small deterministic rng (SplitMix64), good enough for simulating networks.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn chance(&mut self, probability: f64) -> bool {
        probability > 0. && self.next_f64() < probability
    }

    /// Standard normal sample using the Box-Muller transform.
    pub fn normal(&mut self) -> f64 {
        let u1 = 1. - self.next_f64(); // (0, 1] so ln is finite
        let u2 = self.next_f64();
        (-2. * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// State of a single direction of traffic that decides the fate of each packet.
pub struct LinkState {
    rng: Rng,
    in_burst: bool,
    last_release: Option<Instant>,
    link_free: Option<Instant>,
}

impl LinkState {
    pub fn new(seed: u64) -> Self {
        LinkState {
            rng: Rng::new(seed),
            in_burst: false,
            last_release: None,
            link_free: None,
        }
    }

    /// Decides when a packet that arrived at `received` should be released.
    ///
    /// Returns no release times if the packet is dropped, and two if it is duplicated.
    pub fn plan(
        &mut self,
        conditions: &LinkConditions,
        received: Instant,
        len: usize,
    ) -> Vec<Instant> {
        if self.dropped(conditions) {
            return Vec::new();
        }

        let mut departure = received;

        if conditions.bandwidth > 0. {
            let start = self.link_free.map_or(received, |free| free.max(received));

            if start - received > MAX_QUEUE_DELAY {
                return Vec::new();
            }

            let transmit =
                Duration::from_secs_f64(len as f64 * 8. / (conditions.bandwidth * 1000.));
            departure = start + transmit;
            self.link_free = Some(departure);
        }

        let mut releases = vec![self.release(conditions, departure)];

        if self.rng.chance(conditions.duplicate) {
            releases.push(self.release(conditions, departure));
        }

        releases
    }

    fn dropped(&mut self, conditions: &LinkConditions) -> bool {
        if conditions.burst_enter > 0. {
            if self.in_burst {
                if self.rng.chance(conditions.burst_exit) {
                    self.in_burst = false;
                }
            } else if self.rng.chance(conditions.burst_enter) {
                self.in_burst = true;
            }
        } else {
            self.in_burst = false;
        }

        self.in_burst || self.rng.chance(conditions.loss)
    }

    fn release(&mut self, conditions: &LinkConditions, departure: Instant) -> Instant {
        let mut delay = conditions.latency;
        if conditions.jitter > 0. {
            delay += conditions.jitter * self.rng.normal();
        }

        let mut release = departure + Duration::from_secs_f64(delay.max(0.) / 1000.);

        if let Some(last_release) = self.last_release {
            if !conditions.reorder {
                release = release.max(last_release);
            }

            self.last_release = Some(last_release.max(release));
        } else {
            self.last_release = Some(release);
        }

        release
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1000.
    }

    #[test]
    fn zero_conditions_pass_immediately() {
        let mut link = LinkState::new(1);
        let now = Instant::now();

        for _ in 0..100 {
            assert_eq!(link.plan(&LinkConditions::default(), now, 1200), vec![now]);
        }
    }

    #[test]
    fn latency_without_jitter_is_exact() {
        let mut link = LinkState::new(1);
        let conditions = LinkConditions {
            latency: 50.,
            ..Default::default()
        };
        let now = Instant::now();

        let releases = link.plan(&conditions, now, 100);
        assert_eq!(releases.len(), 1);
        assert!((ms(releases[0] - now) - 50.).abs() < 0.001);
    }

    #[test]
    fn loss_rate_matches() {
        let mut link = LinkState::new(42);
        let conditions = LinkConditions {
            loss: 0.1,
            ..Default::default()
        };
        let now = Instant::now();

        let passed = (0..10_000)
            .filter(|_| !link.plan(&conditions, now, 100).is_empty())
            .count();

        let loss = 1. - passed as f64 / 10_000.;
        assert!((loss - 0.1).abs() < 0.02, "loss was {loss}");
    }

    #[test]
    fn burst_loss_drops_consecutive_packets() {
        let mut link = LinkState::new(7);
        let conditions = LinkConditions {
            burst_enter: 0.01,
            burst_exit: 0.1,
            ..Default::default()
        };
        let now = Instant::now();

        let mut longest_run = 0;
        let mut run = 0;
        let mut dropped = 0;

        for _ in 0..10_000 {
            if link.plan(&conditions, now, 100).is_empty() {
                dropped += 1;
                run += 1;
                longest_run = longest_run.max(run);
            } else {
                run = 0;
            }
        }

        assert!(dropped > 0);
        // average burst length is 1 / burst_exit = 10
        assert!(longest_run >= 10, "longest run was {longest_run}");
    }

    #[test]
    fn jitter_without_reorder_keeps_order() {
        let mut link = LinkState::new(3);
        let conditions = LinkConditions {
            latency: 50.,
            jitter: 30.,
            reorder: false,
            ..Default::default()
        };
        let start = Instant::now();

        let mut last = start;
        for i in 0..1000 {
            let received = start + Duration::from_millis(i);
            let release = link.plan(&conditions, received, 100)[0];
            assert!(release >= last);
            assert!(release >= received);
            last = release;
        }
    }

    #[test]
    fn jitter_with_reorder_reorders_and_averages_latency() {
        let mut link = LinkState::new(3);
        let conditions = LinkConditions {
            latency: 50.,
            jitter: 10.,
            reorder: true,
            ..Default::default()
        };
        let start = Instant::now();

        let mut last = start;
        let mut reordered = 0;
        let mut total_delay = 0.;
        for i in 0..10_000 {
            let received = start + Duration::from_millis(i);
            let release = link.plan(&conditions, received, 100)[0];
            if release < last {
                reordered += 1;
            }
            last = release;
            total_delay += ms(release - received);
        }

        assert!(reordered > 0);
        let average = total_delay / 10_000.;
        assert!((average - 50.).abs() < 1., "average delay was {average}");
    }

    #[test]
    fn duplicates() {
        let mut link = LinkState::new(9);
        let conditions = LinkConditions {
            duplicate: 0.5,
            ..Default::default()
        };
        let now = Instant::now();

        let total: usize = (0..10_000)
            .map(|_| link.plan(&conditions, now, 100).len())
            .sum();

        assert!((total as f64 / 10_000. - 1.5).abs() < 0.05);
    }

    #[test]
    fn bandwidth_serializes_packets() {
        let mut link = LinkState::new(1);
        let conditions = LinkConditions {
            bandwidth: 80., // 1000 bytes takes 100ms
            ..Default::default()
        };
        let now = Instant::now();

        let first = link.plan(&conditions, now, 1000)[0];
        let second = link.plan(&conditions, now, 1000)[0];
        assert!((ms(first - now) - 100.).abs() < 0.001);
        assert!((ms(second - now) - 200.).abs() < 0.001);

        // queue is now over the limit after enough packets
        let dropped = (0..20)
            .filter(|_| link.plan(&conditions, now, 1000).is_empty())
            .count();
        assert!(dropped > 0);
    }

    #[test]
    fn set_fields() {
        let mut conditions = LinkConditions::default();
        conditions.set("latency", "20").unwrap();
        conditions.set("reorder", "on").unwrap();
        assert_eq!(conditions.latency, 20.);
        assert!(conditions.reorder);
        assert!(conditions.set("loss", "2").is_err());
        assert!(conditions.set("nope", "1").is_err());
        assert!(conditions.set("jitter", "abc").is_err());
    }
}
