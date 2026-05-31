//! `rafn bench` — run benchmarks, save a local snapshot, show regressions.
//!
//! Unlike the old `run` command, results are never submitted over gRPC here;
//! use `rafn push` to upload snapshots to the server.

use anyhow::Result;
use clap::Args;
use std::path::PathBuf;
use uuid::Uuid;

use crate::config::{Config, RepoConfig};
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

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Quiet mode — suppress output except errors
    #[arg(short, long)]
    quiet: bool,

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

        if !self.quiet {
            println!(
                "Detected {} benchmark framework",
                framework_config.framework
            );
        }

        ensure_result_dir(&framework_config.results_strategy)?;

        let mut bench_exit = 0;
        for command in &framework_config.commands {
            if !self.quiet {
                println!("Running: {}", command.display());
            }
            let result = runner::run_benchmark(command, self.verbose)?;
            bench_exit = result.exit_status.code().unwrap_or(1);

            if !result.exit_status.success() {
                if !self.quiet {
                    eprintln!("Benchmark command exited with code {bench_exit}");
                }
                break;
            }
        }

        let discovered = discovery::discover_results(
            &framework_config.results_strategy,
            self.results_dir.as_deref(),
        )?;

        if discovered.is_empty() {
            if !self.quiet {
                eprintln!("No benchmark results found");
            }
            std::process::exit(if bench_exit == 0 { 1 } else { bench_exit });
        }

        if !self.quiet {
            println!("Found {} benchmark result(s)", discovered.len());
        }

        let (repository, commit, _branch) =
            git::GitInfo::resolve(self.repo, self.commit, self.branch);
        let repository = repository?;
        let commit = commit?;

        // Parse all discovered results into the domain Benchmark type.
        let mut benchmarks = Vec::new();
        for bench in &discovered {
            let json = serde_json::to_string(&bench.data)?;
            let format = ingest::detect_format(&json).unwrap_or_else(|_| "criterion".to_string());
            let parser =
                ingest::get_parser(&format, Uuid::nil(), repository.clone(), commit.clone());
            match parser {
                Ok(p) => match p.parse(&json) {
                    Ok(mut parsed) => benchmarks.append(&mut parsed),
                    Err(e) => {
                        if !self.quiet {
                            eprintln!("Warning: Failed to parse {}: {e}", bench.name);
                        }
                    }
                },
                Err(e) => {
                    if !self.quiet {
                        eprintln!("Warning: No parser for {}: {e}", bench.name);
                    }
                }
            }
        }

        // Save the snapshot.
        let local_store = store::LocalBackend::default();
        local_store.save(&commit, &benchmarks)?;
        if !self.quiet {
            println!("Snapshot saved for commit {commit}");
        }

        // Compare against the previous snapshot and show regressions.
        let prev = local_store.previous_before(&commit)?;
        let mut regressed = false;

        match prev {
            None => {
                if !self.quiet {
                    println!("No previous snapshot found — skipping regression check.");
                }
            }
            Some(prev_benches) => {
                let rows = comparison::compare(&prev_benches, &benchmarks);
                if rows.is_empty() {
                    if !self.quiet {
                        println!("No common benchmarks with previous snapshot.");
                    }
                } else {
                    if !self.quiet {
                        comparison::print_table(&rows);
                    }
                    regressed = comparison::has_regressions(&rows, threshold);
                    if regressed && !self.quiet {
                        eprintln!("✗ Regression detected (threshold: {threshold:.1}%)");
                    }
                }
            }
        }

        if bench_exit != 0 {
            std::process::exit(bench_exit);
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
