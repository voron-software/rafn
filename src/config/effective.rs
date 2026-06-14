//! Effective configuration: repo-level (`rafn.toml`) merged over user-level
//! (`~/.config/rafn/config.toml`) merged over compiled defaults.

use super::{BackendType, Config, RepoConfig};

/// Resolved configuration values after applying the precedence
/// repo-level > user-level > compiled defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    pub backend_type: BackendType,
    pub cloud_api_url: String,
}

impl EffectiveConfig {
    /// Merge repo-level config over user-level config. Fields absent from
    /// `repo` fall through to `user`, whose own defaults (e.g. `grpc_url`)
    /// serve as the compiled defaults.
    pub fn merge(repo: &RepoConfig, user: &Config) -> Self {
        Self {
            backend_type: repo.backend.backend_type,
            cloud_api_url: repo
                .cloud_api_url()
                .map(str::to_string)
                .unwrap_or_else(|| user.grpc_url.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CloudConfig;

    #[test]
    fn repo_cloud_api_url_overrides_user() {
        let mut repo = RepoConfig::default();
        repo.backend.cloud = Some(CloudConfig {
            api_url: Some("https://repo.example.com".to_string()),
        });
        let user = Config {
            grpc_url: "https://user.example.com".to_string(),
            ..Config::default()
        };

        let effective = EffectiveConfig::merge(&repo, &user);
        assert_eq!(effective.cloud_api_url, "https://repo.example.com");
    }

    #[test]
    fn falls_back_to_user_level_when_repo_absent() {
        let repo = RepoConfig::default();
        let user = Config {
            grpc_url: "https://user.example.com".to_string(),
            ..Config::default()
        };

        let effective = EffectiveConfig::merge(&repo, &user);
        assert_eq!(effective.cloud_api_url, "https://user.example.com");
    }

    #[test]
    fn falls_back_to_compiled_default_when_both_absent() {
        let repo = RepoConfig::default();
        let user = Config::default();

        let effective = EffectiveConfig::merge(&repo, &user);
        assert_eq!(effective.cloud_api_url, "http://localhost:50051");
    }

    #[test]
    fn backend_type_comes_from_repo_config() {
        let mut repo = RepoConfig::default();
        repo.backend.backend_type = BackendType::Local;
        let user = Config::default();

        let effective = EffectiveConfig::merge(&repo, &user);
        assert_eq!(effective.backend_type, BackendType::Local);
    }
}
