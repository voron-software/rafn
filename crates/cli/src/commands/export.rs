//! Export command - export benchmark data

use anyhow::{Context, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Write};

use crate::config_file::Config;

#[derive(Args)]
pub struct ExportCommand {
    /// Repository name
    #[arg(short, long)]
    repo: Option<String>,

    /// Output format
    #[arg(short, long)]
    format: OutputFormat,

    /// Commit SHA filter (optional)
    #[arg(short, long)]
    commit: Option<String>,

    /// Benchmark name filter (optional)
    #[arg(short, long)]
    name: Option<String>,

    /// Output file (stdout if not specified)
    #[arg(short, long)]
    output: Option<String>,

    /// Limit number of results
    #[arg(short, long, default_value = "10000")]
    limit: u32,

    /// API URL (defaults to config file value)
    #[arg(long)]
    api_url: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Json,
    Csv,
}

#[derive(Debug, Deserialize, Serialize)]
struct BenchmarkSummary {
    benchmark_name: String,
    mean_ns: f64,
    median_ns: f64,
    stddev_ns: f64,
    min_ns: f64,
    max_ns: f64,
    iterations: u64,
    ops_per_sec: f64,
}

impl ExportCommand {
    pub async fn execute(self) -> Result<()> {
        let config = Config::load()?;
        let api_url = self.api_url.unwrap_or(config.api_url);
        let repo = self.repo.or(config.default_repo).context(
            "Repository not specified. Use --repo, set PERFSCOPE_REPO, or configure default_repo",
        )?;

        // Build query parameters and URL
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
        let benchmarks: Vec<BenchmarkSummary> = serde_json::from_value(
            body.get("benchmarks")
                .context("Missing 'benchmarks' field in response")?
                .clone(),
        )?;

        if benchmarks.is_empty() {
            eprintln!("Warning: No benchmarks found");
            return Ok(());
        }

        let count = benchmarks.len();

        // Prepare output writer
        let mut writer: Box<dyn Write> = match &self.output {
            Some(path) => Box::new(File::create(path)?),
            None => Box::new(io::stdout()),
        };

        // Write output
        match self.format {
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&benchmarks)?;
                writeln!(writer, "{}", json)?;
            }
            OutputFormat::Csv => {
                writeln!(writer, "benchmark_name,mean_ns,median_ns,stddev_ns,min_ns,max_ns,iterations,ops_per_sec")?;
                for bench in benchmarks {
                    writeln!(
                        writer,
                        "{},{},{},{},{},{},{},{}",
                        bench.benchmark_name,
                        bench.mean_ns,
                        bench.median_ns,
                        bench.stddev_ns,
                        bench.min_ns,
                        bench.max_ns,
                        bench.iterations,
                        bench.ops_per_sec
                    )?;
                }
            }
        }

        if let Some(path) = &self.output {
            eprintln!("Exported {} benchmarks to {}", count, path);
        } else {
            eprintln!("Exported {} benchmarks", count);
        }

        Ok(())
    }
}
