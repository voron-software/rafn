//! `rafn bench` — run benchmarks, save a local snapshot, show regressions.
//!
//! Unlike the old `run` command, results are never submitted over gRPC here;
//! use `rafn push` to upload snapshots to the server.

use anyhow::{Result, bail};
use clap::Args;
use std::path::PathBuf;
use tracing::{error, info};
use uuid::Uuid;

use crate::config::{Config, EffectiveConfig, RepoConfig};
use crate::proto::benchmark::timestamp_now;
use crate::{comparison, discovery, framework, git, ingest, runner, store};

#[derive(Args, Debug)]
pub struct BenchCommand {
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
        let user_config = Config::load()?;
        let repo_config = RepoConfig::load()?;
        let effective = EffectiveConfig::resolve(&repo_config, &user_config);

        let threshold = self.threshold.unwrap_or(effective.bench_threshold);
        comparison::validate_threshold_pct(threshold)?;
        let repository = store::require_repository(&effective)?;

        let framework_config = framework::detect_framework(&self.args)?;

        info!(
            "Detected {} benchmark framework",
            framework_config.framework
        );

        framework_config.results_strategy.ensure_dir()?;

        for command in &framework_config.commands {
            info!("Running: {}", command.display());
            let result = runner::run_benchmark(command)?;

            if !result.exit_status.success() {
                let bench_exit = result.exit_status.code().unwrap_or(1);
                runner::replay_stderr_on_failure(&result.stderr);
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

        let (commit, branch) = git::GitInfo::resolve(self.commit, self.branch);
        let commit = commit?;
        let run_uuid = Uuid::new_v4().to_string();
        let run_started_at = timestamp_now();

        let benchmark_sets = ingest::parse_discovered(
            &discovered,
            &repository,
            &commit,
            branch.as_deref(),
            &run_uuid,
            run_started_at,
        )?;

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
                let base_series = comparison::flatten_series(&prev_benches);
                let head_series = comparison::flatten_series(&benchmark_sets);
                if base_series.is_empty() && head_series.is_empty() {
                    info!("No common benchmarks with previous snapshot.");
                } else {
                    let report = comparison::compare(&base_series, &head_series, threshold);
                    comparison::print_report(&report);
                    regressed = report.summary.has_regressions;
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
