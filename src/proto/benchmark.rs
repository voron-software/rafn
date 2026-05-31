use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::pb;
use super::{Error, Metrics, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Benchmark {
    pub tenant_id: Uuid,
    pub repository: String,
    pub commit_sha: String,
    pub benchmark_name: String,
    pub timestamp: DateTime<Utc>,
    pub toolset: String,
    pub language: String,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub ci_job_id: Option<String>,
    pub metrics: Metrics,
    pub custom_metrics: HashMap<String, f64>,
    pub labels: HashMap<String, String>,
    pub cpu_model: Option<String>,
    pub os: Option<String>,
    pub raw_json: Option<String>,
}

impl Benchmark {
    pub fn builder() -> BenchmarkBuilder {
        BenchmarkBuilder::default()
    }
}

pub fn toolset_enum(s: &str) -> pb::Toolset {
    match s {
        "criterion" => pb::Toolset::Criterion,
        "divan" => pb::Toolset::Divan,
        "jmh" => pb::Toolset::Jmh,
        "google_benchmark" => pb::Toolset::GoogleBenchmark,
        "benchmarkdotnet" => pb::Toolset::BenchmarkDotnet,
        "go_test" => pb::Toolset::GoTest,
        "pytest_benchmark" => pb::Toolset::PytestBenchmark,
        "pyperf" => pb::Toolset::Pyperf,
        "vitest_bench" => pb::Toolset::VitestBench,
        "benchmark_js" => pb::Toolset::BenchmarkJs,
        "catch2" => pb::Toolset::Catch2,
        _ => pb::Toolset::Unspecified,
    }
}

pub fn language_enum(s: &str) -> pb::Language {
    match s {
        "rust" => pb::Language::Rust,
        "go" => pb::Language::Go,
        "java" => pb::Language::Java,
        "kotlin" => pb::Language::Kotlin,
        "csharp" => pb::Language::Csharp,
        "fsharp" => pb::Language::Fsharp,
        "cpp" => pb::Language::Cpp,
        "c" => pb::Language::C,
        "python" => pb::Language::Python,
        "javascript" => pb::Language::Javascript,
        "typescript" => pb::Language::Typescript,
        _ => pb::Language::Unspecified,
    }
}

pub fn record(b: &Benchmark) -> pb::Benchmark {
    pb::Benchmark {
        name: b.benchmark_name.clone(),
        location: None,
        parameters: Default::default(),
        samples: vec![],
        statistics: Some(pb::MetricStatistics {
            mean: Some(b.metrics.mean_ns),
            median: Some(b.metrics.median_ns),
            stddev: Some(b.metrics.stddev_ns),
            min: Some(b.metrics.min_ns),
            max: Some(b.metrics.max_ns),
            sample_count: Some(b.metrics.iterations),
            p50: None,
            p90: None,
            p95: None,
            p99: None,
        }),
    }
}

pub fn timestamp_to_proto(dt: DateTime<Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

pub fn timestamp_from_proto(ts: &prost_types::Timestamp) -> i64 {
    ts.seconds * 1000 + i64::from(ts.nanos) / 1_000_000
}

pub fn metrics_from_statistics(stats: &pb::MetricStatistics) -> Metrics {
    Metrics {
        mean_ns: stats.mean.unwrap_or(0.0),
        median_ns: stats.median.unwrap_or(0.0),
        stddev_ns: stats.stddev.unwrap_or(0.0),
        min_ns: stats.min.unwrap_or(0.0),
        max_ns: stats.max.unwrap_or(0.0),
        iterations: stats.sample_count.unwrap_or(0),
        ops_per_sec: 0.0,
    }
}

pub fn benchmark_from_proto(
    set: &pb::BenchmarkSet,
    b: &pb::Benchmark,
    repository: &str,
) -> Benchmark {
    let source = set.source.as_ref();
    Benchmark {
        tenant_id: uuid::Uuid::nil(),
        repository: repository.to_string(),
        commit_sha: source.map(|s| s.commit_sha.clone()).unwrap_or_default(),
        benchmark_name: b.name.clone(),
        timestamp: chrono::Utc::now(),
        toolset: String::new(),
        language: String::new(),
        branch: source.and_then(|s| s.branch.clone()),
        tag: source.and_then(|s| s.tag.clone()),
        ci_job_id: None,
        metrics: b
            .statistics
            .as_ref()
            .map(metrics_from_statistics)
            .unwrap_or_default(),
        custom_metrics: Default::default(),
        labels: Default::default(),
        cpu_model: None,
        os: None,
        raw_json: None,
    }
}

#[derive(Default)]
pub struct BenchmarkBuilder {
    tenant_id: Option<Uuid>,
    repository: Option<String>,
    commit_sha: Option<String>,
    benchmark_name: Option<String>,
    timestamp: Option<DateTime<Utc>>,
    toolset: Option<String>,
    language: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    ci_job_id: Option<String>,
    metrics: Option<Metrics>,
    custom_metrics: HashMap<String, f64>,
    labels: HashMap<String, String>,
    cpu_model: Option<String>,
    os: Option<String>,
    raw_json: Option<String>,
}

impl BenchmarkBuilder {
    pub fn tenant_id(mut self, tenant_id: Uuid) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    pub fn repository(mut self, repository: String) -> Self {
        self.repository = Some(repository);
        self
    }

    pub fn commit_sha(mut self, commit_sha: String) -> Self {
        self.commit_sha = Some(commit_sha);
        self
    }

    pub fn benchmark_name(mut self, benchmark_name: String) -> Self {
        self.benchmark_name = Some(benchmark_name);
        self
    }

    pub fn timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn toolset(mut self, toolset: String) -> Self {
        self.toolset = Some(toolset);
        self
    }

    pub fn language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    pub fn branch(mut self, branch: String) -> Self {
        self.branch = Some(branch);
        self
    }

    pub fn metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn raw_json(mut self, raw_json: String) -> Self {
        self.raw_json = Some(raw_json);
        self
    }

    pub fn build(self) -> Result<Benchmark> {
        Ok(Benchmark {
            tenant_id: self
                .tenant_id
                .ok_or_else(|| Error::Validation("tenant_id is required".to_string()))?,
            repository: self
                .repository
                .ok_or_else(|| Error::Validation("repository is required".to_string()))?,
            commit_sha: self
                .commit_sha
                .ok_or_else(|| Error::Validation("commit_sha is required".to_string()))?,
            benchmark_name: self
                .benchmark_name
                .ok_or_else(|| Error::Validation("benchmark_name is required".to_string()))?,
            timestamp: self.timestamp.unwrap_or_else(Utc::now),
            toolset: self
                .toolset
                .ok_or_else(|| Error::Validation("toolset is required".to_string()))?,
            language: self
                .language
                .ok_or_else(|| Error::Validation("language is required".to_string()))?,
            branch: self.branch,
            tag: self.tag,
            ci_job_id: self.ci_job_id,
            metrics: self
                .metrics
                .ok_or_else(|| Error::Validation("metrics is required".to_string()))?,
            custom_metrics: self.custom_metrics,
            labels: self.labels,
            cpu_model: self.cpu_model,
            os: self.os,
            raw_json: self.raw_json,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_builder() {
        let benchmark = Benchmark::builder()
            .tenant_id(Uuid::new_v4())
            .repository("test/repo".to_string())
            .commit_sha("abc123".to_string())
            .benchmark_name("test_bench".to_string())
            .toolset("criterion".to_string())
            .language("rust".to_string())
            .metrics(Metrics::default())
            .build();

        assert!(benchmark.is_ok());
    }

    #[test]
    fn test_benchmark_builder_missing_fields() {
        let benchmark = Benchmark::builder()
            .repository("test/repo".to_string())
            .build();

        assert!(benchmark.is_err());
    }
}
