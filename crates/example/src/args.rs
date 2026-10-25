//! A minimal `--key value` argument parser for the example binaries.

use std::{collections::HashMap, str::FromStr, time::Duration};

pub struct Args {
    values: HashMap<String, Option<String>>,
    positional: Vec<String>,
}

impl Args {
    pub fn from_env() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut values = HashMap::new();
        let mut positional = Vec::new();
        let mut args = args.into_iter().peekable();

        while let Some(arg) = args.next() {
            if let Some(key) = arg.strip_prefix("--") {
                let value = args.next_if(|next| !next.starts_with("--"));
                values.insert(key.to_string(), value);
            } else {
                positional.push(arg);
            }
        }

        Args { values, positional }
    }

    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positional.get(index).map(String::as_str)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn get<T>(&self, key: &str) -> Option<T>
    where
        T: FromStr,
        T::Err: std::fmt::Debug,
    {
        let value = self.values.get(key)?.as_ref().unwrap_or_else(|| {
            panic!("Expected a value for --{key}");
        });

        Some(
            value
                .parse()
                .unwrap_or_else(|error| panic!("Invalid value for --{key}: {error:?}")),
        )
    }

    pub fn get_or<T>(&self, key: &str, default: T) -> T
    where
        T: FromStr,
        T::Err: std::fmt::Debug,
    {
        self.get(key).unwrap_or(default)
    }

    /// Parses a number of milliseconds.
    pub fn millis(&self, key: &str) -> Option<Duration> {
        self.get::<f64>(key)
            .map(|millis| Duration::from_secs_f64(millis / 1000.))
    }

    /// Parses a number of seconds.
    pub fn seconds(&self, key: &str) -> Option<Duration> {
        self.get::<f64>(key).map(Duration::from_secs_f64)
    }
}
