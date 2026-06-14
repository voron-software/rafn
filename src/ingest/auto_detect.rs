use super::parser::BenchmarkParser;
use super::parsers::benchmarkdotnet::BenchmarkDotNetParser;
use super::parsers::criterion::CriterionParser;
use super::parsers::google_benchmark::GoogleBenchmarkParser;
use super::parsers::jmh::JmhParser;
use super::{Error, Result};
use crate::config::RepositoryRef;

fn dummy_repository() -> RepositoryRef {
    RepositoryRef {
        forge: "dummy".to_string(),
        owner: "dummy".to_string(),
        repository: "dummy".to_string(),
    }
}

pub fn detect_format(json: &str) -> Result<String> {
    let ts = prost_types::Timestamp::default();

    // Try BenchmarkDotNet first (most specific format)
    let bdn_parser = BenchmarkDotNetParser::new(
        dummy_repository(),
        "dummy".to_string(),
        None,
        String::new(),
        ts,
    );
    if bdn_parser.can_parse(json) {
        return Ok("benchmarkdotnet".to_string());
    }

    // Try Google Benchmark (distinctive context + benchmarks object structure)
    let gbench_parser = GoogleBenchmarkParser::new(
        dummy_repository(),
        "dummy".to_string(),
        None,
        String::new(),
        ts,
    );
    if gbench_parser.can_parse(json) {
        return Ok("google_benchmark".to_string());
    }

    // Try JMH
    let jmh_parser = JmhParser::new(
        dummy_repository(),
        "dummy".to_string(),
        None,
        String::new(),
        ts,
    );
    if jmh_parser.can_parse(json) {
        return Ok("jmh".to_string());
    }

    // Try Criterion (least specific)
    let criterion_parser = CriterionParser::new(
        dummy_repository(),
        "dummy".to_string(),
        None,
        String::new(),
        ts,
    );
    if criterion_parser.can_parse(json) {
        return Ok("criterion".to_string());
    }

    Err(Error::UnknownFormat)
}
