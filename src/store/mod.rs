//! Benchmark storage backends.
//!
//! Local snapshots are staged under `.rafn/snapshots/`. Both reads and writes
//! for the remote backend use the gRPC service.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tabled::Tabled;

use crate::config::{BackendType, Config, EffectiveConfig, RepoConfig, RepositoryRef};
use crate::proto::pb::BenchmarkSet;

pub mod local;
pub mod remote;

pub use local::LocalBackend;
pub use remote::RemoteBackend;

#[derive(Debug, Clone)]
pub struct TrendQuery {
    pub benchmark_name: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, Tabled)]
pub struct TrendDataPoint {
    #[tabled(rename = "Benchmark")]
    pub benchmark_name: String,
    #[tabled(rename = "Commit")]
    pub commit_sha: String,
    #[tabled(rename = "Timestamp", display = "format_timestamp")]
    pub timestamp: i64,
    #[tabled(rename = "Mean", display = "format_duration_trend")]
    pub mean_ns: f64,
    #[tabled(rename = "Median", display = "format_duration_trend")]
    pub median_ns: f64,
    #[tabled(rename = "Std Dev", display = "format_duration_trend")]
    pub stddev_ns: f64,
}

pub(crate) trait Backend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<BenchmarkSet>>;
    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>>;
}

pub enum SelectedBackend {
    Local(LocalBackend),
    Remote(RemoteBackend),
}

impl SelectedBackend {
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    pub fn repository(&self) -> Option<&RepositoryRef> {
        match self {
            Self::Local(_) => None,
            Self::Remote(backend) => backend.repository(),
        }
    }
}

impl Backend for SelectedBackend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<BenchmarkSet>> {
        match self {
            Self::Local(backend) => backend.benchmarks_for_commit(commit_sha).await,
            Self::Remote(backend) => backend.benchmarks_for_commit(commit_sha).await,
        }
    }

    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>> {
        match self {
            Self::Local(backend) => backend.trend(query).await,
            Self::Remote(backend) => backend.trend(query).await,
        }
    }
}

/// Select and build the configured backend by loading `rafn.toml` and the
/// user config and resolving the [`EffectiveConfig`].
pub fn selected_backend() -> Result<SelectedBackend> {
    let repo_config = RepoConfig::load()?;
    let user_config = Config::load()?;
    let effective = EffectiveConfig::resolve(&repo_config, &user_config);

    match effective.backend_type {
        BackendType::Local => Ok(SelectedBackend::Local(local_backend(&repo_config))),
        BackendType::Cloud => Ok(SelectedBackend::Remote(RemoteBackend::from_effective(
            effective,
        )?)),
    }
}

/// Local backend anchored at the discovered project root, so snapshots resolve
/// to the same `.rafn/snapshots` no matter which subdirectory `rafn` runs in.
/// Falls back to the process cwd when no project root was found.
pub fn local_backend(repo_config: &RepoConfig) -> LocalBackend {
    repo_config
        .project_root
        .as_deref()
        .map_or_else(LocalBackend::default, LocalBackend::with_root)
}

pub fn remote_backend_for_push(repo_config: RepoConfig) -> RemoteBackend {
    let user_config = Config::load().unwrap_or_default();
    let effective = EffectiveConfig::resolve(&repo_config, &user_config);
    RemoteBackend::for_push(effective)
}

pub fn format_duration_trend(ns: &f64) -> String {
    crate::comparison::format_duration(ns)
}

pub fn format_timestamp(ts: &i64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp(*ts / 1000, 0)
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// Repository identity for the remote backend: `[project.repository]` from
/// `rafn.toml`, falling back to git auto-detection (both already applied by
/// [`EffectiveConfig::resolve`]).
pub(crate) fn require_repository(effective: &EffectiveConfig) -> Result<RepositoryRef> {
    effective.repository.clone().context(
        "Repository not specified. Set [project.repository] in rafn.toml, \
         or run inside a git repository with a configured remote.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::benchmark::{benchmark_record, benchmark_set, metric_statistics};

    fn test_repository() -> RepositoryRef {
        RepositoryRef {
            forge: "github.com".to_string(),
            owner: "test".to_string(),
            repository: "repo".to_string(),
        }
    }

    fn make_set() -> BenchmarkSet {
        benchmark_set(
            &test_repository(),
            "abc123",
            None,
            "run-1".to_string(),
            prost_types::Timestamp::default(),
            "rust",
            "criterion",
            vec![benchmark_record(
                "foo".to_string(),
                metric_statistics(1.0, 0.0, 0.0, 0.0, 0.0, None),
            )],
        )
    }

    #[test]
    fn local_backend_anchors_to_project_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo_config = RepoConfig {
            project_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        local_backend(&repo_config)
            .save("abc123", &[make_set()])
            .unwrap();

        // Snapshot lands under the discovered root, not the process cwd.
        assert!(dir.path().join(".rafn/snapshots/abc123.pb").exists());
    }

    fn effective_with_repository(repository: Option<RepositoryRef>) -> EffectiveConfig {
        EffectiveConfig {
            backend_type: BackendType::Cloud,
            endpoint: "http://localhost:50051".to_string(),
            repository,
            bench_threshold: 5.0,
        }
    }

    #[test]
    fn require_repository_returns_resolved_repository() {
        let effective = effective_with_repository(Some(test_repository()));
        assert_eq!(require_repository(&effective).unwrap(), test_repository());
    }

    #[test]
    fn require_repository_errors_when_unset() {
        let effective = effective_with_repository(None);
        assert!(require_repository(&effective).is_err());
    }

    #[test]
    fn format_duration_trend_delegates_to_adaptive_units() {
        assert_eq!(format_duration_trend(&500.0), "500.000 ns");
        assert_eq!(format_duration_trend(&2_338.0), "2.338 µs");
    }
}
