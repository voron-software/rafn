//! Repository-local configuration loaded from `rafn.toml`.
//!
//! Searched from the current directory upward so that `rafn` can be run
//! from any subdirectory of a project.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Which storage backend to use for snapshot reads and writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Store and read snapshots from the local `.rafn/` directory.
    Local,
    /// Push snapshots to and read history from the remote gRPC service.
    #[default]
    Remote,
}

/// `[remote.cloud]` section — remote service endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteCloud {
    /// gRPC server URL for `rafn push`.
    pub url: Option<String>,
}

/// `[remote]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Remote {
    pub cloud: Option<RemoteCloud>,
}

/// `[bench]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchConfig {
    /// Regression threshold percentage. A benchmark must be slower by more
    /// than this percentage before `rafn bench` treats it as a regression.
    pub threshold: Option<f64>,
}

/// Repo-level configuration from `rafn.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoConfig {
    #[serde(default)]
    pub backend: Backend,

    pub remote: Option<Remote>,

    pub bench: Option<BenchConfig>,

    /// Directory containing the discovered `rafn.toml`. Resolved once during
    /// [`load`](Self::load) so that snapshot storage and other consumers anchor
    /// to the project root rather than the process cwd.
    #[serde(skip)]
    pub project_root: Option<PathBuf>,
}

impl RepoConfig {
    /// Load `rafn.toml` by walking from `cwd` up toward the filesystem root.
    /// Returns `RepoConfig::default()` when no file is found.
    pub fn load() -> Result<Self> {
        let Some(path) = find_rafn_toml(std::env::current_dir()?.as_path()) else {
            return Ok(Self::default());
        };
        let mut config = Self::load_from(&path)?;
        config.project_root = path.parent().map(Path::to_path_buf);
        Ok(config)
    }

    fn load_from(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", path.display()))?;
        Ok(config)
    }

    /// gRPC server URL from `[remote.cloud].url`, if set.
    pub fn grpc_url(&self) -> Option<&str> {
        self.remote
            .as_ref()
            .and_then(|r| r.cloud.as_ref())
            .and_then(|c| c.url.as_deref())
    }

    /// Regression threshold from `[bench].threshold`, defaulting to 5 %.
    pub fn bench_threshold(&self) -> f64 {
        self.bench.as_ref().and_then(|b| b.threshold).unwrap_or(5.0)
    }
}

fn find_rafn_toml(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        let candidate = dir.join("rafn.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_remote() {
        let cfg = RepoConfig::default();
        assert_eq!(cfg.backend, Backend::Remote);
        assert!(cfg.grpc_url().is_none());
    }

    #[test]
    fn test_parse_local_backend() {
        let toml = r#"backend = "local""#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.backend, Backend::Local);
    }

    #[test]
    fn test_parse_grpc_url() {
        let toml = r#"
backend = "remote"
[remote.cloud]
url = "http://grpc.example.com:50051"
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.grpc_url(), Some("http://grpc.example.com:50051"));
    }

    #[test]
    fn test_parse_bench_threshold() {
        let toml = r#"
[bench]
threshold = 10.0
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.bench_threshold(), 10.0);
    }

    #[test]
    fn test_default_bench_threshold() {
        let cfg = RepoConfig::default();
        assert_eq!(cfg.bench_threshold(), 5.0);
    }

    #[test]
    fn test_missing_optional_sections() {
        let toml = r#"backend = "local""#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert!(cfg.grpc_url().is_none());
        assert_eq!(cfg.bench_threshold(), 5.0);
    }
}
