//! Benchmark results discovery.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

use crate::framework::ResultsStrategy;

/// A discovered benchmark result.
#[derive(Debug)]
pub struct BenchmarkResult {
    pub name: String,
    pub data: Value,
}

/// Discover benchmark results based on the framework's results strategy.
pub fn discover_results(
    strategy: &ResultsStrategy,
    results_dir_override: Option<&Path>,
) -> Result<Vec<BenchmarkResult>> {
    match strategy {
        ResultsStrategy::Directory(default_dir) => {
            let dir = results_dir_override.unwrap_or(default_dir);
            scan_criterion_directory(dir)
        }
    }
}

fn scan_criterion_directory(dir: &Path) -> Result<Vec<BenchmarkResult>> {
    let mut results = Vec::new();

    if !dir.exists() {
        return Ok(results);
    }

    for entry in std::fs::read_dir(dir).context("Failed to read criterion directory")? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let estimates_file = path.join("new").join("estimates.json");
        let benchmark_file = path.join("new").join("benchmark.json");

        if estimates_file.exists() && benchmark_file.exists() {
            let estimates: Value = serde_json::from_str(
                &std::fs::read_to_string(&estimates_file)
                    .context("Failed to read estimates.json")?,
            )
            .context("Failed to parse estimates.json")?;

            let benchmark_info: Value = serde_json::from_str(
                &std::fs::read_to_string(&benchmark_file)
                    .context("Failed to read benchmark.json")?,
            )
            .context("Failed to parse benchmark.json")?;

            let full_id = benchmark_info["full_id"]
                .as_str()
                .unwrap_or_else(|| path.file_name().unwrap().to_str().unwrap())
                .to_string();

            // Extract total_iterations from sample.json if available
            let sample_file = path.join("new").join("sample.json");
            let total_iterations: u64 = if sample_file.exists() {
                let sample: Value = serde_json::from_str(
                    &std::fs::read_to_string(&sample_file).context("Failed to read sample.json")?,
                )
                .context("Failed to parse sample.json")?;

                // sample.json has { "iters": [100, 200, ...], ... }
                sample["iters"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_f64())
                            .map(|v| v as u64)
                            .sum()
                    })
                    .unwrap_or(0)
            } else {
                0
            };

            // Format data as expected by CriterionParser
            let data = serde_json::json!({
                "id": full_id,
                "mean": estimates["mean"],
                "median": estimates["median"],
                "std_dev": estimates["std_dev"],
                "total_iterations": total_iterations,
            });

            results.push(BenchmarkResult {
                name: full_id,
                data,
            });
        }
    }

    Ok(results)
}
