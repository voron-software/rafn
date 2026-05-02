//! Compare command - compare benchmarks between two commits

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tabled::{Table, Tabled};

use crate::config_file::Config;

#[derive(Args)]
pub struct CompareCommand {
    /// Repository name
    #[arg(short, long)]
    repo: Option<String>,

    /// Base commit SHA
    #[arg(long)]
    base: String,

    /// Head commit SHA
    #[arg(long)]
    head: String,

    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,

    /// API URL (defaults to config file value)
    #[arg(long)]
    api_url: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BenchmarkSummary {
    benchmark_name: String,
    commit_sha: String,
    mean_ns: f64,
    median_ns: f64,
    stddev_ns: f64,
    min_ns: f64,
    max_ns: f64,
    iterations: u64,
    ops_per_sec: f64,
}

fn format_duration_compare(ns: &f64) -> String {
    format!("{:.3}", ns / 1_000_000.0)
}

fn format_diff(ns: &f64) -> String {
    let ms = ns / 1_000_000.0;
    if *ns > 0.0 {
        format!("+{:.3}", ms)
    } else {
        format!("{:.3}", ms)
    }
}

fn format_percent(pct: &f64) -> String {
    if *pct > 0.0 {
        format!("+{:.1}%", pct)
    } else {
        format!("{:.1}%", pct)
    }
}

#[derive(Debug, Clone, Serialize, Tabled)]
struct ComparisonRow {
    #[tabled(rename = "Benchmark")]
    benchmark_name: String,
    #[tabled(rename = "Base (ms)", display = "format_duration_compare")]
    base_mean_ns: f64,
    #[tabled(rename = "Head (ms)", display = "format_duration_compare")]
    head_mean_ns: f64,
    #[tabled(rename = "Diff (ms)", display = "format_diff")]
    diff_ns: f64,
    #[tabled(rename = "Change %", display = "format_percent")]
    diff_pct: f64,
}

impl CompareCommand {
    pub async fn execute(self) -> Result<()> {
        let config = Config::load()?;
        let api_url = self.api_url.unwrap_or(config.api_url);
        let repo = self.repo.or(config.default_repo).context(
            "Repository not specified. Use --repo, set PERFSCOPE_REPO, or configure default_repo",
        )?;

        println!("Comparing commits:");
        println!("  Base: {}", self.base);
        println!("  Head: {}", self.head);
        println!();

        // Fetch base commit benchmarks
        let base_benchmarks = fetch_benchmarks(&api_url, &repo, &self.base).await?;
        println!(
            "Fetched {} benchmarks from base commit",
            base_benchmarks.len()
        );

        // Fetch head commit benchmarks
        let head_benchmarks = fetch_benchmarks(&api_url, &repo, &self.head).await?;
        println!(
            "Fetched {} benchmarks from head commit",
            head_benchmarks.len()
        );
        println!();

        // Build lookup map for head benchmarks
        let head_map: HashMap<String, &BenchmarkSummary> = head_benchmarks
            .iter()
            .map(|b| (b.benchmark_name.clone(), b))
            .collect();

        // Compare benchmarks
        let mut comparisons = Vec::new();
        for base_bench in &base_benchmarks {
            if let Some(head_bench) = head_map.get(&base_bench.benchmark_name) {
                let diff_ns = head_bench.mean_ns - base_bench.mean_ns;
                let diff_pct = if base_bench.mean_ns > 0.0 {
                    (diff_ns / base_bench.mean_ns) * 100.0
                } else {
                    0.0
                };

                comparisons.push(ComparisonRow {
                    benchmark_name: base_bench.benchmark_name.clone(),
                    base_mean_ns: base_bench.mean_ns,
                    head_mean_ns: head_bench.mean_ns,
                    diff_ns,
                    diff_pct,
                });
            }
        }

        if comparisons.is_empty() {
            println!("{}", "No common benchmarks found between commits".yellow());
            return Ok(());
        }

        // Sort by absolute diff (largest changes first)
        comparisons.sort_by(|a, b| b.diff_ns.abs().partial_cmp(&a.diff_ns.abs()).unwrap());

        // Output results
        match self.format {
            OutputFormat::Table => {
                let table = Table::new(&comparisons).to_string();
                println!("{}", table);

                // Summary
                let improvements = comparisons.iter().filter(|c| c.diff_ns < 0.0).count();
                let regressions = comparisons.iter().filter(|c| c.diff_ns > 0.0).count();
                println!();
                println!("Summary:");
                println!("  Total: {}", comparisons.len());
                println!("  {} Improvements: {}", "✓".green(), improvements);
                println!("  {} Regressions: {}", "✗".red(), regressions);
            }
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&comparisons)?;
                println!("{}", json);
            }
        }

        Ok(())
    }
}

async fn fetch_benchmarks(
    api_url: &str,
    repository: &str,
    commit_sha: &str,
) -> Result<Vec<BenchmarkSummary>> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/v1/benchmarks?repository={}&commit_sha={}&limit=1000",
        api_url,
        urlencoding::encode(repository),
        urlencoding::encode(commit_sha)
    );

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
    let benchmarks: Vec<BenchmarkSummary> = serde_json::from_value(
        body.get("benchmarks")
            .context("Missing 'benchmarks' field in response")?
            .clone(),
    )?;

    Ok(benchmarks)
}
