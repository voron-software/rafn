//! Remote benchmark store.

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::api;
use crate::proto::{Benchmark, Metrics};

use super::{Backend, BackendConfig, TrendDataPoint, TrendQuery, require_repository};

const DEFAULT_GRPC_URL: &str = "http://localhost:50051";

#[derive(Clone)]
pub struct RemoteBackend {
    api_url: String,
    grpc_url: String,
    repository: String,
    http: reqwest::Client,
}

impl RemoteBackend {
    pub fn from_config(config: BackendConfig) -> Result<Self> {
        let grpc_url = resolved_grpc_url(&config);
        let repository = require_repository(&config)?;
        let api_url = config.api_url.unwrap_or(config.user_config.api_url.clone());

        Ok(Self {
            api_url,
            grpc_url,
            repository,
            http: reqwest::Client::new(),
        })
    }

    pub fn for_push(config: BackendConfig) -> Self {
        let grpc_url = resolved_grpc_url(&config);
        Self {
            api_url: config.api_url.unwrap_or(config.user_config.api_url.clone()),
            grpc_url,
            repository: String::new(),
            http: reqwest::Client::new(),
        }
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn grpc_url(&self) -> &str {
        &self.grpc_url
    }

    pub async fn connect_ingest(&self) -> Result<api::IngestClient> {
        api::IngestClient::connect(self.grpc_url.clone()).await
    }

    pub async fn submit(&self, benchmarks: Vec<Benchmark>) -> Result<u32> {
        let mut client = self.connect_ingest().await?;
        client.submit(benchmarks).await
    }

    fn benchmarks_url(&self, commit_sha: &str) -> String {
        format!(
            "{}/v1/benchmarks?repository={}&commit_sha={}&limit=1000",
            self.api_url,
            urlencoding::encode(&self.repository),
            urlencoding::encode(commit_sha)
        )
    }

    fn trend_url(&self, benchmark_name: &str, limit: u32) -> String {
        format!(
            "{}/v1/benchmarks/trend?repository={}&benchmark_name={}&limit={}",
            self.api_url,
            urlencoding::encode(&self.repository),
            urlencoding::encode(benchmark_name),
            limit
        )
    }
}

fn resolved_grpc_url(config: &BackendConfig) -> String {
    config
        .grpc_url
        .clone()
        .or_else(|| config.repo_config.grpc_url().map(str::to_string))
        .unwrap_or_else(|| {
            if config.user_config.grpc_url != DEFAULT_GRPC_URL {
                config.user_config.grpc_url.clone()
            } else {
                DEFAULT_GRPC_URL.to_string()
            }
        })
}

impl Backend for RemoteBackend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<Benchmark>> {
        let response = self
            .http
            .get(self.benchmarks_url(commit_sha))
            .send()
            .await
            .context("Failed to send request to API")?;

        ensure_success(response)
            .await?
            .benchmarks(&self.repository, commit_sha)
    }

    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>> {
        let name = query.benchmark_name.context(
            "Benchmark name is required for the remote backend. Use --name or switch to backend = \"local\"",
        )?;

        let response = self
            .http
            .get(self.trend_url(&name, query.limit))
            .send()
            .await
            .context("Failed to send request to API")?;

        let mut data_points = ensure_success(response).await?.trend(&name)?;
        data_points.reverse();
        Ok(data_points)
    }
}

async fn ensure_success(response: reqwest::Response) -> Result<RemoteResponse> {
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("API error ({status}): {text}");
    }

    Ok(RemoteResponse(response.json().await?))
}

struct RemoteResponse(serde_json::Value);

