#![allow(unused_crate_dependencies)]

pub mod auto_detect;
pub mod error;
pub mod parser;
pub mod parsers;
pub mod validation;

pub use auto_detect::detect_format;
pub use error::{Error, Result};
pub use parser::BenchmarkParser;

use crate::parsers::benchmarkdotnet::BenchmarkDotNetParser;
use crate::parsers::criterion::CriterionParser;
use crate::parsers::google_benchmark::GoogleBenchmarkParser;
use crate::parsers::jmh::JmhParser;
use uuid::Uuid;

pub fn get_parser(
    format: &str,
    tenant_id: Uuid,
    repository: String,
    commit_sha: String,
) -> Result<Box<dyn BenchmarkParser>> {
    match format {
        "criterion" => Ok(Box::new(CriterionParser::new(
            tenant_id, repository, commit_sha,
        ))),
        "jmh" => Ok(Box::new(JmhParser::new(tenant_id, repository, commit_sha))),
        "benchmarkdotnet" => Ok(Box::new(BenchmarkDotNetParser::new(
            tenant_id, repository, commit_sha,
        ))),
        "google_benchmark" => Ok(Box::new(GoogleBenchmarkParser::new(
            tenant_id, repository, commit_sha,
        ))),
        _ => Err(Error::UnknownFormat),
    }
}
