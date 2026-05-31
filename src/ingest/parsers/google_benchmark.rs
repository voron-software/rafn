use crate::ingest::error::Result;
use crate::ingest::parser::BenchmarkParser;
use crate::proto::benchmark::{
    benchmark_record, benchmark_set, metric_statistics, microseconds_to_ns, milliseconds_to_ns,
    seconds_to_ns,
};
use crate::proto::pb::BenchmarkSet;
use serde::Deserialize;
use std::collections::HashMap;

pub struct GoogleBenchmarkParser {
    repository: String,
    commit_sha: String,
    branch: Option<String>,
    run_uuid: String,
    run_started_at: prost_types::Timestamp,
}

#[derive(Deserialize, Debug)]
struct GBenchReport {
    #[allow(dead_code)]
    context: serde_json::Value,
    benchmarks: Vec<GBenchEntry>,
}

#[derive(Deserialize, Debug)]
struct GBenchEntry {
    name: String,
    run_type: String,
    cpu_time: f64,
    time_unit: String,
}

impl GoogleBenchmarkParser {
    pub fn new(
        repository: String,
        commit_sha: String,
        branch: Option<String>,
        run_uuid: String,
        run_started_at: prost_types::Timestamp,
    ) -> Self {
        Self {
            repository,
            commit_sha,
            branch,
            run_uuid,
            run_started_at,
        }
    }

    fn convert_to_ns(&self, value: f64, unit: &str) -> f64 {
        match unit {
            "ns" => value,
            "us" => microseconds_to_ns(value),
            "ms" => milliseconds_to_ns(value),
            "s" => seconds_to_ns(value),
            _ => value,
        }
    }
}

impl BenchmarkParser for GoogleBenchmarkParser {
    fn parse(&self, json: &str) -> Result<Vec<BenchmarkSet>> {
        let report: GBenchReport =
            serde_json::from_str(json).map_err(crate::proto::Error::Serialization)?;

        let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
        let mut units: HashMap<String, String> = HashMap::new();
        for entry in &report.benchmarks {
            if entry.run_type == "iteration" {
                units
                    .entry(entry.name.clone())
                    .or_insert_with(|| entry.time_unit.clone());
                groups
                    .entry(entry.name.clone())
                    .or_default()
                    .push(entry.cpu_time);
            }
        }

        let mut benchmarks = Vec::new();

        for (name, cpu_times) in groups {
            let unit = units.get(&name).map(String::as_str).unwrap_or("ns");
            let times_ns: Vec<f64> = cpu_times
                .iter()
                .map(|&v| self.convert_to_ns(v, unit))
                .collect();

            let count = times_ns.len() as f64;
            let mean_ns = times_ns.iter().sum::<f64>() / count;

            let mut sorted = times_ns.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            let median_ns = if sorted.len().is_multiple_of(2) {
                (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
            } else {
                sorted[sorted.len() / 2]
            };

            let stddev_ns = if sorted.len() == 1 {
                0.0
            } else {
                let variance =
                    times_ns.iter().map(|v| (v - mean_ns).powi(2)).sum::<f64>() / (count - 1.0);
                variance.sqrt()
            };

            let min_ns = sorted[0];
            let max_ns = sorted[sorted.len() - 1];

            let statistics = metric_statistics(mean_ns, median_ns, stddev_ns, min_ns, max_ns, None);
            benchmarks.push(benchmark_record(name, statistics));
        }

        if benchmarks.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![benchmark_set(
            &self.repository,
            &self.commit_sha,
            self.branch.clone(),
            self.run_uuid.clone(),
            self.run_started_at.clone(),
            "cpp",
            "google_benchmark",
            benchmarks,
        )])
    }

