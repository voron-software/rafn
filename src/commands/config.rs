//! Config command - manage configuration

use crate::config::Config;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Subcommand)]
pub enum ConfigSubcommand {
    /// Initialize configuration file
    Init,
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set {
        /// Configuration key (grpc_url, db_url, tenant_id, default_repo)
        key: String,
        /// Configuration value
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Configuration key
        key: String,
    },
}

impl ConfigCommand {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            ConfigSubcommand::Init => {
                let config = Config::default();
                config.save().context("Failed to save config")?;
                let path = Config::config_path()?;
                println!("Configuration file initialized at: {}", path.display());
                println!("\nDefault values:");
                println!(
                    "  db_url: {}",
                    config.db_url.unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "  tenant_id: {}",
                    config
                        .tenant_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "  default_repo: {}",
                    config
                        .default_repo
                        .unwrap_or_else(|| "(not set)".to_string())
                );
            }
            ConfigSubcommand::Show => {
                let config = Config::load()?;
                let path = Config::config_path()?;
                println!("Configuration file: {}", path.display());
                println!("\nCurrent values:");
                println!(
                    "  db_url: {}",
                    config.db_url.unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "  tenant_id: {}",
                    config
                        .tenant_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "  default_repo: {}",
                    config
                        .default_repo
                        .unwrap_or_else(|| "(not set)".to_string())
                );
            }
            ConfigSubcommand::Set { key, value } => {
                let mut config = Config::load()?;
                config
                    .set(&key, &value)
                    .context("Failed to set config value")?;
                config.save().context("Failed to save config")?;
                println!("Set {} = {}", key, value);
            }
            ConfigSubcommand::Get { key } => {
                let config = Config::load()?;
                match config.get(&key) {
                    Some(value) => println!("{}", value),
                    None => println!("(not set)"),
                }
            }
        }
        Ok(())
    }
}
