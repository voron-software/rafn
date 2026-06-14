//! Benchmark storage backends.
//!
//! Local snapshots are staged under `.rafn/snapshots/`. Both reads and writes
//! for the remote backend use the gRPC service.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tabled::Tabled;

use crate::config::{BackendType, Config, RepoConfig};
use crate::proto::pb::BenchmarkSet;

pub mod local;
pub mod remote;

pub use local::LocalBackend;
pub use remote::RemoteBackend;

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub repo_config: RepoConfig,
    pub user_config: Config,
    pub repo: Option<String>,
}

impl BackendConfig {
    pub fn new(repo_config: RepoConfig, user_config: Config, repo: Option<String>) -> Self {
        Self {
            repo_config,
            user_config,
            repo,
        }
    }
}

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
    #[tabled(rename = "Mean (ms)", display = "format_duration_trend")]
    pub mean_ns: f64,
    #[tabled(rename = "Median (ms)", display = "format_duration_trend")]
    pub median_ns: f64,
    #[tabled(rename = "Std Dev (ms)", display = "format_duration_trend")]
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

    pub fn repository(&self) -> Option<&str> {
        match self {
            Self::Local(_) => None,
            Self::Remote(backend) => Some(backend.repository()),
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

pub fn selected_backend(repo: Option<String>) -> Result<SelectedBackend> {
    let repo_config = RepoConfig::load()?;
    match repo_config.backend.backend_type {
        BackendType::Local => Ok(SelectedBackend::Local(local_backend(&repo_config))),
        BackendType::Cloud => {
            let config = BackendConfig::new(repo_config, Config::load()?, repo);
            Ok(SelectedBackend::Remote(RemoteBackend::from_config(config)?))
        }
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
    RemoteBackend::for_push(BackendConfig::new(
        repo_config,
        Config::load().unwrap_or_default(),
        None,
    ))
}

pub fn format_duration_trend(ns: &f64) -> String {
    format!("{:.3}", ns / 1_000_000.0)
}

pub fn format_timestamp(ts: &i64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp(*ts / 1000, 0)
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

pub(crate) fn require_repository(config: &BackendConfig) -> Result<String> {
    config
        .repo
        .clone()
        .or_else(|| config.user_config.default_repo.clone())
        .context("Repository not specified. Use --repo or configure default_repo")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::benchmark::{benchmark_record, benchmark_set, metric_statistics};

    fn make_set() -> BenchmarkSet {
        benchmark_set(
            "test/repo",
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
}
