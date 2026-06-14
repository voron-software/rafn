use crate::config::RepositoryRef;
use crate::ingest::error::Result;
use crate::ingest::parser::BenchmarkParser;
use crate::proto::benchmark::{
    benchmark_record, benchmark_set, metric_statistics, microseconds_to_ns, milliseconds_to_ns,
    seconds_to_ns,
};
use crate::proto::pb::BenchmarkSet;
use serde::Deserialize;
use std::collections::HashMap;

pub struct JmhParser {
    repository: RepositoryRef,
    commit_sha: String,
    branch: Option<String>,
    run_uuid: String,
    run_started_at: prost_types::Timestamp,
}

/// JMH outputs "NaN" as a JSON string (not a number) when statistics can't be computed.
fn deserialize_f64_or_nan<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<f64, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = f64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a float or \"NaN\"")
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<f64, E> {
            Ok(v)
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<f64, E> {
            Ok(v as f64)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<f64, E> {
            Ok(v as f64)
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<f64, E> {
            match v {
                "NaN" | "Infinity" | "-Infinity" => Ok(0.0),
                _ => Err(E::invalid_value(serde::de::Unexpected::Str(v), &self)),
            }
        }
    }
    d.deserialize_any(Visitor)
}

fn deserialize_f64_map_or_nan<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<HashMap<String, f64>, D::Error> {
    struct MapVisitor;
    impl<'de> serde::de::Visitor<'de> for MapVisitor {
        type Value = HashMap<String, f64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a map of string to float-or-NaN")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut result = HashMap::new();
            while let Some(key) = map.next_key::<String>()? {
                let raw: serde_json::Value = map.next_value()?;
                let v = match &raw {
                    serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
                    serde_json::Value::String(s)
                        if matches!(s.as_str(), "NaN" | "Infinity" | "-Infinity") =>
                    {
                        continue;
                    }
                    _ => continue,
                };
                result.insert(key, v);
            }
            Ok(result)
        }
    }
    d.deserialize_map(MapVisitor)
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct JmhBenchmark {
    benchmark: String,
    primary_metric: PrimaryMetric,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct PrimaryMetric {
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    score: f64,
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    score_error: f64,
    score_unit: String,
    #[serde(default, deserialize_with = "deserialize_f64_map_or_nan")]
    percentiles: HashMap<String, f64>,
}

impl JmhParser {
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

    fn convert_to_ns(&self, value: f64, unit: &str) -> f64 {
        match unit {
            "ns/op" => value,
            "us/op" => microseconds_to_ns(value),
            "ms/op" => milliseconds_to_ns(value),
            "s/op" => seconds_to_ns(value),
            _ => value,
        }
    }
}

impl BenchmarkParser for JmhParser {
    fn parse(&self, json: &str) -> Result<Vec<BenchmarkSet>> {
        let jmh_benchmarks: Vec<JmhBenchmark> =
            serde_json::from_str(json).map_err(crate::proto::Error::Serialization)?;

        let mut benchmarks = Vec::new();

        for jmh_bench in jmh_benchmarks {
            let mean_ns = self.convert_to_ns(
                jmh_bench.primary_metric.score,
                &jmh_bench.primary_metric.score_unit,
            );

            let median_ns = jmh_bench
                .primary_metric
                .percentiles
                .get("50.0")
                .map(|&v| self.convert_to_ns(v, &jmh_bench.primary_metric.score_unit))
                .unwrap_or(mean_ns);

            let stddev_ns = self.convert_to_ns(
                jmh_bench.primary_metric.score_error / 3.291,
                &jmh_bench.primary_metric.score_unit,
            );

            let min_ns = jmh_bench
                .primary_metric
                .percentiles
                .get("0.0")
                .map(|&v| self.convert_to_ns(v, &jmh_bench.primary_metric.score_unit))
                .unwrap_or_else(|| (mean_ns - stddev_ns).max(0.0));

            let max_ns = jmh_bench
                .primary_metric
                .percentiles
                .get("100.0")
                .map(|&v| self.convert_to_ns(v, &jmh_bench.primary_metric.score_unit))
                .unwrap_or(mean_ns + stddev_ns);

            let statistics = metric_statistics(mean_ns, median_ns, stddev_ns, min_ns, max_ns, None);
            benchmarks.push(benchmark_record(jmh_bench.benchmark, statistics));
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
            "java",
            "jmh",
            benchmarks,
        )])
    }

    fn name(&self) -> &'static str {
        "jmh"
    }

    fn can_parse(&self, json: &str) -> bool {
        serde_json::from_str::<Vec<JmhBenchmark>>(json).is_ok()
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

    fn make_parser() -> JmhParser {
        JmhParser::new(
            test_repository(),
            "abc123".to_string(),
            None,
            "run-1".to_string(),
            prost_types::Timestamp::default(),
        )
    }

    #[test]
    fn test_jmh_parser_single_benchmark() {
        let json = r#"[{
            "benchmark": "com.example.MyBenchmark.testMethod",
            "primaryMetric": {
                "score": 1234.5,
                "scoreError": 164.55,
                "scoreUnit": "ns/op",
                "percentiles": {
                    "0.0": 1100.0,
                    "50.0": 1230.0,
                    "100.0": 1400.0
                }
            }
        }]"#;

        let parser = make_parser();

        assert!(parser.can_parse(json));

        let sets = parser.parse(json).unwrap();
        assert_eq!(sets.len(), 1);

        let b = &sets[0].benchmarks[0];
        assert_eq!(b.name, "com.example.MyBenchmark.testMethod");
        let stats = b.statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 1234.5).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 1230.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 1100.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 1400.0).abs() < f64::EPSILON);
        assert!((stats.stddev.unwrap() - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_jmh_parser_unit_conversion_microseconds() {
        let json = r#"[{
            "benchmark": "com.example.MyBenchmark.testMethod",
            "primaryMetric": {
                "score": 1.5,
                "scoreError": 0.16455,
                "scoreUnit": "us/op",
                "percentiles": {"0.0": 1.1, "50.0": 1.4, "100.0": 1.8}
            }
        }]"#;

        let sets = make_parser().parse(json).unwrap();
        let stats = sets[0].benchmarks[0].statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 1500.0).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 1400.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 1100.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 1800.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jmh_parser_unit_conversion_milliseconds() {
        let json = r#"[{
            "benchmark": "com.example.MyBenchmark.testMethod",
            "primaryMetric": {
                "score": 2.5,
                "scoreError": 0.32910,
                "scoreUnit": "ms/op",
                "percentiles": {"0.0": 2.0, "50.0": 2.4, "100.0": 3.0}
            }
        }]"#;

        let sets = make_parser().parse(json).unwrap();
        let stats = sets[0].benchmarks[0].statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 2_500_000.0).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 2_400_000.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 2_000_000.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 3_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jmh_parser_unit_conversion_seconds() {
        let json = r#"[{
            "benchmark": "com.example.MyBenchmark.testMethod",
            "primaryMetric": {
                "score": 0.5,
                "scoreError": 0.06582,
                "scoreUnit": "s/op",
                "percentiles": {"0.0": 0.4, "50.0": 0.48, "100.0": 0.6}
            }
        }]"#;

        let sets = make_parser().parse(json).unwrap();
        let stats = sets[0].benchmarks[0].statistics.as_ref().unwrap();
        assert!((stats.mean.unwrap() - 500_000_000.0).abs() < f64::EPSILON);
        assert!((stats.median.unwrap() - 480_000_000.0).abs() < f64::EPSILON);
        assert!((stats.min.unwrap() - 400_000_000.0).abs() < f64::EPSILON);
        assert!((stats.max.unwrap() - 600_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jmh_parser_multiple_benchmarks() {
        let json = r#"[
            {
                "benchmark": "com.example.Benchmark1.method1",
                "primaryMetric": {
                    "score": 1000.0, "scoreError": 164.55, "scoreUnit": "ns/op",
                    "percentiles": {"0.0": 900.0, "50.0": 980.0, "100.0": 1100.0}
                }
            },
            {
                "benchmark": "com.example.Benchmark2.method2",
                "primaryMetric": {
                    "score": 2000.0, "scoreError": 329.1, "scoreUnit": "ns/op",
                    "percentiles": {"0.0": 1800.0, "50.0": 1950.0, "100.0": 2200.0}
                }
            }
        ]"#;

        let sets = make_parser().parse(json).unwrap();
        assert_eq!(sets[0].benchmarks.len(), 2);
        let first = sets[0].benchmarks[0].statistics.as_ref().unwrap();
        let second = sets[0].benchmarks[1].statistics.as_ref().unwrap();
        assert!((first.mean.unwrap() - 1000.0).abs() < f64::EPSILON);
        assert!((second.mean.unwrap() - 2000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jmh_parser_missing_percentiles() {
        let json = r#"[{
            "benchmark": "com.example.MyBenchmark.testMethod",
            "primaryMetric": {
                "score": 1234.5,
                "scoreError": 164.55,
                "scoreUnit": "ns/op"
            }
        }]"#;

        let parser = make_parser();

        assert!(parser.can_parse(json));

        let sets = parser.parse(json).unwrap();
        let stats = sets[0].benchmarks[0].statistics.as_ref().unwrap();
        assert!((stats.median.unwrap() - 1234.5).abs() < f64::EPSILON);
        let expected_stddev = 50.0;
        assert!((stats.stddev.unwrap() - expected_stddev).abs() < 0.1);
        assert!((stats.min.unwrap() - (1234.5 - expected_stddev)).abs() < 0.1);
        assert!((stats.max.unwrap() - (1234.5 + expected_stddev)).abs() < 0.1);
    }

    #[test]
    fn test_jmh_can_parse_invalid() {
        let parser = make_parser();
        assert!(!parser.can_parse(r#"{"not": "jmh"}"#));
    }

    #[test]
    fn test_jmh_can_parse_criterion_format() {
        let parser = make_parser();
        assert!(!parser.can_parse(r#"{"id": "my_benchmark", "mean": {"point_estimate": 1234.5}}"#));
    }
}
