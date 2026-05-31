use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

use crate::framework::ResultsStrategy;

pub struct BenchmarkResult {
    pub name: String,
    pub data: Value,
}

pub fn discover_results(
    strategy: &ResultsStrategy,
    results_dir_override: Option<&Path>,
) -> Result<Vec<BenchmarkResult>> {
    match strategy {
        ResultsStrategy::CriterionDirectory(default_dir) => {
            let dir = results_dir_override.unwrap_or(default_dir);
            scan_criterion_directory(dir)
        }
        ResultsStrategy::JsonFile(default_file) => {
            if let Some(dir) = results_dir_override
                && dir.is_dir()
            {
                return scan_json_directory(dir, None);
            }
            let path = results_dir_override.unwrap_or(default_file);
            read_json_file(path)
                .map(|result| vec![result])
                .or_else(|err| {
                    if path.exists() {
                        Err(err)
                    } else {
                        Ok(Vec::new())
                    }
                })
        }
        ResultsStrategy::JsonDirectory {
            dir,
            required_suffix,
        } => {
            let dir = results_dir_override.unwrap_or(dir);
            scan_json_directory(dir, required_suffix.as_deref())
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

            let sample_file = path.join("new").join("sample.json");
            let total_iterations: u64 = if sample_file.exists() {
                let sample: Value = serde_json::from_str(
                    &std::fs::read_to_string(&sample_file).context("Failed to read sample.json")?,
                )
                .context("Failed to parse sample.json")?;

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

fn scan_json_directory(dir: &Path, required_suffix: Option<&str>) -> Result<Vec<BenchmarkResult>> {
    let mut results = Vec::new();

    if !dir.exists() {
        return Ok(results);
    }

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read JSON results directory {}", dir.display()))?
    {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(suffix) = required_suffix {
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.ends_with(suffix) {
                continue;
            }
        }
        results.push(read_json_file(&path)?);
    }

    Ok(results)
}

fn read_json_file(path: &Path) -> Result<BenchmarkResult> {
    let data = serde_json::from_str(
        &std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read benchmark result {}", path.display()))?,
    )
    .with_context(|| format!("Failed to parse benchmark result {}", path.display()))?;

    Ok(BenchmarkResult {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("benchmark result")
            .to_string(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn discovers_json_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("jmh.json");
        std::fs::write(&path, r#"[{"benchmark":"x","primaryMetric":{"score":1.0,"scoreError":0.0,"scoreUnit":"ns/op"}}]"#).unwrap();

        let results = discover_results(&ResultsStrategy::JsonFile(path), None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "jmh.json");
    }

    #[test]
    fn discovers_json_directory_with_suffix() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("keep-report-full.json"),
            r#"{"Benchmarks":[]}"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join("skip.json"), r#"{"Benchmarks":[]}"#).unwrap();

        let results = discover_results(
            &ResultsStrategy::JsonDirectory {
                dir: tmp.path().to_path_buf(),
                required_suffix: Some("-report-full.json".into()),
            },
            None,
        )
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "keep-report-full.json");
    }

    #[test]
    fn json_file_strategy_accepts_directory_override() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("result.json"), r#"{"benchmarks":[]}"#).unwrap();

        let results = discover_results(
            &ResultsStrategy::JsonFile(tmp.path().join("default.json")),
            Some(tmp.path()),
        )
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "result.json");
    }
}
