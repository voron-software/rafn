//! Benchmark storage backends.
//!
//! Local snapshots are staged under `.rafn/snapshots/`. Remote reads use the
//! HTTP API, while remote writes for `rafn push` use the gRPC ingest service.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tabled::Tabled;

use crate::config::{Backend as ConfigBackend, Config, RepoConfig};
use crate::proto::Benchmark;

pub mod local;
pub mod remote;

pub use local::LocalBackend;
pub use remote::RemoteBackend;

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub repo_config: RepoConfig,
    pub user_config: Config,
    pub repo: Option<String>,
    pub api_url: Option<String>,
    pub grpc_url: Option<String>,
}

impl BackendConfig {
    pub fn new(
        repo_config: RepoConfig,
        user_config: Config,
        repo: Option<String>,
        api_url: Option<String>,
        grpc_url: Option<String>,
    ) -> Self {
        Self {
            repo_config,
            user_config,
            repo,
            api_url,
            grpc_url,
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
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<Benchmark>>;
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
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<Benchmark>> {
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

pub fn selected_backend(repo: Option<String>, api_url: Option<String>) -> Result<SelectedBackend> {
    let repo_config = RepoConfig::load()?;
    match repo_config.backend {
        ConfigBackend::Local => Ok(SelectedBackend::Local(LocalBackend::default())),
        ConfigBackend::Remote => {
            let config = BackendConfig::new(repo_config, Config::load()?, repo, api_url, None);
            Ok(SelectedBackend::Remote(RemoteBackend::from_config(config)?))
        }
    }
}

pub fn remote_backend_for_push(repo_config: RepoConfig, grpc_url: Option<String>) -> RemoteBackend {
    RemoteBackend::for_push(BackendConfig::new(
        repo_config,
        Config::load().unwrap_or_default(),
        None,
        None,
        grpc_url,
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