    fn name(&self) -> &'static str {
        "google_benchmark"
    }

    fn can_parse(&self, json: &str) -> bool {
        serde_json::from_str::<GBenchReport>(json).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_parser() -> GoogleBenchmarkParser {
        GoogleBenchmarkParser::new(
            "test/repo".into(),
            "abc123".into(),
            None,
            "run-1".into(),
            prost_types::Timestamp::default(),
        )
    }

    #[test]
    fn test_single_benchmark_ns() {
        let json = r#"{
            "context": { "date": "2024-01-01", "host_name": "test" },
            "benchmarks": [
                {
                    "name": "BM_Fibonacci/10",
                    "run_type": "iteration",
                    "iterations": 1000000,
                    "real_time": 45.0,
                    "cpu_time": 44.5,
                    "time_unit": "ns"
                }
            ]
        }"#;

        let parser = make_parser();
        assert!(parser.can_parse(json));

        let sets = parser.parse(json).unwrap();
        assert_eq!(sets.len(), 1);

        let b = &sets[0].benchmarks[0];
        assert_eq!(b.name, "BM_Fibonacci/10");
        let stats = b.statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 44.5).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 44.5).abs() < f64::EPSILON);
        assert_eq!(stats.stddev.unwrap(), 0.0);
        assert_eq!(stats.min, stats.max);
    }

    #[test]
    fn test_multiple_benchmarks() {
        let json = r#"{
            "context": { "date": "2024-01-01" },
            "benchmarks": [
                {
                    "name": "BM_FibRecursive/10",
                    "run_type": "iteration",
                    "iterations": 1000000,
                    "real_time": 100.0,
                    "cpu_time": 100.0,
                    "time_unit": "ns"
                },
                {
                    "name": "BM_FibIterative/10",
                    "run_type": "iteration",
                    "iterations": 5000000,
                    "real_time": 10.0,
                    "cpu_time": 10.0,
                    "time_unit": "ns"
                }
            ]
        }"#;

        let parser = make_parser();
        let sets = parser.parse(json).unwrap();
        assert_eq!(sets[0].benchmarks.len(), 2);

        let names: Vec<&str> = sets[0].benchmarks.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"BM_FibRecursive/10"));
        assert!(names.contains(&"BM_FibIterative/10"));
    }

    #[test]
    fn test_time_unit_microseconds() {
        let json = r#"{
            "context": {},
            "benchmarks": [
                {"name": "BM_Sort", "run_type": "iteration", "iterations": 1000, "real_time": 1.5, "cpu_time": 1.5, "time_unit": "us"}
            ]
        }"#;

        let sets = make_parser().parse(json).unwrap();
        let mean = sets[0].benchmarks[0]
            .statistics
            .as_ref()
            .unwrap()
            .mean
            .unwrap();
        assert!((mean - 1500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_time_unit_milliseconds() {
        let json = r#"{
            "context": {},
            "benchmarks": [
                {"name": "BM_Sort", "run_type": "iteration", "iterations": 100, "real_time": 2.5, "cpu_time": 2.5, "time_unit": "ms"}
            ]
        }"#;

        let sets = make_parser().parse(json).unwrap();
        let mean = sets[0].benchmarks[0]
            .statistics
            .as_ref()
            .unwrap()
            .mean
            .unwrap();
        assert!((mean - 2_500_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_time_unit_seconds() {
        let json = r#"{
            "context": {},
            "benchmarks": [
                {"name": "BM_Heavy", "run_type": "iteration", "iterations": 10, "real_time": 0.5, "cpu_time": 0.5, "time_unit": "s"}
            ]
        }"#;

        let sets = make_parser().parse(json).unwrap();
        let mean = sets[0].benchmarks[0]
            .statistics
            .as_ref()
            .unwrap()
            .mean
            .unwrap();
        assert!((mean - 500_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_repetitions_aggregate_rows_skipped() {
        let json = r#"{
            "context": {},
            "benchmarks": [
                {"name": "BM_Fib/10", "run_type": "iteration", "iterations": 1000000, "real_time": 40.0, "cpu_time": 40.0, "time_unit": "ns"},
                {"name": "BM_Fib/10", "run_type": "iteration", "iterations": 1000000, "real_time": 50.0, "cpu_time": 50.0, "time_unit": "ns"},
                {"name": "BM_Fib/10", "run_type": "iteration", "iterations": 1000000, "real_time": 60.0, "cpu_time": 60.0, "time_unit": "ns"},
                {"name": "BM_Fib/10_mean", "run_type": "aggregate", "aggregate_name": "mean", "iterations": 1, "real_time": 50.0, "cpu_time": 50.0, "time_unit": "ns"},
                {"name": "BM_Fib/10_stddev", "run_type": "aggregate", "aggregate_name": "stddev", "iterations": 1, "real_time": 10.0, "cpu_time": 10.0, "time_unit": "ns"}
            ]
        }"#;

        let parser = make_parser();
        let sets = parser.parse(json).unwrap();
        assert_eq!(sets[0].benchmarks.len(), 1);

        let b = &sets[0].benchmarks[0];
        assert_eq!(b.name, "BM_Fib/10");
        let stats = b.statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 50.0).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 50.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 40.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 60.0).abs() < f64::EPSILON);
        assert!(stats.stddev.unwrap() > 0.0);
    }

    #[test]
    fn test_can_parse_rejects_jmh() {
        let jmh_json = r#"[{"benchmark": "com.example.Test.method", "primaryMetric": {"score": 100.0, "scoreError": 5.0, "scoreUnit": "ns/op"}}]"#;
        assert!(!make_parser().can_parse(jmh_json));
    }
}
