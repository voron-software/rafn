use super::parser::BenchmarkParser;
use super::parsers::benchmarkdotnet::BenchmarkDotNetParser;
use super::parsers::criterion::CriterionParser;
use super::parsers::google_benchmark::GoogleBenchmarkParser;
use super::parsers::jmh::JmhParser;
use super::{Error, Result};
use uuid::Uuid;

pub fn detect_format(json: &str) -> Result<String> {
    // Try BenchmarkDotNet first (most specific format)
    let bdn_parser =
        BenchmarkDotNetParser::new(Uuid::nil(), "dummy".to_string(), "dummy".to_string());
    if bdn_parser.can_parse(json) {
        return Ok("benchmarkdotnet".to_string());
    }

    // Try Google Benchmark (distinctive context + benchmarks object structure)
    let gbench_parser =
        GoogleBenchmarkParser::new(Uuid::nil(), "dummy".to_string(), "dummy".to_string());
    if gbench_parser.can_parse(json) {
        return Ok("google_benchmark".to_string());
    }

    // Try JMH
    let jmh_parser = JmhParser::new(Uuid::nil(), "dummy".to_string(), "dummy".to_string());
    if jmh_parser.can_parse(json) {
        return Ok("jmh".to_string());
    }

    // Try Criterion (least specific)
    let criterion_parser =
        CriterionParser::new(Uuid::nil(), "dummy".to_string(), "dummy".to_string());
    if criterion_parser.can_parse(json) {
        return Ok("criterion".to_string());
    }

    Err(Error::UnknownFormat)
}
