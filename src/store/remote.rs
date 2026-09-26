//! Remote benchmark store backed entirely by gRPC.

use anyhow::{Context, Result};
use tonic::transport::Channel;

use crate::comparison;
use crate::config::{EffectiveConfig, RepositoryRef};
use crate::proto::benchmark::timestamp_to_millis;
use crate::proto::pb::{
    BenchmarkComparison, BenchmarkSet, CompareCommitsRequest, CompareCommitsResponse,
    ComparisonSummary, GetBenchmarkTrendRequest, GetCommitBenchmarksRequest,
    GetRepositoryTrendsRequest, PushResultsRequest, RepositoryReference, SeriesKey, Verdict,
    benchmark_comparison, benchmark_service_client::BenchmarkServiceClient,
};

use super::{Backend, TrendDataPoint, TrendQuery, require_repository};

#[derive(Clone, Debug)]
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
            .with_context(|| format!("Failed to connect to gRPC service at {}", self.endpoint))?;

        let response = client
            .get_commit_benchmarks(GetCommitBenchmarksRequest {
                repository: repository_ref(self.repository.as_ref()),
                commit_sha: commit_sha.to_string(),
                metric_name: Some("wall_time".to_string()),
                benchmark_name: None,
            })
            .await
            .with_context(|| {
                format!(
                    "gRPC get_commit_benchmarks call to {} failed",
                    self.endpoint
                )
            })?;

        Ok(response.into_inner().benchmark_sets)
    }

    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>> {
        let mut client = BenchmarkServiceClient::connect(self.endpoint.clone())
            .await
            .with_context(|| format!("Failed to connect to gRPC service at {}", self.endpoint))?;

        let mut data_points: Vec<TrendDataPoint> = if let Some(name) = query.benchmark_name {
            let response = client
                .get_benchmark_trend(GetBenchmarkTrendRequest {
                    repository: repository_ref(self.repository.as_ref()),
                    benchmark_name: name.clone(),
                    metric_name: "wall_time".to_string(),
                    limit: Some(query.limit),
                    // Not yet exposed as CLI flags; `None` keeps the
                    // all-bindings/all-branches behavior `rafn trend` has today.
                    parameters_json: None,
                    branch: None,
                })
                .await
                .with_context(|| {
                    format!("gRPC get_benchmark_trend call to {} failed", self.endpoint)
                })?;

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
                .with_context(|| {
                    format!(
                        "gRPC get_repository_trends call to {} failed",
                        self.endpoint
                    )
                })?;

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

    async fn compare_commits(
        &self,
        base: &str,
        head: &str,
        threshold_pct: f64,
    ) -> Result<comparison::Report> {
        let mut client = BenchmarkServiceClient::connect(self.endpoint.clone())
            .await
            .with_context(|| format!("Failed to connect to gRPC service at {}", self.endpoint))?;

        let response = client
            .compare_commits(CompareCommitsRequest {
                repository: repository_ref(self.repository.as_ref()),
                base_commit_sha: base.to_string(),
                head_commit_sha: head.to_string(),
                regression_threshold_pct: Some(threshold_pct),
            })
            .await
            .with_context(|| format!("gRPC compare_commits call to {} failed", self.endpoint))?;

        report_from_proto(response.into_inner())
    }
}

/// Map a `CompareCommitsResponse` back into the CLI's local [`comparison::Report`],
/// the reverse of rafn-backend's `proto_comparison`/`proto_summary` mapping
/// (`crates/api/src/grpc/benchmark_service.rs`).
fn report_from_proto(response: CompareCommitsResponse) -> Result<comparison::Report> {
    let comparisons = response
        .comparisons
        .into_iter()
        .map(comparison_from_proto)
        .collect::<Result<Vec<_>>>()?;
    let summary = response
        .summary
        .context("compare_commits response is missing its summary")?;

    Ok(comparison::Report {
        comparisons,
        summary: summary_from_proto(summary),
    })
}

fn comparison_from_proto(comparison: BenchmarkComparison) -> Result<comparison::Comparison> {
    let key = comparison
        .key
        .context("benchmark comparison is missing its key")?;
    let outcome = comparison
        .outcome
        .context("benchmark comparison is missing its outcome")?;

    Ok(comparison::Comparison {
        key: series_key_from_proto(key),
        outcome: outcome_from_proto(outcome),
    })
}

fn series_key_from_proto(key: SeriesKey) -> comparison::SeriesKey {
    comparison::SeriesKey {
        benchmark_name: key.benchmark_name,
        metric_name: key.metric_name,
        parameters_json: key.parameters_json,
        branch: key.branch,
    }
}

fn verdict_from_proto(verdict: i32) -> comparison::Verdict {
    match Verdict::try_from(verdict).unwrap_or(Verdict::Unchanged) {
        Verdict::Regressed => comparison::Verdict::Regressed,
        Verdict::Improved => comparison::Verdict::Improved,
        Verdict::Unchanged | Verdict::Unspecified => comparison::Verdict::Unchanged,
    }
}

fn outcome_from_proto(outcome: benchmark_comparison::Outcome) -> comparison::Outcome {
    match outcome {
        benchmark_comparison::Outcome::Compared(delta) => comparison::Outcome::Compared {
            unit: comparison::unit_name(delta.unit),
            base_mean: delta.base_mean,
            head_mean: delta.head_mean,
            diff: delta.diff,
            diff_pct: delta.diff_pct,
            verdict: verdict_from_proto(delta.verdict),
        },
        benchmark_comparison::Outcome::Added(point) => comparison::Outcome::Added {
            unit: comparison::unit_name(point.unit),
            head_mean: point.mean,
        },
        benchmark_comparison::Outcome::Removed(point) => comparison::Outcome::Removed {
            unit: comparison::unit_name(point.unit),
            base_mean: point.mean,
        },
        benchmark_comparison::Outcome::Incomparable(mismatch) => {
            comparison::Outcome::Incomparable {
                base_unit: comparison::unit_name(mismatch.base_unit),
                head_unit: comparison::unit_name(mismatch.head_unit),
            }
        }
        benchmark_comparison::Outcome::Invalid(invalid) => comparison::Outcome::Invalid {
            reason: invalid.reason,
        },
    }
}

fn summary_from_proto(summary: ComparisonSummary) -> comparison::Summary {
    comparison::Summary {
        regressed: summary.regressed,
        improved: summary.improved,
        unchanged: summary.unchanged,
        added: summary.added,
        removed: summary.removed,
        unassessable: summary.unassessable,
        threshold_pct: summary.threshold_pct,
        has_regressions: summary.has_regressions,
    }
}

#[derive(Debug)]
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

#[cfg(test)]
mod tests {
    use tonic::Status;

    fn wrapped_status_error() -> anyhow::Error {
        let status = Status::internal("ClickHouse: table default.benchmarks does not exist");
        anyhow::Error::new(status).context("gRPC push_results call failed")
    }

    #[test]
    fn test_status_context_hides_detail_under_default_display() {
        let short = format!("{}", wrapped_status_error());
        assert_eq!(short, "gRPC push_results call failed");
        assert!(
            !short.contains("ClickHouse"),
            "default Display should not leak the wrapped status: {short}"
        );
    }

    #[test]
    fn test_status_context_reveals_code_and_message_under_alternate_display() {
        let full = format!("{:#}", wrapped_status_error());
        assert!(full.contains("gRPC push_results call failed"));
        assert!(full.contains("Internal"));
        assert!(full.contains("ClickHouse: table default.benchmarks does not exist"));
    }
}
