use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Metrics {
    pub mean_ns: f64,
    pub median_ns: f64,
    pub stddev_ns: f64,
    pub min_ns: f64,
    pub max_ns: f64,
    pub iterations: u64,
    pub ops_per_sec: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            mean_ns: 0.0,
            median_ns: 0.0,
            stddev_ns: 0.0,
            min_ns: 0.0,
            max_ns: 0.0,
            iterations: 0,
            ops_per_sec: 0.0,
        }
    }
}

impl Metrics {
    pub fn new(mean_ns: f64, median_ns: f64, stddev_ns: f64, min_ns: f64, max_ns: f64) -> Self {
        let ops_per_sec = if mean_ns > 0.0 {
            1_000_000_000.0 / mean_ns
        } else {
            0.0
        };
        Self {
            mean_ns,
            median_ns,
            stddev_ns,
            min_ns,
            max_ns,
            iterations: 0,
            ops_per_sec,
        }
    }

    pub fn with_iterations(mut self, iterations: u64) -> Self {
        self.iterations = iterations;
        self
    }

    pub fn from_seconds(seconds: f64) -> f64 {
        seconds * 1_000_000_000.0
    }

    pub fn from_milliseconds(ms: f64) -> f64 {
        ms * 1_000_000.0
    }

    pub fn from_microseconds(us: f64) -> f64 {
        us * 1_000.0
    }
}
