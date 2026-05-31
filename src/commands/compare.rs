//! `rafn compare` — compare benchmarks between two commits.
//!
//! With `backend = "local"` (rafn.toml) snapshots are read from the local
//! store. With `backend = "remote"` (default) the remote HTTP API is queried.

use anyhow::Result;
use clap::Args;

use crate::comparison;
use crate::proto::Benchmark;
use crate::store::{self, Backend};

#[derive(Args)]
pub struct CompareCommand {
    /// Base commit SHA
    #[arg(long)]
    base: String,

    /// Head commit SHA
    #[arg(long)]
    head: String,

    /// Repository name (required for remote backend)
    #[arg(short, long)]
    repo: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,

    /// API URL (overrides user config; remote backend only)
    #[arg(long)]
    api_url: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

impl CompareCommand {
    pub async fn execute(self) -> Result<()> {
        let CompareCommand {
            base: base_sha,
            head: head_sha,
            repo,
            format,
            api_url,
        } = self;

        let backend = store::selected_backend(repo, api_url)?;

        if backend.is_remote() {
            println!("Comparing commits:");
            println!("  Base: {base_sha}");
            println!("  Head: {head_sha}");
            println!();
        }

        let base = backend.benchmarks_for_commit(&base_sha).await?;
        if backend.is_remote() {
            println!("Fetched {} benchmarks from base commit", base.len());
        }

        let head = backend.benchmarks_for_commit(&head_sha).await?;
        if backend.is_remote() {
            println!("Fetched {} benchmarks from head commit", head.len());
            println!();
        }

        output_results(format, base, head)
    }
}

fn output_results(format: OutputFormat, base: Vec<Benchmark>, head: Vec<Benchmark>) -> Result<()> {
    let rows = comparison::compare(&base, &head);

    if rows.is_empty() {
        println!("No common benchmarks found between the two commits.");
        return Ok(());
    }

    match format {
        OutputFormat::Table => comparison::print_table(&rows),
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&rows)?;
            println!("{json}");
        }
    }

    Ok(())
}
