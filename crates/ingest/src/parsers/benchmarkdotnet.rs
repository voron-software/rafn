use crate::error::Result;
use crate::parser::BenchmarkParser;
use proto::{Benchmark, Metrics};
use serde::Deserialize;
use uuid::Uuid;

pub struct BenchmarkDotNetParser {
    tenant_id: Uuid,
    repository: String,
    commit_sha: String,
}

#[derive(Deserialize, Debug)]
struct BenchmarkDotNetReport {
    #[serde(rename = "Benchmarks")]
    benchmarks: Vec<BenchmarkDotNetBenchmark>,
}

#[derive(Deserialize, Debug)]
struct BenchmarkDotNetBenchmark {
    #[serde(rename = "Method")]
    method: String,
    #[serde(rename = "DisplayInfo", default)]
    display_info: Option<String>,
    #[serde(rename = "Type")]
    type_name: String,
    #[serde(rename = "Parameters", default)]
    parameters: String,
    #[serde(rename = "Statistics")]
    statistics: Statistics,
}

#[derive(Deserialize, Debug)]
struct Statistics {
    #[serde(rename = "Mean")]
    mean: f64,
    #[serde(rename = "Median")]
    median: f64,
    #[serde(rename = "StdDev", default)]
    std_dev: f64,
    #[serde(rename = "Min")]
    min: f64,
    #[serde(rename = "Max")]
    max: f64,
    #[serde(rename = "N", default)]
    n: u64,
}

impl BenchmarkDotNetParser {
    pub fn new(tenant_id: Uuid, repository: String, commit_sha: String) -> Self {
        Self {
            tenant_id,
            repository,
            commit_sha,
        }
    }

    fn construct_name(&self, benchmark: &BenchmarkDotNetBenchmark) -> String {
        let base_name = if let Some(ref display_info) = benchmark.display_info {
            display_info.clone()
        } else {
            format!("{}.{}", benchmark.type_name, benchmark.method)
        };

        if !benchmark.parameters.is_empty() {
            format!("{}: {}", base_name, benchmark.parameters)
        } else {
            base_name
        }
    }
}

impl BenchmarkParser for BenchmarkDotNetParser {
    fn parse(&self, json: &str) -> Result<Vec<Benchmark>> {
        let report: BenchmarkDotNetReport =
            serde_json::from_str(json).map_err(proto::Error::Serialization)?;

        let mut benchmarks = Vec::new();

        for bdn_bench in report.benchmarks {
            let name = self.construct_name(&bdn_bench);

            // BenchmarkDotNet reports times in nanoseconds
            let metrics = Metrics::new(
                bdn_bench.statistics.mean,
                bdn_bench.statistics.median,
                bdn_bench.statistics.std_dev,
                bdn_bench.statistics.min,
                bdn_bench.statistics.max,
            )
            .with_iterations(bdn_bench.statistics.n);

            let benchmark = Benchmark::builder()
                .tenant_id(self.tenant_id)
                .repository(self.repository.clone())
                .commit_sha(self.commit_sha.clone())
                .benchmark_name(name)
                .toolset("benchmarkdotnet".to_string())
                .language("csharp".to_string())
                .metrics(metrics)
                .build()?;

            benchmarks.push(benchmark);
        }

        Ok(benchmarks)
    }

