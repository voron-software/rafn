//! User-level configuration, loaded from `~/.config/rafn/config.toml`.
//!
//! Loaded via the `config` crate so values layer file < env
//! (`RAFN_CLOUD__API_URL` etc.), with compiled defaults as the final
//! fallback. `api_key` is intentionally not a field here: it is always
//! sourced from the `RAFN_API_KEY` environment variable, never persisted to
//! this file.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

fn default_api_url() -> String {
    "https://api.rqfn.dev".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserCloudConfig {
    #[serde(default = "default_api_url")]
    pub api_url: String,
}

impl Default for UserCloudConfig {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub cloud: UserCloudConfig,
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .context("Could not determine home directory")?;

        Ok(PathBuf::from(home)
            .join(".config")
            .join("rafn")
            .join("config.toml"))
    }

    /// Load from `path` (if it exists) layered with the given environment
    /// source, falling back to compiled defaults.
    fn load_from_with_env(path: &Path, env: config::Environment) -> Result<Self> {
        let built = config::Config::builder()
            .add_source(config::File::from(path).required(false))
            .add_source(env)
            .build()
            .context("Failed to build configuration")?;

        built
            .try_deserialize()
            .context("Failed to parse configuration")
    }

    /// Load from `path` (if it exists) layered with `RAFN_`-prefixed
    /// environment variables (double underscore as the nesting separator),
    /// falling back to compiled defaults.
    fn load_from(path: &Path) -> Result<Self> {
        Self::load_from_with_env(
            path,
            config::Environment::with_prefix("RAFN")
                .prefix_separator("_")
                .separator("__"),
        )
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::config_path()?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::config_path()?)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(path, content)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    /// Write the default config to `path`, refusing to overwrite a file that
    /// already exists there.
    pub fn init_at(path: &Path) -> Result<Self> {
        if path.exists() {
            bail!(
                "Configuration file already exists at {}; refusing to overwrite",
                path.display()
            );
        }

        let config = Self::default();
        config.save_to(path)?;
        Ok(config)
    }

    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "cloud.api_url" => Some(self.cloud.api_url.clone()),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "cloud.api_url" => self.cloud.api_url = value.to_string(),
            _ => anyhow::bail!("Unknown configuration key: {}", key),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.cloud.api_url, "https://api.rqfn.dev");
    }

    #[test]
    fn test_get_set() {
        let mut config = Config::default();
        config
            .set("cloud.api_url", "http://grpc.example.com:50051")
            .unwrap();
        assert_eq!(
            config.get("cloud.api_url").unwrap(),
            "http://grpc.example.com:50051"
        );
    }

    #[test]
    fn test_unknown_key() {
        let mut config = Config::default();
        assert!(config.set("unknown_key", "value").is_err());
        assert!(config.get("unknown_key").is_none());
    }

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_load_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[cloud]\napi_url = \"https://file.example.com\"\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.cloud.api_url, "https://file.example.com");
    }

    #[test]
    fn test_init_at_writes_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = Config::init_at(&path).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
    }

    #[test]
    fn test_init_at_refuses_to_overwrite_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[cloud]\napi_url = \"https://custom.example.com\"\n").unwrap();

        assert!(Config::init_at(&path).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[cloud]\napi_url = \"https://custom.example.com\"\n"
        );
    }

    #[test]
    fn test_env_override_flows_through_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        // Inject a fake environment rather than mutating real process env
        // vars, so this test can't race with others reading RAFN_* vars.
        let mut env = config::Map::new();
        env.insert(
            "RAFN_CLOUD__API_URL".to_string(),
            "https://env.example.com".to_string(),
        );
        let env_source = config::Environment::with_prefix("RAFN")
            .prefix_separator("_")
            .separator("__")
            .source(Some(env));

        let config = Config::load_from_with_env(&path, env_source).unwrap();
        assert_eq!(config.cloud.api_url, "https://env.example.com");
    }
}
