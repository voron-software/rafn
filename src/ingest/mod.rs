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
use crate::config::RepositoryRef;
use crate::discovery::BenchmarkResult;
use crate::proto::pb::BenchmarkSet;

pub fn get_parser(
    format: &str,
    repository: RepositoryRef,
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

/// Parse every discovered result file into benchmark sets. A file no parser
/// accepts is logged and skipped rather than failing the whole run, so one
/// stray JSON file in a results directory doesn't discard every other result.
pub fn parse_discovered(
    discovered: &[BenchmarkResult],
    repository: &RepositoryRef,
    commit_sha: &str,
    branch: Option<&str>,
    run_uuid: &str,
    run_started_at: prost_types::Timestamp,
) -> std::result::Result<Vec<BenchmarkSet>, serde_json::Error> {
    let mut benchmark_sets = Vec::new();
    for bench in discovered {
        let json = serde_json::to_string(&bench.data)?;
        let format = detect_format(&json).unwrap_or_else(|_| "criterion".to_string());
        let parser = get_parser(
            &format,
            repository.clone(),
            commit_sha.to_string(),
            branch.map(str::to_string),
            run_uuid.to_string(),
            run_started_at,
        );
        match parser {
            Ok(p) => match p.parse(&json) {
                Ok(mut parsed) => benchmark_sets.append(&mut parsed),
                Err(e) => tracing::warn!("Failed to parse {}: {e}", bench.name),
            },
            Err(e) => tracing::warn!("No parser for {}: {e}", bench.name),
        }
    }
    Ok(benchmark_sets)
}
