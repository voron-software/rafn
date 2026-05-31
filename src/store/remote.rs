//! Remote benchmark store backed entirely by gRPC.

use anyhow::{Context, Result};
use tonic::transport::Channel;

use crate::proto::benchmark::{split_repository, timestamp_to_millis};
use crate::proto::pb::{
    BenchmarkSet, GetBenchmarkTrendRequest, GetCommitBenchmarksRequest, GetRepositoryTrendsRequest,
    PushResultsRequest, RepositoryReference, benchmark_service_client::BenchmarkServiceClient,
};

use super::{Backend, BackendConfig, TrendDataPoint, TrendQuery, require_repository};

const DEFAULT_GRPC_URL: &str = "http://localhost:50051";

#[derive(Clone)]
pub struct RemoteBackend {
    grpc_url: String,
    repository: String,
}

impl RemoteBackend {
    pub fn from_config(config: BackendConfig) -> Result<Self> {
        let grpc_url = resolved_grpc_url(&config);
        let repository = require_repository(&config)?;
        Ok(Self {
            grpc_url,
            repository,
        })
    }

    pub fn for_push(config: BackendConfig) -> Self {
        let grpc_url = resolved_grpc_url(&config);
        Self {
            grpc_url,
            repository: String::new(),
        }
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn grpc_url(&self) -> &str {
        &self.grpc_url
    }

    pub async fn connect_push(&self) -> Result<BenchmarkClient> {
        BenchmarkClient::connect(self.grpc_url.clone()).await
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

fn repository_ref(repository: &str) -> RepositoryReference {
    let (owner, repo) = split_repository(repository);
    RepositoryReference {
        forge: "github.com".to_string(),
        owner,
        repository: repo,
    }
}

impl Backend for RemoteBackend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<BenchmarkSet>> {
        let mut client = BenchmarkServiceClient::connect(self.grpc_url.clone())
            .await
            .context("Failed to connect to gRPC service")?;

        let response = client
            .get_commit_benchmarks(GetCommitBenchmarksRequest {
                repository: Some(repository_ref(&self.repository)),
                commit_sha: commit_sha.to_string(),
                metric_name: Some("wall_time".to_string()),
                benchmark_name: None,
            })
            .await
            .context("gRPC get_commit_benchmarks call failed")?;

        Ok(response.into_inner().benchmark_sets)
    }

    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>> {
        let mut client = BenchmarkServiceClient::connect(self.grpc_url.clone())
            .await
            .context("Failed to connect to gRPC service")?;

        let mut data_points: Vec<TrendDataPoint> = if let Some(name) = query.benchmark_name {
            let response = client
                .get_benchmark_trend(GetBenchmarkTrendRequest {
                    repository: Some(repository_ref(&self.repository)),
                    benchmark_name: name.clone(),
                    metric_name: "wall_time".to_string(),
                    limit: Some(query.limit),
                })
                .await
                .context("gRPC get_benchmark_trend call failed")?;

            response
                .into_inner()
                .trend
                .map(|trend| {
                    trend
                        .points
                        .iter()
                        .map(|p| TrendDataPoint {
                            benchmark_name: trend.benchmark_name.clone(),
                            commit_sha: p.commit_sha.clone(),
                            timestamp: p.timestamp.as_ref().map(timestamp_to_millis).unwrap_or(0),
                            mean_ns: p.statistics.as_ref().and_then(|s| s.mean).unwrap_or(0.0),
                            median_ns: p.statistics.as_ref().and_then(|s| s.median).unwrap_or(0.0),
                            stddev_ns: p.statistics.as_ref().and_then(|s| s.stddev).unwrap_or(0.0),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let response = client
                .get_repository_trends(GetRepositoryTrendsRequest {
                    repository: Some(repository_ref(&self.repository)),
                    metric_name: Some("wall_time".to_string()),
                    limit: Some(query.limit),
                })
                .await
                .context("gRPC get_repository_trends call failed")?;

            response
                .into_inner()
                .trends
                .iter()
                .flat_map(|trend| {
                    trend.points.iter().map(|p| TrendDataPoint {
                        benchmark_name: trend.benchmark_name.clone(),
                        commit_sha: p.commit_sha.clone(),
                        timestamp: p.timestamp.as_ref().map(timestamp_to_millis).unwrap_or(0),
                        mean_ns: p.statistics.as_ref().and_then(|s| s.mean).unwrap_or(0.0),
                        median_ns: p.statistics.as_ref().and_then(|s| s.median).unwrap_or(0.0),
                        stddev_ns: p.statistics.as_ref().and_then(|s| s.stddev).unwrap_or(0.0),
                    })
                })
                .collect()
        };

        data_points.reverse();
        Ok(data_points)
    }
}

pub struct BenchmarkClient {
    inner: BenchmarkServiceClient<Channel>,
}

impl BenchmarkClient {
    pub async fn connect(grpc_url: String) -> Result<Self> {
        let inner = BenchmarkServiceClient::connect(grpc_url)
            .await
            .context("Failed to connect to gRPC service")?;
        Ok(Self { inner })
    }

    pub async fn submit(&mut self, benchmark_sets: Vec<BenchmarkSet>) -> Result<u32> {
        let response = self
            .inner
            .push_results(PushResultsRequest { benchmark_sets })
            .await
            .context("gRPC push_results call failed")?;

        Ok(response.into_inner().benchmarks_pushed)
    }
}
