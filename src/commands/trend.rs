//! `rafn trend` — display benchmark history over time.
//!
//! With `[backend] type = "local"` (rafn.toml) the history is read from the
//! local snapshot store. With `[backend] type = "cloud"` (default) the rafn
//! cloud service is queried.

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use std::collections::HashMap;
use tabled::Table;
use tracing::info;

use crate::store::{self, Backend, TrendDataPoint, TrendQuery};

#[derive(Args)]
pub struct TrendCommand {
    /// Repository name
    #[arg(short, long)]
    repo: Option<String>,

    /// Benchmark name. When omitted, all benchmarks are shown.
    #[arg(short, long)]
    name: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,

    /// Limit number of data points (remote backend only)
    #[arg(short, long, default_value = "50")]
    limit: u32,

    /// gRPC URL (overrides user config; remote backend only)
    #[arg(long)]
    grpc_url: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

impl TrendCommand {
    pub async fn execute(self) -> Result<()> {
        let backend = store::selected_backend(self.repo.clone(), self.grpc_url.clone())?;

        if backend.is_remote() {
            if let Some(ref name) = self.name {
                info!("Fetching trend data for benchmark: {}", name.cyan());
            } else {
                info!("Fetching trend data for all benchmarks");
            }
            info!("Repository: {}", backend.repository().unwrap_or_default());
        }

        let data_points = backend
            .trend(TrendQuery {
                benchmark_name: self.name.clone(),
                limit: self.limit,
            })
            .await?;

        if data_points.is_empty() {
            println!("{}", "No trend data found.".yellow());
            return Ok(());
        }

        self.output(data_points)
    }

    fn output(self, data_points: Vec<TrendDataPoint>) -> Result<()> {
        match self.format {
            OutputFormat::Table => {
                let table = Table::new(&data_points).to_string();
                println!("{table}");

                // Per-benchmark statistics when showing all benchmarks.
                let mut by_name: HashMap<String, Vec<f64>> = HashMap::new();
                for dp in &data_points {
                    by_name
                        .entry(dp.benchmark_name.clone())
                        .or_default()
                        .push(dp.mean_ns);
                }

                println!();
                println!("Statistics:");
                println!("  Data points: {}", data_points.len());

                for (name, values) in &by_name {
                    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
                    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let avg = values.iter().sum::<f64>() / values.len() as f64;
                    println!(
                        "  [{name}] min={:.3}ms  max={:.3}ms  avg={:.3}ms",
                        min / 1_000_000.0,
                        max / 1_000_000.0,
                        avg / 1_000_000.0
                    );
                    if values.len() >= 2 {
                        let change_pct = (values[values.len() - 1] - values[0]) / values[0] * 100.0;
                        println!("  [{name}] overall change: {change_pct:+.1}%");
                    }
                }
            }
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&data_points)?;
                println!("{json}");
            }
        }
        Ok(())
    }
}