impl RemoteResponse {
    fn benchmarks(self, repository: &str, commit_sha: &str) -> Result<Vec<Benchmark>> {
        let summaries: Vec<BenchmarkSummary> = serde_json::from_value(
            self.0
                .get("benchmarks")
                .context("Missing 'benchmarks' field in response")?
                .clone(),
        )?;

        Ok(summaries
            .into_iter()
            .map(|s| Benchmark {
                tenant_id: uuid::Uuid::nil(),
                repository: repository.to_string(),
                commit_sha: commit_sha.to_string(),
                benchmark_name: s.benchmark_name,
                timestamp: chrono::Utc::now(),
                toolset: String::new(),
                language: String::new(),
                branch: None,
                tag: None,
                ci_job_id: None,
                metrics: Metrics {
                    mean_ns: s.mean_ns,
                    median_ns: s.median_ns,
                    stddev_ns: s.stddev_ns,
                    min_ns: 0.0,
                    max_ns: 0.0,
                    iterations: 0,
                    ops_per_sec: 0.0,
                },
                custom_metrics: Default::default(),
                labels: Default::default(),
                cpu_model: None,
                os: None,
                raw_json: None,
            })
            .collect())
    }

    fn trend(self, benchmark_name: &str) -> Result<Vec<TrendDataPoint>> {
        let raw: Vec<RemotePoint> = serde_json::from_value(
            self.0
                .get("data")
                .context("Missing 'data' field in response")?
                .clone(),
        )?;

        Ok(raw
            .into_iter()
            .map(|p| TrendDataPoint {
                benchmark_name: benchmark_name.to_string(),
                commit_sha: p.commit_sha,
                timestamp: p.timestamp,
                mean_ns: p.mean_ns,
                median_ns: p.median_ns,
                stddev_ns: p.stddev_ns,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct BenchmarkSummary {
    benchmark_name: String,
    mean_ns: f64,
    median_ns: f64,
    stddev_ns: f64,
}

#[derive(Debug, Deserialize)]
struct RemotePoint {
    commit_sha: String,
    timestamp: i64,
    mean_ns: f64,
    median_ns: f64,
    stddev_ns: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RepoConfig};

    fn config() -> BackendConfig {
        BackendConfig {
            repo_config: RepoConfig::default(),
            user_config: Config::default(),
            repo: Some("owner/repo".to_string()),
            api_url: Some("http://api.example.com".to_string()),
            grpc_url: Some("http://grpc.example.com:50051".to_string()),
        }
    }

    #[test]
    fn test_url_construction_encodes_query_values() {
        let mut config = config();
        config.repo = Some("owner/repo with spaces".to_string());
        let backend = RemoteBackend::from_config(config).unwrap();

        assert_eq!(
            backend.benchmarks_url("abc 123"),
            "http://api.example.com/v1/benchmarks?repository=owner%2Frepo%20with%20spaces&commit_sha=abc%20123&limit=1000"
        );
        assert_eq!(
            backend.trend_url("bench/name", 10),
            "http://api.example.com/v1/benchmarks/trend?repository=owner%2Frepo%20with%20spaces&benchmark_name=bench%2Fname&limit=10"
        );
    }

    #[test]
    fn test_parse_benchmarks_response() {
        let value = serde_json::json!({
            "benchmarks": [{
                "benchmark_name": "parse",
                "mean_ns": 1.0,
                "median_ns": 2.0,
                "stddev_ns": 3.0
            }]
        });

        let benchmarks = RemoteResponse(value)
            .benchmarks("owner/repo", "abc123")
            .unwrap();

        assert_eq!(benchmarks.len(), 1);
        assert_eq!(benchmarks[0].repository, "owner/repo");
        assert_eq!(benchmarks[0].commit_sha, "abc123");
        assert_eq!(benchmarks[0].benchmark_name, "parse");
        assert_eq!(benchmarks[0].metrics.mean_ns, 1.0);
    }

    #[test]
    fn test_parse_trend_response_injects_benchmark_name() {
        let value = serde_json::json!({
            "data": [{
                "commit_sha": "abc123",
                "timestamp": 42,
                "mean_ns": 1.0,
                "median_ns": 2.0,
                "stddev_ns": 3.0
            }]
        });

        let points = RemoteResponse(value).trend("parse").unwrap();

        assert_eq!(points.len(), 1);
        assert_eq!(points[0].benchmark_name, "parse");
        assert_eq!(points[0].commit_sha, "abc123");
    }
}
