//! Effective configuration: repo-level (`rafn.toml`) merged over user-level
//! (`~/.config/rafn/config.toml`) merged over compiled defaults, plus git
//! auto-detection for the repository identity.

use super::{BackendType, Config, RepoConfig, RepositoryRef};
use crate::git;

/// Resolved configuration values after applying the precedence
/// repo-level > user-level > compiled defaults (and, for the repository
/// identity, git auto-detection as the final fallback).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    pub backend_type: BackendType,
    pub endpoint: String,
    pub repository: Option<RepositoryRef>,
    pub bench_threshold: f64,
    pub lock_enabled: bool,
    pub lock_timeout_secs: u64,
}

impl EffectiveConfig {
    /// Resolve repo-level config over user-level config, with the
    /// repository identity falling back to git auto-detection when absent
    /// from `rafn.toml`.
    pub fn resolve(repo: &RepoConfig, user: &Config) -> Self {
        Self::resolve_with_git_repository(repo, user, git::detect_repository())
    }

    /// Same as [`resolve`](Self::resolve), but with the git-detected
    /// repository injected rather than shelled out to `git`. Used by tests
    /// to exercise the fallback without depending on a real repository.
    fn resolve_with_git_repository(
        repo: &RepoConfig,
        user: &Config,
        git_repository: Option<RepositoryRef>,
    ) -> Self {
        Self {
            backend_type: repo.backend.backend_type,
            endpoint: repo
                .cloud_api_url()
                .map(str::to_string)
                .unwrap_or_else(|| user.cloud.api_url.clone()),
            repository: repo.project_repository().cloned().or(git_repository),
            bench_threshold: repo.bench_threshold(),
            lock_enabled: repo.lock_enabled(),
            lock_timeout_secs: repo.lock_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CloudConfig, ProjectConfig, UserCloudConfig};

    fn repository_ref(owner: &str, repository: &str) -> RepositoryRef {
        RepositoryRef {
            forge: "github.com".to_string(),
            owner: owner.to_string(),
            repository: repository.to_string(),
        }
    }

    #[test]
    fn repo_cloud_api_url_overrides_user() {
        let mut repo = RepoConfig::default();
        repo.backend.cloud = Some(CloudConfig {
            api_url: Some("https://repo.example.com".to_string()),
        });
        let user = Config {
            cloud: UserCloudConfig {
                api_url: "https://user.example.com".to_string(),
            },
        };

        let effective = EffectiveConfig::resolve_with_git_repository(&repo, &user, None);
        assert_eq!(effective.endpoint, "https://repo.example.com");
    }

    #[test]
    fn falls_back_to_user_level_when_repo_absent() {
        let repo = RepoConfig::default();
        let user = Config {
            cloud: UserCloudConfig {
                api_url: "https://user.example.com".to_string(),
            },
        };

        let effective = EffectiveConfig::resolve_with_git_repository(&repo, &user, None);
        assert_eq!(effective.endpoint, "https://user.example.com");
    }

    #[test]
    fn falls_back_to_compiled_default_when_both_absent() {
        let repo = RepoConfig::default();
        let user = Config::default();

        let effective = EffectiveConfig::resolve_with_git_repository(&repo, &user, None);
        assert_eq!(effective.endpoint, "http://localhost:50051");
    }

    #[test]
    fn backend_type_comes_from_repo_config() {
        let mut repo = RepoConfig::default();
        repo.backend.backend_type = BackendType::Local;
        let user = Config::default();

        let effective = EffectiveConfig::resolve_with_git_repository(&repo, &user, None);
        assert_eq!(effective.backend_type, BackendType::Local);
    }

    #[test]
    fn bench_threshold_comes_from_repo_config() {
        let repo = RepoConfig {
            bench: Some(crate::config::BenchConfig {
                threshold: Some(12.5),
                ..Default::default()
            }),
            ..Default::default()
        };
        let user = Config::default();

        let effective = EffectiveConfig::resolve_with_git_repository(&repo, &user, None);
        assert_eq!(effective.bench_threshold, 12.5);
    }

    #[test]
    fn repository_comes_from_project_config_when_set() {
        let repo = RepoConfig {
            project: Some(ProjectConfig {
                repository: Some(repository_ref("acme", "from-config")),
            }),
            ..Default::default()
        };
        let user = Config::default();

        let effective = EffectiveConfig::resolve_with_git_repository(
            &repo,
            &user,
            Some(repository_ref("acme", "from-git")),
        );
        assert_eq!(
            effective.repository,
            Some(repository_ref("acme", "from-config"))
        );
    }

    #[test]
    fn repository_falls_back_to_git_detection_when_project_config_absent() {
        let repo = RepoConfig::default();
        let user = Config::default();

        let effective = EffectiveConfig::resolve_with_git_repository(
            &repo,
            &user,
            Some(repository_ref("acme", "from-git")),
        );
        assert_eq!(
            effective.repository,
            Some(repository_ref("acme", "from-git"))
        );
    }

    #[test]
    fn repository_is_none_when_project_config_and_git_both_absent() {
        let repo = RepoConfig::default();
        let user = Config::default();

        let effective = EffectiveConfig::resolve_with_git_repository(&repo, &user, None);
        assert_eq!(effective.repository, None);
    }
}
