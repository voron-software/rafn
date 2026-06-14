use crate::config::RepositoryRef;
use crate::ingest::error::Result;
use crate::ingest::parser::BenchmarkParser;
use crate::proto::benchmark::{benchmark_record, benchmark_set, metric_statistics};
use crate::proto::pb::BenchmarkSet;
use serde::Deserialize;

pub struct BenchmarkDotNetParser {
    repository: RepositoryRef,
    commit_sha: String,
    branch: Option<String>,
    run_uuid: String,
    run_started_at: prost_types::Timestamp,
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
    pub fn new(
        repository: RepositoryRef,
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
    fn parse(&self, json: &str) -> Result<Vec<BenchmarkSet>> {
        let report: BenchmarkDotNetReport =
            serde_json::from_str(json).map_err(crate::proto::Error::Serialization)?;

        let mut benchmarks = Vec::new();

        for bdn_bench in report.benchmarks {
            let name = self.construct_name(&bdn_bench);

            let statistics = metric_statistics(
                bdn_bench.statistics.mean,
                bdn_bench.statistics.median,
                bdn_bench.statistics.std_dev,
                bdn_bench.statistics.min,
                bdn_bench.statistics.max,
                Some(bdn_bench.statistics.n),
            );
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
            self.run_started_at,
            "csharp",
            "benchmarkdotnet",
            benchmarks,
        )])
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

    fn test_repository() -> RepositoryRef {
        RepositoryRef {
            forge: "github.com".to_string(),
            owner: "test".to_string(),
            repository: "repo".to_string(),
        }
    }

    fn make_parser() -> BenchmarkDotNetParser {
        BenchmarkDotNetParser::new(
            test_repository(),
            "abc123".into(),
            None,
            "run-1".into(),
            prost_types::Timestamp::default(),
        )
    }

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

        let parser = make_parser();

        assert!(parser.can_parse(json));
        let sets = parser.parse(json).unwrap();
        assert_eq!(sets.len(), 1);

        let b = &sets[0].benchmarks[0];
        assert_eq!(b.name, "MyNamespace.MyClass.TestMethod");
        let stats = b.statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 1234.5).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 1230.0).abs() < f64::EPSILON);
        assert!((stats.stddev.unwrap() - 50.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 1100.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 1400.0).abs() < f64::EPSILON);
        assert_eq!(stats.sample_count, Some(100));
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

        let benchmarks = make_parser().parse(json).unwrap();
        assert_eq!(benchmarks[0].benchmarks.len(), 2);
        assert_eq!(benchmarks[0].benchmarks[0].name, "MyClass.Method1");
        assert_eq!(benchmarks[0].benchmarks[1].name, "MyClass.Method2");
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

        let benchmarks = make_parser().parse(json).unwrap();
        assert_eq!(benchmarks[0].benchmarks[0].name, "Custom Display Name");
    }

    #[test]
    fn test_name_with_parameters() {
        let json = r#"{
            "Benchmarks": [{
                "Method": "TestMethod", "Type": "MyClass", "Parameters": "N=100, Size=1024",
                "Statistics": {"Mean": 1000.0, "Median": 990.0, "StdDev": 10.0, "Min": 900.0, "Max": 1100.0}
            }]
        }"#;

        let benchmarks = make_parser().parse(json).unwrap();
        assert_eq!(
            benchmarks[0].benchmarks[0].name,
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

        let benchmarks = make_parser().parse(json).unwrap();
        assert_eq!(benchmarks[0].benchmarks[0].name, "Custom Name: N=100");
    }

    #[test]
    fn test_can_parse_criterion_json_negative() {
        let criterion_json = r#"{"id": "my_benchmark", "mean": {"point_estimate": 1234.5}, "median": {"point_estimate": 1230.0}, "std_dev": {"point_estimate": 50.0}}"#;
        assert!(!make_parser().can_parse(criterion_json));
    }
}
