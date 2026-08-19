//! Config command - manage configuration

use crate::config::Config;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigSubcommand {
    /// Initialize configuration file
    Init,
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set {
        /// Configuration key (cloud.api_url)
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
    // stdout is this CLI's output contract, not debug noise — users pipe/read it
    // directly, unlike `tracing`'s log lines.
    #[allow(clippy::print_stdout)]
    pub async fn execute(self) -> Result<()> {
        match self.command {
            ConfigSubcommand::Init => {
                let path = Config::config_path()?;
                let config = Config::init_at(&path)?;
                println!("Configuration file initialized at: {}", path.display());
                println!("\nDefault values:");
                println!("  cloud.api_url: {}", config.cloud.api_url);
            }
            ConfigSubcommand::Show => {
                let config = Config::load()?;
                let path = Config::config_path()?;
                println!("Configuration file: {}", path.display());
                println!("\nCurrent values:");
                println!("  cloud.api_url: {}", config.cloud.api_url);
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
