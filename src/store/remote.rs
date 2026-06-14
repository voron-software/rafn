//! Remote benchmark store backed entirely by gRPC.

use anyhow::{Context, Result};
use tonic::transport::Channel;

use crate::config::{EffectiveConfig, RepositoryRef};
use crate::proto::benchmark::timestamp_to_millis;
use crate::proto::pb::{
    BenchmarkSet, GetBenchmarkTrendRequest, GetCommitBenchmarksRequest, GetRepositoryTrendsRequest,
    PushResultsRequest, RepositoryReference, benchmark_service_client::BenchmarkServiceClient,
};

use super::{Backend, TrendDataPoint, TrendQuery, require_repository};

#[derive(Clone)]
pub struct RemoteBackend {
    endpoint: String,
    repository: Option<RepositoryRef>,
}

impl RemoteBackend {
    /// Build a backend for read operations (trend/compare), which require a
    /// resolved repository identity.
    pub fn from_effective(effective: EffectiveConfig) -> Result<Self> {
        let repository = require_repository(&effective)?;
        Ok(Self {
            endpoint: effective.endpoint,
            repository: Some(repository),
        })
    }

    /// Build a backend for `rafn push`, which reads repository identity from
    /// each snapshot's `SourceInformation` rather than from config.
    pub fn for_push(effective: EffectiveConfig) -> Self {
        Self {
            endpoint: effective.endpoint,
            repository: None,
        }
    }

    pub fn repository(&self) -> Option<&RepositoryRef> {
        self.repository.as_ref()
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn connect_push(&self) -> Result<BenchmarkClient> {
        BenchmarkClient::connect(self.endpoint.clone()).await
    }
}

fn repository_ref(repository: Option<&RepositoryRef>) -> Option<RepositoryReference> {
    repository.map(RepositoryRef::to_proto)
}

impl Backend for RemoteBackend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<BenchmarkSet>> {
        let mut client = BenchmarkServiceClient::connect(self.endpoint.clone())
            .await
            .context("Failed to connect to gRPC service")?;

        let response = client
            .get_commit_benchmarks(GetCommitBenchmarksRequest {
                repository: repository_ref(self.repository.as_ref()),
                commit_sha: commit_sha.to_string(),
                metric_name: Some("wall_time".to_string()),
                benchmark_name: None,
            })
            .await
            .context("gRPC get_commit_benchmarks call failed")?;

        Ok(response.into_inner().benchmark_sets)
    }

    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>> {
        let mut client = BenchmarkServiceClient::connect(self.endpoint.clone())
            .await
            .context("Failed to connect to gRPC service")?;

        let mut data_points: Vec<TrendDataPoint> = if let Some(name) = query.benchmark_name {
            let response = client
                .get_benchmark_trend(GetBenchmarkTrendRequest {
                    repository: repository_ref(self.repository.as_ref()),
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
                    repository: repository_ref(self.repository.as_ref()),
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
    pub async fn connect(endpoint: String) -> Result<Self> {
        let inner = BenchmarkServiceClient::connect(endpoint)
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
