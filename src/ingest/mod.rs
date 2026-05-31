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

pub fn get_parser(
    format: &str,
    repository: String,
    commit_sha: String,
    branch: Option<String>,
    run_uuid: String,
    run_started_at: prost_types::Timestamp,
) -> Result<Box<dyn BenchmarkParser>> {
    match format {
        "criterion" => Ok(Box::new(CriterionParser::new(
            repository,
            commit_sha,
            branch,
            run_uuid,
            run_started_at,
        ))),
        "jmh" => Ok(Box::new(JmhParser::new(
            repository,
            commit_sha,
            branch,
            run_uuid,
            run_started_at,
        ))),
        "benchmarkdotnet" => Ok(Box::new(BenchmarkDotNetParser::new(
            repository,
            commit_sha,
            branch,
            run_uuid,
            run_started_at,
        ))),
        "google_benchmark" => Ok(Box::new(GoogleBenchmarkParser::new(
            repository,
            commit_sha,
            branch,
            run_uuid,
            run_started_at,
        ))),
        _ => Err(Error::UnknownFormat),
    }
}
