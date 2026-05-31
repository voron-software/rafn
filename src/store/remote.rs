//! Remote benchmark store backed entirely by gRPC.

use anyhow::{Context, Result};
use tonic::transport::Channel;
use uuid::Uuid;

use crate::proto::Benchmark;
use crate::proto::benchmark::{
    benchmark_from_proto, language_enum, record, timestamp_from_proto, timestamp_to_proto,
    toolset_enum,
};
use crate::proto::pb::{
    BenchmarkSet, CiInformation, GetBenchmarkTrendRequest, GetCommitBenchmarksRequest,
    GetRepositoryTrendsRequest, MachineInformation, PushResultsRequest, RepositoryReference,
    SourceInformation, ToolsetInformation, Unit, benchmark_service_client::BenchmarkServiceClient,
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

pub fn split_repository(repository: &str) -> (String, String) {
    match repository.split_once('/') {
        Some((owner, repo)) => (owner.to_string(), repo.to_string()),
        None => (String::new(), repository.to_string()),
    }
}

impl Backend for RemoteBackend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<Benchmark>> {
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

        let benchmarks = response
            .into_inner()
            .benchmark_sets
            .iter()
            .flat_map(|set| {
                set.benchmarks
                    .iter()
                    .map(|b| benchmark_from_proto(set, b, &self.repository))
            })
            .collect();

        Ok(benchmarks)
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
                            timestamp: p.timestamp.as_ref().map(timestamp_from_proto).unwrap_or(0),
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
                        timestamp: p.timestamp.as_ref().map(timestamp_from_proto).unwrap_or(0),
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

    pub async fn submit(&mut self, benchmarks: Vec<Benchmark>) -> Result<u32> {
        let benchmark_sets = group_into_sets(benchmarks);

        let response = self
            .inner
            .push_results(PushResultsRequest { benchmark_sets })
            .await
            .context("gRPC push_results call failed")?;

        Ok(response.into_inner().benchmarks_pushed)
    }
}

#[derive(Hash, Eq, PartialEq)]
struct GroupKey {
    repository: String,
    commit_sha: String,
    branch: Option<String>,
    tag: Option<String>,
    toolset: String,
    language: String,
    ci_job_id: Option<String>,
    cpu_model: Option<String>,
    os: Option<String>,
}

impl From<&Benchmark> for GroupKey {
    fn from(b: &Benchmark) -> Self {
        Self {
            repository: b.repository.clone(),
            commit_sha: b.commit_sha.clone(),
            branch: b.branch.clone(),
            tag: b.tag.clone(),
            toolset: b.toolset.clone(),
            language: b.language.clone(),
            ci_job_id: b.ci_job_id.clone(),
            cpu_model: b.cpu_model.clone(),
            os: b.os.clone(),
        }
    }
}

fn group_into_sets(benchmarks: Vec<Benchmark>) -> Vec<BenchmarkSet> {
    let mut groups: std::collections::HashMap<GroupKey, Vec<Benchmark>> =
        std::collections::HashMap::new();
    for b in benchmarks {
        groups.entry(GroupKey::from(&b)).or_default().push(b);
    }

    groups
        .into_iter()
        .map(|(key, members)| {
            let (owner, repo) = split_repository(&key.repository);
            BenchmarkSet {
                run_uuid: Uuid::new_v4().to_string(),
                source: Some(SourceInformation {
                    forge: "github.com".to_string(),
                    owner,
                    repository: repo,
                    commit_sha: key.commit_sha.clone(),
                    commit_graph: None,
                    branch: key.branch.clone(),
                    tag: key.tag.clone(),
                    dirty: false,
                }),
                toolset: Some(ToolsetInformation {
                    language: language_enum(&key.language) as i32,
                    language_other: None,
                    language_version: None,
                    toolset: toolset_enum(&key.toolset) as i32,
                    toolset_other: None,
                    toolset_version: None,
                }),
                machine: Some(MachineInformation {
                    cpu_count: 0,
                    cpu_model: key.cpu_model.unwrap_or_default(),
                    operating_system: key.os.unwrap_or_default(),
                    architecture: String::new(),
                }),
                ci: Some(CiInformation {
                    provider: String::new(),
                    job_id: key.ci_job_id.clone(),
                    pull_request_id: None,
                    target_branch: None,
                    build_id: None,
                    run_url: None,
                }),
                metric_name: "wall_time".to_string(),
                unit: Unit::Nanoseconds as i32,
                benchmarks: members.iter().map(record).collect(),
                labels: members
                    .first()
                    .map(|b| b.labels.clone())
                    .unwrap_or_default(),
                run_started_at: members.first().map(|b| timestamp_to_proto(b.timestamp)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::benchmark::{
        benchmark_from_proto, metrics_from_statistics, timestamp_from_proto,
    };
    use crate::proto::pb::{BenchmarkSet, MetricStatistics, SourceInformation};
    use crate::proto::{Benchmark, Metrics};

    fn make_benchmark(name: &str, metrics: Metrics) -> Benchmark {
        Benchmark::builder()
            .tenant_id(Uuid::new_v4())
            .repository("acme/perf-suite".to_string())
            .commit_sha("deadbeef".to_string())
            .benchmark_name(name.to_string())
            .toolset("criterion".to_string())
            .language("rust".to_string())
            .branch("main".to_string())
            .metrics(metrics)
            .build()
            .unwrap()
    }

    #[test]
    fn two_benchmarks_same_metadata_produce_one_set() {
        let m1 = Metrics::new(100.0, 99.0, 5.0, 90.0, 110.0).with_iterations(50);
        let m2 = Metrics::new(200.0, 198.0, 8.0, 185.0, 215.0).with_iterations(30);
        let benchmarks = vec![
            make_benchmark("bench_a", m1.clone()),
            make_benchmark("bench_b", m2.clone()),
        ];

        let sets = group_into_sets(benchmarks);
        assert_eq!(
            sets.len(),
            1,
            "shared metadata must produce exactly one group"
        );

        let pb_benchmarks = &sets[0].benchmarks;
        assert_eq!(pb_benchmarks.len(), 2);

        let stats_a = pb_benchmarks
            .iter()
            .find(|b| b.name == "bench_a")
            .and_then(|b| b.statistics.as_ref())
            .expect("bench_a statistics must be present");
        assert_eq!(stats_a.mean, Some(m1.mean_ns));
        assert_eq!(stats_a.median, Some(m1.median_ns));
        assert_eq!(stats_a.stddev, Some(m1.stddev_ns));
        assert_eq!(stats_a.min, Some(m1.min_ns));
        assert_eq!(stats_a.max, Some(m1.max_ns));
        assert_eq!(stats_a.sample_count, Some(50));
    }

    #[test]
    fn different_commit_sha_produces_separate_sets() {
        let metrics = Metrics::default();
        let b1 = make_benchmark("bench_x", metrics.clone());
        let mut b2 = make_benchmark("bench_x", metrics);
        b2.commit_sha = "cafebabe".to_string();

        let sets = group_into_sets(vec![b1, b2]);
        assert_eq!(sets.len(), 2);
    }

    #[test]
    fn split_repository_handles_no_slash() {
        let (owner, repo) = split_repository("standalone");
        assert_eq!(owner, "");
        assert_eq!(repo, "standalone");
    }

    #[test]
    fn split_repository_splits_owner_repo() {
        let (owner, repo) = split_repository("acme/perf-suite");
        assert_eq!(owner, "acme");
        assert_eq!(repo, "perf-suite");
    }

    #[test]
    fn benchmark_from_proto_maps_metrics() {
        let stats = MetricStatistics {
            mean: Some(100.0),
            median: Some(99.0),
            stddev: Some(5.0),
            min: Some(90.0),
            max: Some(110.0),
            sample_count: Some(50),
            p50: None,
            p90: None,
            p95: None,
            p99: None,
        };
        let pb_bench = crate::proto::pb::Benchmark {
            name: "parse".to_string(),
            location: None,
            parameters: Default::default(),
            samples: vec![],
            statistics: Some(stats),
        };
        let set = BenchmarkSet {
            run_uuid: Uuid::new_v4().to_string(),
            source: Some(SourceInformation {
                forge: "github.com".to_string(),
                owner: "acme".to_string(),
                repository: "perf-suite".to_string(),
                commit_sha: "abc123".to_string(),
                commit_graph: None,
                branch: Some("main".to_string()),
                tag: None,
                dirty: false,
            }),
            ..Default::default()
        };

        let domain = benchmark_from_proto(&set, &pb_bench, "acme/perf-suite");
        assert_eq!(domain.benchmark_name, "parse");
        assert_eq!(domain.commit_sha, "abc123");
        assert_eq!(domain.metrics.mean_ns, 100.0);
        assert_eq!(domain.metrics.median_ns, 99.0);
        assert_eq!(domain.metrics.stddev_ns, 5.0);
        assert_eq!(domain.metrics.iterations, 50);
        assert_eq!(domain.branch, Some("main".to_string()));
    }

    #[test]
    fn trend_point_timestamp_converts_correctly() {
        let ts = prost_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 500_000_000,
        };
        let ms = timestamp_from_proto(&ts);
        assert_eq!(ms, 1_700_000_000 * 1000 + 500);
    }

    #[test]
    fn metrics_from_statistics_defaults_missing_fields() {
        let stats = MetricStatistics {
            mean: Some(42.0),
            median: None,
            stddev: None,
            min: None,
            max: None,
            sample_count: None,
            p50: None,
            p90: None,
            p95: None,
            p99: None,
        };
        let metrics = metrics_from_statistics(&stats);
        assert_eq!(metrics.mean_ns, 42.0);
        assert_eq!(metrics.median_ns, 0.0);
        assert_eq!(metrics.iterations, 0);
    }
}
