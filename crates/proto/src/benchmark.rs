use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::pb;
use crate::{Error, Metrics, Result};

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

impl From<Benchmark> for pb::Benchmark {
    fn from(b: Benchmark) -> Self {
        let toolset = match b.toolset.as_str() {
            "criterion" => pb::Toolset::Criterion as i32,
            "jmh" => pb::Toolset::Jmh as i32,
            "google_benchmark" => pb::Toolset::GoogleBenchmark as i32,
            "benchmarkdotnet" => pb::Toolset::Benchmarkdotnet as i32,
            _ => pb::Toolset::Unspecified as i32,
        };
        let language = match b.language.as_str() {
            "rust" => pb::Language::Rust as i32,
            "java" => pb::Language::Java as i32,
            "cpp" => pb::Language::Cpp as i32,
            "csharp" => pb::Language::Csharp as i32,
            _ => pb::Language::Unspecified as i32,
        };
        pb::Benchmark {
            tenant_id: b.tenant_id.to_string(),
            repository: b.repository,
            commit_sha: b.commit_sha,
            benchmark_name: b.benchmark_name,
            timestamp_ms: b.timestamp.timestamp_millis(),
            toolset,
            language,
            branch: b.branch,
            tag: b.tag,
            ci_job_id: b.ci_job_id,
            metrics: Some(pb::Metrics {
                mean_ns: b.metrics.mean_ns,
                median_ns: b.metrics.median_ns,
                stddev_ns: b.metrics.stddev_ns,
                min_ns: b.metrics.min_ns,
                max_ns: b.metrics.max_ns,
                iterations: b.metrics.iterations,
                ops_per_sec: b.metrics.ops_per_sec,
            }),
            custom_metrics: b.custom_metrics,
            labels: b.labels,
            cpu_model: b.cpu_model,
            os: b.os,
            raw_json: b.raw_json,
        }
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
