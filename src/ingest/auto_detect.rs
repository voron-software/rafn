use super::parser::BenchmarkParser;
use super::parsers::benchmarkdotnet::BenchmarkDotNetParser;
use super::parsers::criterion::CriterionParser;
use super::parsers::google_benchmark::GoogleBenchmarkParser;
use super::parsers::jmh::JmhParser;
use super::{Error, Result};

pub fn detect_format(json: &str) -> Result<String> {
    let ts = prost_types::Timestamp::default();

    // Try BenchmarkDotNet first (most specific format)
    let bdn_parser = BenchmarkDotNetParser::new(
        "dummy".to_string(),
        "dummy".to_string(),
        None,
        String::new(),
        ts.clone(),
    );
    if bdn_parser.can_parse(json) {
        return Ok("benchmarkdotnet".to_string());
    }

    // Try Google Benchmark (distinctive context + benchmarks object structure)
    let gbench_parser = GoogleBenchmarkParser::new(
        "dummy".to_string(),
        "dummy".to_string(),
        None,
        String::new(),
        ts.clone(),
    );
    if gbench_parser.can_parse(json) {
        return Ok("google_benchmark".to_string());
    }

    // Try JMH
    let jmh_parser = JmhParser::new(
        "dummy".to_string(),
        "dummy".to_string(),
        None,
        String::new(),
        ts.clone(),
    );
    if jmh_parser.can_parse(json) {
        return Ok("jmh".to_string());
    }

    // Try Criterion (least specific)
    let criterion_parser = CriterionParser::new(
        "dummy".to_string(),
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
