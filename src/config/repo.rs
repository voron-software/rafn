//! Repository-local configuration loaded from `rafn.toml`.
//!
//! `rafn.toml` is discovered by walking up from the current directory toward
//! the filesystem root, stopping at the first match — the same pattern used
//! by `.gitignore`, `Cargo.toml`, `pyproject.toml`, etc. No git repository is
//! required for discovery.
//!
//! Loaded via the `config` crate so values layer file < env
//! (`RAFN_BACKEND__CLOUD__API_URL`, `RAFN_PROJECT__REPOSITORY__OWNER`, etc.),
//! with compiled defaults as the final fallback.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::RepositoryRef;

/// Which storage backend to use for snapshot reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    /// Store and read snapshots from the local `.rafn/` directory.
    Local,
    /// Push snapshots to and read history from the rafn cloud service.
    #[default]
    Cloud,
}

/// `[backend.cloud]` section — rafn cloud service endpoint.
///
/// `api_key` is intentionally not a field here: it is always sourced from the
/// `RAFN_API_KEY` environment variable, never from `rafn.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloudConfig {
    /// Base URL of the rafn cloud API.
    pub api_url: Option<String>,
}

/// `[backend]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendSection {
    #[serde(rename = "type", default)]
    pub backend_type: BackendType,

    pub cloud: Option<CloudConfig>,
}

/// `[project]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    /// Repository identity, used to resolve the remote backend's
    /// `RepositoryReference`/`SourceInformation`.
    pub repository: Option<RepositoryRef>,
}

/// `[bench]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchConfig {
    /// Regression threshold percentage. A benchmark must be slower by more
    /// than this percentage before `rafn bench` treats it as a regression.
    pub threshold: Option<f64>,

    /// When set, `rafn bench` acquires a machine-global lock so only one
    /// benchmark run executes at a time on the host. Off by default.
    pub enable_lock: Option<bool>,

    /// Seconds to wait for the machine-global lock before failing fatally.
    pub lock_timeout: Option<u64>,
}

/// Repo-level configuration from `rafn.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoConfig {
    #[serde(default)]
    pub backend: BackendSection,

    pub project: Option<ProjectConfig>,

    pub bench: Option<BenchConfig>,

    /// Directory that anchors snapshot storage and other repo-relative
    /// state. Resolved once during [`load`](Self::load): the directory
    /// containing the discovered `rafn.toml`, falling back to the git
    /// repository root, falling back to the current directory.
    #[serde(skip)]
    pub project_root: Option<PathBuf>,
}

impl RepoConfig {
    /// Load `rafn.toml` by walking from `cwd` toward the filesystem root,
    /// stopping at the first match, layered with `RAFN_`-prefixed
    /// environment variables. Falls back to compiled defaults when no
    /// `rafn.toml` is found.
    pub fn load() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let found = find_rafn_toml(&cwd);

        let mut config = Self::build(found.as_deref())?;

        config.project_root = found
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(crate::git::detect_git_root)
            .or(Some(cwd));

        Ok(config)
    }

    /// Build the layered config from an optional `rafn.toml` path plus the
    /// given environment source.
    fn build_with_env(toml_path: Option<&Path>, env: config::Environment) -> Result<Self> {
        let mut builder = config::Config::builder();
        if let Some(path) = toml_path {
            builder = builder.add_source(config::File::from(path).required(false));
        }
        builder = builder.add_source(env);

        builder
            .build()
            .context("Failed to build configuration")?
            .try_deserialize()
            .context("Failed to parse configuration")
    }

    /// Build the layered config from an optional `rafn.toml` path plus
    /// `RAFN_`-prefixed environment variables (`__` as the nesting
    /// separator).
    fn build(toml_path: Option<&Path>) -> Result<Self> {
        Self::build_with_env(
            toml_path,
            config::Environment::with_prefix("RAFN")
                .prefix_separator("_")
                .separator("__"),
        )
    }

    /// Base URL from `[backend.cloud].api_url`, if set.
    pub fn cloud_api_url(&self) -> Option<&str> {
        self.backend
            .cloud
            .as_ref()
            .and_then(|c| c.api_url.as_deref())
    }

    /// Repository identity from `[project.repository]`, if set.
    pub fn project_repository(&self) -> Option<&RepositoryRef> {
        self.project.as_ref().and_then(|p| p.repository.as_ref())
    }

    /// Regression threshold from `[bench].threshold`, defaulting to 5 %.
    pub fn bench_threshold(&self) -> f64 {
        self.bench.as_ref().and_then(|b| b.threshold).unwrap_or(5.0)
    }

    /// Whether the machine-global benchmark lock is enabled, from
    /// `[bench].enable_lock`. Off by default.
    pub fn lock_enabled(&self) -> bool {
        self.bench
            .as_ref()
            .and_then(|b| b.enable_lock)
            .unwrap_or(false)
    }

    /// Seconds to wait for the machine-global lock, from
    /// `[bench].lock_timeout`, defaulting to 600 (10 minutes).
    pub fn lock_timeout_secs(&self) -> u64 {
        self.bench
            .as_ref()
            .and_then(|b| b.lock_timeout)
            .unwrap_or(600)
    }
}

