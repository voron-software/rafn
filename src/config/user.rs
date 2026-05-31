use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_api_url")]
    pub api_url: String,

    #[serde(default = "default_grpc_url")]
    pub grpc_url: String,

    pub db_url: Option<String>,

    pub tenant_id: Option<Uuid>,

    pub default_repo: Option<String>,
}

fn default_api_url() -> String {
    "http://localhost:3000".to_string()
}

fn default_grpc_url() -> String {
    "http://localhost:50051".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
            grpc_url: default_grpc_url(),
            db_url: None,
            tenant_id: None,
            default_repo: None,
        }
    }
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

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(&path, content)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "api_url" => Some(self.api_url.clone()),
            "grpc_url" => Some(self.grpc_url.clone()),
            "db_url" => self.db_url.clone(),
            "tenant_id" => self.tenant_id.map(|id| id.to_string()),
            "default_repo" => self.default_repo.clone(),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "api_url" => self.api_url = value.to_string(),
            "grpc_url" => self.grpc_url = value.to_string(),
            "db_url" => {
                self.db_url = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "tenant_id" => {
                self.tenant_id = if value.is_empty() {
                    None
                } else {
                    Some(Uuid::parse_str(value).context("Invalid UUID")?)
                };
            }
            "default_repo" => {
                self.default_repo = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
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
        assert_eq!(config.api_url, "http://localhost:3000");
        assert_eq!(config.grpc_url, "http://localhost:50051");
        assert!(config.db_url.is_none());
        assert!(config.tenant_id.is_none());
        assert!(config.default_repo.is_none());
    }

    #[test]
    fn test_get_set() {
        let mut config = Config::default();
        config.set("api_url", "http://example.com").unwrap();
        assert_eq!(config.get("api_url").unwrap(), "http://example.com");

        config
            .set("grpc_url", "http://grpc.example.com:50051")
            .unwrap();
        assert_eq!(
            config.get("grpc_url").unwrap(),
            "http://grpc.example.com:50051"
        );

        config.set("default_repo", "myrepo").unwrap();
        assert_eq!(config.get("default_repo").unwrap(), "myrepo");
    }

    #[test]
    fn test_invalid_tenant_id() {
        let mut config = Config::default();
        assert!(config.set("tenant_id", "not-a-uuid").is_err());
    }

    #[test]
    fn test_unknown_key() {
        let mut config = Config::default();
        assert!(config.set("unknown_key", "value").is_err());
    }
}
