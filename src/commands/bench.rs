//! `rafn bench` — run benchmarks, save a local snapshot, show regressions.
//!
//! Unlike the old `run` command, results are never submitted over gRPC here;
//! use `rafn push` to upload snapshots to the server.

use anyhow::{Result, bail};
use clap::Args;
use std::path::PathBuf;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::{Config, RepoConfig};
use crate::proto::benchmark::timestamp_now;
use crate::{comparison, discovery, framework, git, ingest, runner, store};

#[derive(Args)]
pub struct BenchCommand {
    /// Repository name (auto-detected from git if not specified)
    #[arg(long, env = "RAFN_REPO")]
    repo: Option<String>,

    /// Commit SHA (auto-detected from git if not specified)
    #[arg(long, env = "RAFN_COMMIT")]
    commit: Option<String>,

    /// Branch name (auto-detected from git if not specified)
    #[arg(long, env = "RAFN_BRANCH")]
    branch: Option<String>,

    /// Results directory (auto-detected based on framework)
    #[arg(long)]
    results_dir: Option<PathBuf>,

    /// Regression threshold percentage (overrides rafn.toml [bench].threshold)
    #[arg(long)]
    threshold: Option<f64>,

    /// Show regressions but do not exit non-zero on regression
    #[arg(long)]
    no_fail: bool,

    /// Arguments passed to the detected benchmark framework command
    #[arg(last = true)]
    args: Vec<String>,
}

impl BenchCommand {
    pub async fn execute(self) -> Result<()> {
        let _user_config = Config::load().unwrap_or_default();
        let repo_config = RepoConfig::load()?;

        let threshold = self
            .threshold
            .unwrap_or_else(|| repo_config.bench_threshold());

        let framework_config = framework::detect_framework(&self.args)?;

        info!(
            "Detected {} benchmark framework",
            framework_config.framework
        );

        ensure_result_dir(&framework_config.results_strategy)?;

        for command in &framework_config.commands {
            info!("Running: {}", command.display());
            let result = runner::run_benchmark(command)?;

            if !result.exit_status.success() {
                let bench_exit = result.exit_status.code().unwrap_or(1);
                error!("Benchmark command exited with code {bench_exit}");
                std::process::exit(bench_exit);
            }
        }

        let discovered = discovery::discover_results(
            &framework_config.results_strategy,
            self.results_dir.as_deref(),
        )?;

        if discovered.is_empty() {
            error!("No benchmark results found");
            std::process::exit(1);
        }

        info!("Found {} benchmark result(s)", discovered.len());

        let (repository, commit, branch) =
            git::GitInfo::resolve(self.repo, self.commit, self.branch);
        let repository = repository?;
        let commit = commit?;
        let run_uuid = Uuid::new_v4().to_string();
        let run_started_at = timestamp_now();

        let mut benchmark_sets = Vec::new();
        for bench in &discovered {
            let json = serde_json::to_string(&bench.data)?;
            let format = ingest::detect_format(&json).unwrap_or_else(|_| "criterion".to_string());
            let parser = ingest::get_parser(
                &format,
                repository.clone(),
                commit.clone(),
                branch.clone(),
                run_uuid.clone(),
                run_started_at.clone(),
            );
            match parser {
                Ok(p) => match p.parse(&json) {
                    Ok(mut parsed) => benchmark_sets.append(&mut parsed),
                    Err(e) => {
                        warn!("Failed to parse {}: {e}", bench.name);
                    }
                },
                Err(e) => {
                    warn!("No parser for {}: {e}", bench.name);
                }
            }
        }

        if benchmark_sets.is_empty() {
            bail!("No benchmarks could be parsed from discovered result files");
        }

        // Save the snapshot.
        let local_store = store::local_backend(&repo_config);
        local_store.save(&commit, &benchmark_sets)?;
        info!("Snapshot saved for commit {commit}");

        // Compare against the previous snapshot and show regressions.
        let prev = local_store.previous_before(&commit)?;
        let mut regressed = false;

        match prev {
            None => {
                info!("No previous snapshot found — skipping regression check.");
            }
            Some(prev_benches) => {
                let rows = comparison::compare(&prev_benches, &benchmark_sets);
                if rows.is_empty() {
                    info!("No common benchmarks with previous snapshot.");
                } else {
                    comparison::print_table(&rows);
                    regressed = comparison::has_regressions(&rows, threshold);
                    if regressed {
                        error!("✗ Regression detected (threshold: {threshold:.1}%)");
                    }
                }
            }
        }

        if regressed && !self.no_fail {
            std::process::exit(1);
        }

        Ok(())
    }
}

fn ensure_result_dir(strategy: &framework::ResultsStrategy) -> Result<()> {
    match strategy {
        framework::ResultsStrategy::JsonFile(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        framework::ResultsStrategy::JsonDirectory { dir, .. } => {
            std::fs::create_dir_all(dir)?;
        }
        framework::ResultsStrategy::CriterionDirectory(_) => {}
    }
    Ok(())
}
