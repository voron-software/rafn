pub mod pb {
    tonic::include_proto!("perfscope.v1");
}

pub mod benchmark;
pub mod error;
pub mod metrics;

pub use benchmark::{Benchmark, BenchmarkBuilder};
pub use error::{Error, Result};
pub use metrics::Metrics;