/// Walk up from `start` looking for `rafn.toml`, stopping at the first match.
/// Returns `None` once the filesystem root is reached without a match.
fn find_rafn_toml(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        let candidate = dir.join("rafn.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_rafn_toml_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("rafn.toml"), "").unwrap();
        assert_eq!(find_rafn_toml(root), Some(root.join("rafn.toml")));
    }

    #[test]
    fn test_find_rafn_toml_in_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        fs::write(root.join("rafn.toml"), "").unwrap();
        assert_eq!(find_rafn_toml(&sub), Some(root.join("rafn.toml")));
    }

    #[test]
    fn test_find_rafn_toml_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        // No rafn.toml anywhere in `dir`'s ancestry (assumed for a fresh tempdir).
        assert_eq!(find_rafn_toml(&sub), None);
    }

    #[test]
    fn test_default_is_cloud() {
        let cfg = RepoConfig::default();
        assert_eq!(cfg.backend.backend_type, BackendType::Cloud);
        assert!(cfg.cloud_api_url().is_none());
    }

    #[test]
    fn test_parse_local_backend() {
        let toml = r#"
[backend]
type = "local"
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.backend.backend_type, BackendType::Local);
    }

    #[test]
    fn test_parse_cloud_api_url() {
        let toml = r#"
[backend]
type = "cloud"

[backend.cloud]
api_url = "https://api.rafn.dev"
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.backend.backend_type, BackendType::Cloud);
        assert_eq!(cfg.cloud_api_url(), Some("https://api.rafn.dev"));
    }

    #[test]
    fn test_parse_project_repository() {
        let toml = r#"
[project.repository]
owner = "acme"
repository = "perf-suite"
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            cfg.project_repository(),
            Some(&RepositoryRef {
                forge: "github.com".to_string(),
                owner: "acme".to_string(),
                repository: "perf-suite".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_project_repository_with_forge() {
        let toml = r#"
[project.repository]
forge = "gitlab.com"
owner = "acme"
repository = "perf-suite"
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            cfg.project_repository().map(|r| r.forge.as_str()),
            Some("gitlab.com")
        );
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
    fn test_parse_bench_lock() {
        let toml = r#"
[bench]
enable_lock = true
lock_timeout = 120
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert!(cfg.lock_enabled());
        assert_eq!(cfg.lock_timeout_secs(), 120);
    }

    #[test]
    fn test_default_bench_lock() {
        let cfg = RepoConfig::default();
        assert!(!cfg.lock_enabled());
        assert_eq!(cfg.lock_timeout_secs(), 600);
    }

    #[test]
    fn test_missing_optional_sections() {
        let toml = r#"
[backend]
type = "local"
"#;
        let cfg: RepoConfig = toml::from_str(toml).unwrap();
        assert!(cfg.cloud_api_url().is_none());
        assert!(cfg.project_repository().is_none());
        assert_eq!(cfg.bench_threshold(), 5.0);
    }

    #[test]
    fn test_invalid_backend_type_rejected() {
        let toml = r#"
[backend]
type = "remote"
"#;
        let result: std::result::Result<RepoConfig, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_with_no_file_uses_defaults() {
        let cfg = RepoConfig::build(None).unwrap();
        assert_eq!(cfg.backend.backend_type, BackendType::Cloud);
        assert!(cfg.project_repository().is_none());
    }

    #[test]
    fn test_build_reads_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn.toml");
        fs::write(
            &path,
            r#"
[backend]
type = "local"

[project.repository]
owner = "acme"
repository = "perf-suite"
"#,
        )
        .unwrap();

        let cfg = RepoConfig::build(Some(&path)).unwrap();
        assert_eq!(cfg.backend.backend_type, BackendType::Local);
        assert_eq!(
            cfg.project_repository().map(|r| r.owner.as_str()),
            Some("acme")
        );
    }

    #[test]
    fn test_env_override_for_project_repository_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn.toml");
        fs::write(
            &path,
            r#"
[project.repository]
owner = "from-file"
repository = "perf-suite"
"#,
        )
        .unwrap();

        // Inject a fake environment rather than mutating real process env
        // vars, so this test can't race with others reading RAFN_* vars.
        let mut env = config::Map::new();
        env.insert(
            "RAFN_PROJECT__REPOSITORY__OWNER".to_string(),
            "from-env".to_string(),
        );
        let env_source = config::Environment::with_prefix("RAFN")
            .prefix_separator("_")
            .separator("__")
            .source(Some(env));

        let cfg = RepoConfig::build_with_env(Some(&path), env_source).unwrap();
        assert_eq!(
            cfg.project_repository().map(|r| r.owner.as_str()),
            Some("from-env")
        );
    }
}
