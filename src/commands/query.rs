//! Query command - query benchmarks from the database

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::config_file::Config;

#[derive(Args)]
pub struct QueryCommand {
    /// Repository name
    #[arg(short, long)]
    repo: Option<String>,

    /// Commit SHA (optional)
    #[arg(short, long)]
    commit: Option<String>,

    /// Benchmark name filter (optional)
    #[arg(short, long)]
    name: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,

    /// Limit number of results
    #[arg(short, long, default_value = "100")]
    limit: u32,

    /// API URL (defaults to config file value)
    #[arg(long)]
    api_url: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

fn format_duration(ns: &f64) -> String {
    format!("{:.3}", ns / 1_000_000.0)
}

fn format_ops(ops: &f64) -> String {
    if *ops > 0.0 {
        format!("{:.2}", ops)
    } else {
        "-".to_string()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Tabled)]
struct BenchmarkRow {
    #[tabled(rename = "Benchmark")]
    benchmark_name: String,
    #[tabled(rename = "Mean (ms)", display = "format_duration")]
    mean_ns: f64,
    #[tabled(rename = "Median (ms)", display = "format_duration")]
    median_ns: f64,
    #[tabled(rename = "Std Dev (ms)", display = "format_duration")]
    stddev_ns: f64,
    #[tabled(rename = "Iterations")]
    iterations: u64,
    #[tabled(rename = "Ops/sec", display = "format_ops")]
    ops_per_sec: f64,
}

impl QueryCommand {
    pub async fn execute(self) -> Result<()> {
        let config = Config::load()?;
        let api_url = self.api_url.unwrap_or(config.api_url);
        let repo = self.repo.or(config.default_repo).context(
            "Repository not specified. Use --repo, set PERFSCOPE_REPO, or configure default_repo",
        )?;

        // Build query parameters
        let mut url = format!(
            "{}/v1/benchmarks?repository={}&limit={}",
            api_url,
            urlencoding::encode(&repo),
            self.limit
        );

        if let Some(commit) = &self.commit {
            url.push_str(&format!("&commit_sha={}", urlencoding::encode(commit)));
        }

        if let Some(name) = &self.name {
            url.push_str(&format!("&benchmark_name={}", urlencoding::encode(name)));
        }

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
        let benchmarks: Vec<BenchmarkRow> = serde_json::from_value(
            body.get("benchmarks")
                .context("Missing 'benchmarks' field in response")?
                .clone(),
        )?;

        if benchmarks.is_empty() {
            println!("{}", "No benchmarks found".yellow());
            return Ok(());
        }

        // Output results
        match self.format {
            OutputFormat::Table => {
                let table = Table::new(&benchmarks).to_string();
                println!("{}", table);
                println!("\n{} Found {} benchmark(s)", "✓".green(), benchmarks.len());
            }
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&benchmarks)?;
                println!("{}", json);
            }
            OutputFormat::Csv => {
                println!("Benchmark,Mean (ns),Median (ns),Std Dev (ns),Iterations,Ops/sec");
                for bench in benchmarks {
                    println!(
                        "{},{},{},{},{},{}",
                        bench.benchmark_name,
                        bench.mean_ns,
                        bench.median_ns,
                        bench.stddev_ns,
                        bench.iterations,
                        bench.ops_per_sec
                    );
                }
            }
        }

        Ok(())
    }
}
