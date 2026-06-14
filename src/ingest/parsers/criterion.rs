use crate::config::RepositoryRef;
use crate::ingest::error::Result;
use crate::ingest::parser::BenchmarkParser;
use crate::proto::benchmark::{benchmark_record, benchmark_set, metric_statistics};
use crate::proto::pb::BenchmarkSet;
use serde::Deserialize;

pub struct CriterionParser {
    repository: RepositoryRef,
    commit_sha: String,
    branch: Option<String>,
    run_uuid: String,
    run_started_at: prost_types::Timestamp,
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
}

impl BenchmarkParser for CriterionParser {
    fn parse(&self, json: &str) -> Result<Vec<BenchmarkSet>> {
        let criterion_bench: CriterionBenchmark =
            serde_json::from_str(json).map_err(crate::proto::Error::Serialization)?;

        let mean_ns = criterion_bench.mean.point_estimate;
        let median_ns = criterion_bench.median.point_estimate;
        let stddev_ns = criterion_bench.std_dev.point_estimate;

        let statistics = metric_statistics(
            mean_ns,
            median_ns,
            stddev_ns,
            median_ns - stddev_ns.max(0.0),
            median_ns + stddev_ns,
            Some(criterion_bench.total_iterations),
        );
        let benchmark = benchmark_record(criterion_bench.benchmark_id, statistics);

        Ok(vec![benchmark_set(
            &self.repository,
            &self.commit_sha,
            self.branch.clone(),
            self.run_uuid.clone(),
            self.run_started_at,
            "rust",
            "criterion",
            vec![benchmark],
        )])
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

    fn test_repository() -> RepositoryRef {
        RepositoryRef {
            forge: "github.com".to_string(),
            owner: "test".to_string(),
            repository: "repo".to_string(),
        }
    }

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
            test_repository(),
            "abc123".to_string(),
            Some("main".to_string()),
            "run-1".to_string(),
            prost_types::Timestamp::default(),
        );

        assert!(parser.can_parse(json));

        let sets = parser.parse(json).unwrap();
        assert_eq!(sets.len(), 1);

        let set = &sets[0];
        assert_eq!(set.metric_name, "wall_time");
        assert_eq!(set.source.as_ref().unwrap().commit_sha, "abc123");
        assert_eq!(
            set.source.as_ref().unwrap().branch,
            Some("main".to_string())
        );
        let b = &set.benchmarks[0];
        assert_eq!(b.name, "my_benchmark");
        let stats = b.statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 1234.5).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 1230.0).abs() < f64::EPSILON);
        assert!((stats.stddev.unwrap() - 50.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 1180.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 1280.0).abs() < f64::EPSILON);
        assert_eq!(stats.sample_count, Some(0));
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
            test_repository(),
            "abc123".to_string(),
            None,
            "run-1".to_string(),
            prost_types::Timestamp::default(),
        );

        let sets = parser.parse(json).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(
            sets[0].benchmarks[0]
                .statistics
                .as_ref()
                .unwrap()
                .sample_count,
            Some(500)
        );
    }
}
