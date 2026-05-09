pub mod auto_detect;
pub mod error;
pub mod parser;
pub mod parsers;
pub mod validation;

pub use self::auto_detect::detect_format;
pub use self::error::{Error, Result};
pub use self::parser::BenchmarkParser;

use self::parsers::benchmarkdotnet::BenchmarkDotNetParser;
use self::parsers::criterion::CriterionParser;
use self::parsers::google_benchmark::GoogleBenchmarkParser;
use self::parsers::jmh::JmhParser;
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
