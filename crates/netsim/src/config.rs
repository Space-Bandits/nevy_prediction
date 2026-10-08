use serde::Deserialize;

use crate::conditions::{Conditions, LinkConditions};

/// Written to the config path if no config file exists.
pub const TEMPLATE: &str = r#"# netsim config. Changes are applied live when this file is saved,
# except for `listen`, `target` and `seed` which need a restart.

# Address clients connect to.
listen = "127.0.0.1:7000"
# Address of the real server.
target = "127.0.0.1:7001"
# Seed for the random number generator, remove for a random seed.
# seed = 1
# Seconds between printing statistics, 0 disables them.
stats = 5
# Drop all traffic, e.g. to simulate a disconnect.
paused = false

# Conditions applied to both directions. Anything set in [up] or [down] overrides these.
#   latency     one way delay in ms
#   jitter      standard deviation of extra delay in ms
#   reorder     allow jitter to reorder packets
#   loss        probability of dropping a packet (0..1)
#   burst_enter per packet probability of starting a loss burst, 0 disables bursts (0..1)
#   burst_exit  per packet probability of ending a loss burst (0..1)
#   duplicate   probability of duplicating a packet (0..1)
#   bandwidth   kilobits per second, 0 is unlimited
[both]
latency = 50
jitter = 5
reorder = false
loss = 0
burst_enter = 0
burst_exit = 0.5
duplicate = 0
bandwidth = 0

# Client -> server.
[up]

# Server -> client.
[down]

"#;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_target")]
    pub target: String,
    pub seed: Option<u64>,
    /// Seconds between printing statistics.
    #[serde(default = "default_stats")]
    pub stats: f64,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub both: PartialLink,
    #[serde(default)]
    pub up: PartialLink,
    #[serde(default)]
    pub down: PartialLink,
}

fn default_listen() -> String {
    "127.0.0.1:7000".to_string()
}

fn default_target() -> String {
    "127.0.0.1:7001".to_string()
}

fn default_stats() -> f64 {
    5.
}

/// Link conditions where unset fields are inherited.
#[derive(Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PartialLink {
    pub latency: Option<f64>,
    pub jitter: Option<f64>,
    pub reorder: Option<bool>,
    pub loss: Option<f64>,
    pub burst_enter: Option<f64>,
    pub burst_exit: Option<f64>,
    pub duplicate: Option<f64>,
    pub bandwidth: Option<f64>,
}

impl PartialLink {
    fn apply(&self, link: &mut LinkConditions) -> Result<(), String> {
        let fields = [
            ("latency", self.latency.map(|v| v.to_string())),
            ("jitter", self.jitter.map(|v| v.to_string())),
            ("reorder", self.reorder.map(|v| v.to_string())),
            ("loss", self.loss.map(|v| v.to_string())),
            ("burst_enter", self.burst_enter.map(|v| v.to_string())),
            ("burst_exit", self.burst_exit.map(|v| v.to_string())),
            ("duplicate", self.duplicate.map(|v| v.to_string())),
            ("bandwidth", self.bandwidth.map(|v| v.to_string())),
        ];

        for (field, value) in fields {
            if let Some(value) = value {
                link.set(field, &value)
                    .map_err(|error| format!("{field}: {error}"))?;
            }
        }

        Ok(())
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Config = toml::from_str(text).map_err(|error| error.to_string())?;

        // validate everything up front so a bad config is rejected as a whole
        config.conditions()?;
        if config.stats.is_nan() || config.stats < 0. {
            return Err("stats must be a non-negative number".to_string());
        }

        Ok(config)
    }

    pub fn conditions(&self) -> Result<Conditions, String> {
        let mut conditions = Conditions {
            paused: self.paused,
            ..Default::default()
        };

        for link in [&mut conditions.up, &mut conditions.down] {
            self.both
                .apply(link)
                .map_err(|error| format!("[both] {error}"))?;
        }
        self.up
            .apply(&mut conditions.up)
            .map_err(|error| format!("[up] {error}"))?;
        self.down
            .apply(&mut conditions.down)
            .map_err(|error| format!("[down] {error}"))?;

        Ok(conditions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let config = Config::parse(TEMPLATE).unwrap();
        let conditions = config.conditions().unwrap();
        assert_eq!(conditions.up.latency, 50.);
        assert_eq!(conditions.down.jitter, 5.);
    }

    #[test]
    fn empty_config_uses_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.listen, "127.0.0.1:7000");
        assert_eq!(config.conditions().unwrap(), Conditions::default());
    }

    #[test]
    fn directions_override_both() {
        let config = Config::parse(
            r#"
            [both]
            latency = 20
            loss = 0.1
            [down]
            latency = 80.5
            "#,
        )
        .unwrap();

        let conditions = config.conditions().unwrap();
        assert_eq!(conditions.up.latency, 20.);
        assert_eq!(conditions.down.latency, 80.5);
        assert_eq!(conditions.down.loss, 0.1);
    }

    #[test]
    fn invalid_configs() {
        assert!(Config::parse("[both]\nloss = 2").is_err());
        assert!(Config::parse("[both]\nlatancy = 2").is_err());
        assert!(Config::parse("bogus = 1").is_err());
        assert!(Config::parse("stats = -1").is_err());
    }
}
