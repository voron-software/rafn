use crate::error::Result;
use crate::parser::BenchmarkParser;
use proto::{Benchmark, Metrics};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

pub struct JmhParser {
    tenant_id: Uuid,
    repository: String,
    commit_sha: String,
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
    pub fn new(tenant_id: Uuid, repository: String, commit_sha: String) -> Self {
        Self {
            tenant_id,
            repository,
            commit_sha,
        }
    }

    fn convert_to_ns(&self, value: f64, unit: &str) -> f64 {
        match unit {
            "ns/op" => value,
            "us/op" => Metrics::from_microseconds(value),
            "ms/op" => Metrics::from_milliseconds(value),
            "s/op" => Metrics::from_seconds(value),
            _ => value,
        }
    }
}

impl BenchmarkParser for JmhParser {
    fn parse(&self, json: &str) -> Result<Vec<Benchmark>> {
        let jmh_benchmarks: Vec<JmhBenchmark> =
            serde_json::from_str(json).map_err(proto::Error::Serialization)?;

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

            let metrics = Metrics::new(mean_ns, median_ns, stddev_ns, min_ns, max_ns);

            let benchmark = Benchmark::builder()
                .tenant_id(self.tenant_id)
                .repository(self.repository.clone())
                .commit_sha(self.commit_sha.clone())
                .benchmark_name(jmh_bench.benchmark)
                .toolset("jmh".to_string())
                .language("java".to_string())
                .metrics(metrics)
                .build()?;

            benchmarks.push(benchmark);
        }

        Ok(benchmarks)
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

        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        assert!(parser.can_parse(json));

        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks.len(), 1);

        let b = &benchmarks[0];
        assert_eq!(b.benchmark_name, "com.example.MyBenchmark.testMethod");
        assert_eq!(b.toolset, "jmh");
        assert_eq!(b.language, "java");
        assert!((b.metrics.mean_ns - 1234.5).abs() < f64::EPSILON);
        assert!((b.metrics.median_ns - 1230.0).abs() < f64::EPSILON);
        assert!((b.metrics.min_ns - 1100.0).abs() < f64::EPSILON);
        assert!((b.metrics.max_ns - 1400.0).abs() < f64::EPSILON);
        assert!((b.metrics.stddev_ns - 50.0).abs() < 0.1);
        assert!(b.metrics.ops_per_sec > 0.0);
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

        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        let benchmarks = parser.parse(json).unwrap();
        let b = &benchmarks[0];
        assert!((b.metrics.mean_ns - 1500.0).abs() < f64::EPSILON);
        assert!((b.metrics.median_ns - 1400.0).abs() < f64::EPSILON);
        assert!((b.metrics.min_ns - 1100.0).abs() < f64::EPSILON);
        assert!((b.metrics.max_ns - 1800.0).abs() < f64::EPSILON);
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

        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        let benchmarks = parser.parse(json).unwrap();
        let b = &benchmarks[0];
        assert!((b.metrics.mean_ns - 2_500_000.0).abs() < f64::EPSILON);
        assert!((b.metrics.median_ns - 2_400_000.0).abs() < f64::EPSILON);
        assert!((b.metrics.min_ns - 2_000_000.0).abs() < f64::EPSILON);
        assert!((b.metrics.max_ns - 3_000_000.0).abs() < f64::EPSILON);
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

        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        let benchmarks = parser.parse(json).unwrap();
        let b = &benchmarks[0];
        assert!((b.metrics.mean_ns - 500_000_000.0).abs() < f64::EPSILON);
        assert!((b.metrics.median_ns - 480_000_000.0).abs() < f64::EPSILON);
        assert!((b.metrics.min_ns - 400_000_000.0).abs() < f64::EPSILON);
        assert!((b.metrics.max_ns - 600_000_000.0).abs() < f64::EPSILON);
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

        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        let benchmarks = parser.parse(json).unwrap();
        assert_eq!(benchmarks.len(), 2);
        assert!((benchmarks[0].metrics.mean_ns - 1000.0).abs() < f64::EPSILON);
        assert!((benchmarks[1].metrics.mean_ns - 2000.0).abs() < f64::EPSILON);
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

        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );

        assert!(parser.can_parse(json));

        let benchmarks = parser.parse(json).unwrap();
        let b = &benchmarks[0];
        assert!((b.metrics.median_ns - 1234.5).abs() < f64::EPSILON);
        let expected_stddev = 50.0;
        assert!((b.metrics.stddev_ns - expected_stddev).abs() < 0.1);
        assert!((b.metrics.min_ns - (1234.5 - expected_stddev)).abs() < 0.1);
        assert!((b.metrics.max_ns - (1234.5 + expected_stddev)).abs() < 0.1);
    }

    #[test]
    fn test_jmh_can_parse_invalid() {
        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );
        assert!(!parser.can_parse(r#"{"not": "jmh"}"#));
    }

    #[test]
    fn test_jmh_can_parse_criterion_format() {
        let parser = JmhParser::new(
            Uuid::new_v4(),
            "test/repo".to_string(),
            "abc123".to_string(),
        );
        assert!(!parser.can_parse(r#"{"id": "my_benchmark", "mean": {"point_estimate": 1234.5}}"#));
    }
}
