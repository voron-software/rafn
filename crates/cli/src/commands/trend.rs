//! Trend command - show time-series trend data for a benchmark

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::config_file::Config;

#[derive(Args)]
pub struct TrendCommand {
    /// Repository name
    #[arg(short, long)]
    repo: Option<String>,

    /// Benchmark name
    #[arg(short, long)]
    name: String,

    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,

    /// Limit number of data points
    #[arg(short, long, default_value = "50")]
    limit: u32,

    /// API URL (defaults to config file value)
    #[arg(long)]
    api_url: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

fn format_duration_trend(ns: &f64) -> String {
    format!("{:.3}", ns / 1_000_000.0)
}

fn format_timestamp(ts: &i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(*ts / 1000, 0).unwrap_or_default();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Tabled)]
struct TrendDataPoint {
    #[tabled(rename = "Commit")]
    commit_sha: String,
    #[tabled(rename = "Timestamp", display = "format_timestamp")]
    timestamp: i64,
    #[tabled(rename = "Mean (ms)", display = "format_duration_trend")]
    mean_ns: f64,
    #[tabled(rename = "Median (ms)", display = "format_duration_trend")]
    median_ns: f64,
    #[tabled(rename = "Std Dev (ms)", display = "format_duration_trend")]
    stddev_ns: f64,
}

impl TrendCommand {
    pub async fn execute(self) -> Result<()> {
        let config = Config::load()?;
        let api_url = self.api_url.unwrap_or(config.api_url);
        let repo = self.repo.or(config.default_repo).context(
            "Repository not specified. Use --repo, set PERFSCOPE_REPO, or configure default_repo",
        )?;

        println!("Fetching trend data for benchmark: {}", self.name.cyan());
        println!("Repository: {}", repo);
        println!();

        // Build query parameters and URL
        let url = format!(
            "{}/v1/benchmarks/trend?repository={}&benchmark_name={}&limit={}",
            api_url,
            urlencoding::encode(&repo),
            urlencoding::encode(&self.name),
            self.limit
        );

        // Make API request
        let client = reqwest::Client::new();

        let response = client
            .get(&url)
            .send()
            .await
            .context("Failed to send request to API")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("API error ({}): {}", status, text);
        }

        let body: serde_json::Value = response.json().await?;
        let mut data_points: Vec<TrendDataPoint> = serde_json::from_value(
            body.get("data")
                .context("Missing 'data' field in response")?
                .clone(),
        )?;

        if data_points.is_empty() {
            println!("{}", "No trend data found".yellow());
            return Ok(());
        }

        // Reverse to show oldest first
        data_points.reverse();

        // Output results
        match self.format {
            OutputFormat::Table => {
                let table = Table::new(&data_points).to_string();
                println!("{}", table);

                // Calculate statistics
                let mean_values: Vec<f64> = data_points.iter().map(|d| d.mean_ns).collect();
                let min_mean = mean_values.iter().cloned().fold(f64::INFINITY, f64::min);
                let max_mean = mean_values
                    .iter()
                    .cloned()
                    .fold(f64::NEG_INFINITY, f64::max);
                let avg_mean = mean_values.iter().sum::<f64>() / mean_values.len() as f64;

                println!();
                println!("Statistics:");
                println!("  Data points: {}", data_points.len());
                println!("  Min mean: {:.3} ms", min_mean / 1_000_000.0);
                println!("  Max mean: {:.3} ms", max_mean / 1_000_000.0);
                println!("  Avg mean: {:.3} ms", avg_mean / 1_000_000.0);

                if data_points.len() >= 2 {
                    let first = &data_points[0];
                    let last = &data_points[data_points.len() - 1];
                    let change = last.mean_ns - first.mean_ns;
                    let change_pct = (change / first.mean_ns) * 100.0;

                    println!(
                        "  Overall change: {:.3} ms ({:.1}%)",
                        change / 1_000_000.0,
                        change_pct
                    );
                }
            }
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&data_points)?;
                println!("{}", json);
            }
        }

        Ok(())
    }
}
