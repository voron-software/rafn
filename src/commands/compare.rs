//! `rafn compare` — compare benchmarks between two commits.
//!
//! With `[backend] type = "local"` (rafn.toml) snapshots are read from the
//! local store. With `[backend] type = "cloud"` (default) the rafn cloud
//! service is queried.

use anyhow::Result;
use clap::Args;
use tracing::info;

use crate::comparison::{self, Report};
use crate::config::{BackendType, Config, EffectiveConfig, RepoConfig};
use crate::store::{self, Backend, RemoteBackend, SelectedBackend};

#[derive(Args, Debug)]
pub struct CompareCommand {
    /// Base commit SHA
    #[arg(long)]
    base: String,

    /// Head commit SHA
    #[arg(long)]
    head: String,

    /// Regression threshold percentage (overrides rafn.toml [bench].threshold)
    #[arg(long)]
    threshold: Option<f64>,

    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

impl CompareCommand {
    pub async fn execute(self) -> Result<()> {
        let CompareCommand {
            base,
            head,
            threshold,
            format,
        } = self;

        let repo_config = RepoConfig::load()?;
        let user_config = Config::load()?;
        let effective = EffectiveConfig::resolve(&repo_config, &user_config);
        let threshold_pct = threshold.unwrap_or(effective.bench_threshold);
        comparison::validate_threshold_pct(threshold_pct)?;

        let backend = match effective.backend_type {
            BackendType::Local => SelectedBackend::Local(store::local_backend(&repo_config)),
            BackendType::Cloud => {
                SelectedBackend::Remote(RemoteBackend::from_effective(effective)?)
            }
        };

        if backend.is_remote() {
            info!("Comparing commits: base={base}, head={head}");
        }

        let report = backend.compare_commits(&base, &head, threshold_pct).await?;

        output_report(format, &report)
    }
}

// stdout is this CLI's output contract, not debug noise — users pipe/read it
// directly, unlike `tracing`'s log lines.
#[allow(clippy::print_stdout)]
fn output_report(format: OutputFormat, report: &Report) -> Result<()> {
    match format {
        OutputFormat::Table => comparison::print_report(report),
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(report)?;
            println!("{json}");
        }
    }

    Ok(())
}
