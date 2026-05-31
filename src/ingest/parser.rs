use super::error::Result;
use crate::proto::pb::BenchmarkSet;

pub trait BenchmarkParser: Send + Sync {
    fn parse(&self, json: &str) -> Result<Vec<BenchmarkSet>>;
    fn name(&self) -> &'static str;
    fn can_parse(&self, json: &str) -> bool;
}
