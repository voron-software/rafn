use crate::ingest::error::Result;
use crate::ingest::parser::BenchmarkParser;
use crate::proto::{Benchmark, Metrics};
use serde::Deserialize;
use uuid::Uuid;

pub struct CriterionParser {
    tenant_id: Uuid,
    repository: String,
    commit_sha: String,
}

#[derive(Deserialize, Debug)]
struct CriterionBenchmark {
    #[serde(rename = "id")]
    benchmark_id: String,
    #[serde(rename = "mean")]
    mean: Estimate,
    #[serde(rename = "median")]
    median: Estimate,
    #[serde(rename = "std_dev")]
    std_dev: Estimate,
    #[serde(default)]
    total_iterations: u64,
}

#[derive(Deserialize, Debug)]
struct Estimate {
    point_estimate: f64,
    #[serde(default)]
    #[allow(dead_code)]
    confidence_interval: ConfidenceInterval,
}

#[derive(Deserialize, Debug, Default)]
struct ConfidenceInterval {
    #[serde(default)]
    #[allow(dead_code)]
    lower_bound: f64,
    #[serde(default)]
    #[allow(dead_code)]
    upper_bound: f64,
}

impl CriterionParser {
    pub fn new(tenant_id: Uuid, repository: String, commit_sha: String) -> Self {
        Self {
            tenant_id,
            repository,
            commit_sha,
        }
    }
}

impl BenchmarkParser for CriterionParser {
    fn parse(&self, json: &str) -> Result<Vec<Benchmark>> {
        let criterion_bench: CriterionBenchmark =
            serde_json::from_str(json).map_err(crate::proto::Error::Serialization)?;

        let mean_ns = criterion_bench.mean.point_estimate;
        let median_ns = criterion_bench.median.point_estimate;
        let stddev_ns = criterion_bench.std_dev.point_estimate;

        let metrics = Metrics::new(
            mean_ns,
            median_ns,
            stddev_ns,
            median_ns - stddev_ns.max(0.0),
            median_ns + stddev_ns,
        )
        .with_iterations(criterion_bench.total_iterations);

        let benchmark = Benchmark::builder()
            .tenant_id(self.tenant_id)
            .repository(self.repository.clone())
            .commit_sha(self.commit_sha.clone())
            .benchmark_name(criterion_bench.benchmark_id)
            .toolset("criterion".to_string())
            .language("rust".to_string())
            .metrics(metrics)
            .build()?;

        Ok(vec![benchmark])
    }

    fn name(&self) -> &'static str {
        "criterion"
    }

    fn can_parse(&self, json: &str) -> bool {
        serde_json::from_str::<CriterionBenchmark>(json).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_criterion_parser() {
        let json = r#"{
            "id": "my_benchmark",
            "mean": {
                "point_estimate": 1234.5,
                "confidence_interval": {
                    "lower_bound": 1200.0,
                    "upper_bound": 1300.0
                }
            },
            "median": {
                "point_estimate": 1230.0,
                "confidence_interval": {
                    "lower_bound": 1200.0,
                    "upper_bound": 1260.0
                }
            },
            "std_dev": {
                "point_estimate": 50.0,
                "confidence_interval": {
                    "lower_bound": 40.0,
                    "upper_bound": 60.0
                }
            }
        }"#;

        let parser = CriterionParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        assert!(parser.can_parse(json));

        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks.len(), 1);

        let b = &benchmarks[0];
        assert_eq!(b.benchmark_name, "my_benchmark");
        assert_eq!(b.toolset, "criterion");
        assert_eq!(b.language, "rust");
        assert!((b.metrics.mean_ns - 1234.5).abs() < f64::EPSILON);
        assert!((b.metrics.median_ns - 1230.0).abs() < f64::EPSILON);
        assert!((b.metrics.stddev_ns - 50.0).abs() < f64::EPSILON);
        assert!((b.metrics.min_ns - 1180.0).abs() < f64::EPSILON);
        assert!((b.metrics.max_ns - 1280.0).abs() < f64::EPSILON);
        assert!(b.metrics.ops_per_sec > 0.0);
        assert_eq!(b.metrics.iterations, 0);
    }

    #[test]
    fn test_criterion_parser_with_iterations() {
        let json = r#"{
            "id": "my_benchmark",
            "mean": {
                "point_estimate": 1234.5,
                "confidence_interval": {"lower_bound": 1200.0, "upper_bound": 1300.0}
            },
            "median": {
                "point_estimate": 1230.0,
                "confidence_interval": {"lower_bound": 1200.0, "upper_bound": 1260.0}
            },
            "std_dev": {
                "point_estimate": 50.0,
                "confidence_interval": {"lower_bound": 40.0, "upper_bound": 60.0}
            },
            "total_iterations": 500
        }"#;

        let parser = CriterionParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks.len(), 1);
        assert_eq!(benchmarks[0].metrics.iterations, 500);
    }
}