    fn name(&self) -> &'static str {
        "benchmarkdotnet"
    }

    fn can_parse(&self, json: &str) -> bool {
        serde_json::from_str::<BenchmarkDotNetReport>(json).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_benchmark() {
        let json = r#"{
            "Benchmarks": [{
                "Method": "TestMethod",
                "Type": "MyNamespace.MyClass",
                "Parameters": "",
                "Statistics": {
                    "Mean": 1234.5, "Median": 1230.0, "StdDev": 50.0,
                    "Min": 1100.0, "Max": 1400.0, "N": 100
                }
            }]
        }"#;

        let parser =
            BenchmarkDotNetParser::new(Uuid::new_v4(), "test/repo".into(), "abc123".into());

        assert!(parser.can_parse(json));
        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks.len(), 1);

        let b = &benchmarks[0];
        assert_eq!(b.benchmark_name, "MyNamespace.MyClass.TestMethod");
        assert_eq!(b.toolset, "benchmarkdotnet");
        assert_eq!(b.language, "csharp");
        assert!((b.metrics.mean_ns - 1234.5).abs() < f64::EPSILON);
        assert!((b.metrics.median_ns - 1230.0).abs() < f64::EPSILON);
        assert!((b.metrics.stddev_ns - 50.0).abs() < f64::EPSILON);
        assert!((b.metrics.min_ns - 1100.0).abs() < f64::EPSILON);
        assert!((b.metrics.max_ns - 1400.0).abs() < f64::EPSILON);
        assert_eq!(b.metrics.iterations, 100);
    }

    #[test]
    fn test_multiple_benchmarks() {
        let json = r#"{
            "Benchmarks": [
                {
                    "Method": "Method1", "Type": "MyClass", "Parameters": "",
                    "Statistics": {"Mean": 1000.0, "Median": 990.0, "StdDev": 10.0, "Min": 900.0, "Max": 1100.0}
                },
                {
                    "Method": "Method2", "Type": "MyClass", "Parameters": "",
                    "Statistics": {"Mean": 2000.0, "Median": 1990.0, "StdDev": 20.0, "Min": 1900.0, "Max": 2100.0}
                }
            ]
        }"#;

        let parser =
            BenchmarkDotNetParser::new(Uuid::new_v4(), "test/repo".into(), "abc123".into());
        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks.len(), 2);
        assert_eq!(benchmarks[0].benchmark_name, "MyClass.Method1");
        assert_eq!(benchmarks[1].benchmark_name, "MyClass.Method2");
    }

    #[test]
    fn test_name_with_display_info() {
        let json = r#"{
            "Benchmarks": [{
                "Method": "TestMethod", "DisplayInfo": "Custom Display Name",
                "Type": "MyClass", "Parameters": "",
                "Statistics": {"Mean": 1000.0, "Median": 990.0, "StdDev": 10.0, "Min": 900.0, "Max": 1100.0}
            }]
        }"#;

        let parser =
            BenchmarkDotNetParser::new(Uuid::new_v4(), "test/repo".into(), "abc123".into());
        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks[0].benchmark_name, "Custom Display Name");
    }

    #[test]
    fn test_name_with_parameters() {
        let json = r#"{
            "Benchmarks": [{
                "Method": "TestMethod", "Type": "MyClass", "Parameters": "N=100, Size=1024",
                "Statistics": {"Mean": 1000.0, "Median": 990.0, "StdDev": 10.0, "Min": 900.0, "Max": 1100.0}
            }]
        }"#;

        let parser =
            BenchmarkDotNetParser::new(Uuid::new_v4(), "test/repo".into(), "abc123".into());
        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(
            benchmarks[0].benchmark_name,
            "MyClass.TestMethod: N=100, Size=1024"
        );
    }

    #[test]
    fn test_name_with_display_info_and_parameters() {
        let json = r#"{
            "Benchmarks": [{
                "Method": "TestMethod", "DisplayInfo": "Custom Name",
                "Type": "MyClass", "Parameters": "N=100",
                "Statistics": {"Mean": 1000.0, "Median": 990.0, "StdDev": 10.0, "Min": 900.0, "Max": 1100.0}
            }]
        }"#;

        let parser =
            BenchmarkDotNetParser::new(Uuid::new_v4(), "test/repo".into(), "abc123".into());
        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks[0].benchmark_name, "Custom Name: N=100");
    }

    #[test]
    fn test_can_parse_criterion_json_negative() {
        let criterion_json = r#"{"id": "my_benchmark", "mean": {"point_estimate": 1234.5}, "median": {"point_estimate": 1230.0}, "std_dev": {"point_estimate": 50.0}}"#;
        let parser =
            BenchmarkDotNetParser::new(Uuid::new_v4(), "test/repo".into(), "abc123".into());
        assert!(!parser.can_parse(criterion_json));
    }
}
