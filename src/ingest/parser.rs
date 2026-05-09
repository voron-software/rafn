use super::error::Result;
use crate::proto::Benchmark;

pub trait BenchmarkParser: Send + Sync {
    fn parse(&self, json: &str) -> Result<Vec<Benchmark>>;
    fn name(&self) -> &'static str;
    fn can_parse(&self, json: &str) -> bool;
}
