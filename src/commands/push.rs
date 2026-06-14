//! `rafn push` — upload local snapshots to the remote gRPC service.
//!
//! When `[backend] type = "local"` is set in `rafn.toml`, this command is a
//! no-op (snapshots are intended to stay local only).

use anyhow::{Result, bail};
use clap::Args;
use tracing::{error, info, warn};

use crate::config::{BackendType, RepoConfig};
use crate::git;
use crate::store;

#[derive(Args)]
pub struct PushCommand {
    /// Commit SHA to push (auto-detected from git if not specified).
    /// Push all stored snapshots with --all.
    #[arg(long, env = "RAFN_COMMIT")]
    commit: Option<String>,

    /// Push every stored snapshot, not just the current commit
    #[arg(long)]
    all: bool,

    /// Parse and validate but do not submit
    #[arg(long)]
    dry_run: bool,

    /// gRPC server URL (overrides rafn.toml and user config)
    #[arg(long, env = "RAFN_GRPC_URL")]
    grpc_url: Option<String>,
}

impl PushCommand {
    pub async fn execute(self) -> Result<()> {
        let repo_config = RepoConfig::load()?;

        if repo_config.backend.backend_type == BackendType::Local {
            info!("Backend is set to \"local\" — nothing to push.");
            return Ok(());
        }

        let local_store = store::local_backend(&repo_config);
        let remote = store::remote_backend_for_push(repo_config, self.grpc_url.clone());

        // Collect the commits to push.
        let commits: Vec<String> = if self.all {
            local_store
                .list_commits()?
                .into_iter()
                .map(|(commit, _)| commit)
                .collect()
        } else {
            let commit = self
                .commit
                .or_else(|| git::detect_git_info().commit_sha)
                .ok_or_else(|| {
                    anyhow::anyhow!("Could not detect commit SHA. Use --commit or set RAFN_COMMIT")
                })?;
            vec![commit]
        };

        if commits.is_empty() {
            info!("No snapshots found to push.");
            return Ok(());
        }

        if self.dry_run {
            info!("Dry run — snapshots will not be submitted.");
        }
        info!("Pushing to {}", remote.grpc_url());

        let mut client = if self.dry_run {
            None
        } else {
            Some(remote.connect_push().await?)
        };

        let mut total_submitted = 0u32;
        let mut failed_commits = Vec::new();
        for commit in &commits {
            let benchmark_sets = match local_store.load(commit)? {
                Some(b) => b,
                None => {
                    warn!("No snapshot found for commit {commit}, skipping");
                    continue;
                }
            };
            let benchmark_count: usize =
                benchmark_sets.iter().map(|set| set.benchmarks.len()).sum();

            info!(
                "Pushing snapshot for {commit} ({} benchmark sets, {} benchmarks)",
                benchmark_sets.len(),
                benchmark_count
            );

            if let Some(ref mut c) = client {
                match c.submit(benchmark_sets).await {
                    Ok(count) => {
                        total_submitted += count;
                        info!("Submitted snapshot for {commit} ({count} pushed)");
                    }
                    Err(e) => {
                        error!("Error pushing {commit}: {e}");
                        failed_commits.push(commit.clone());
                    }
                }
            } else {
                info!("Skipped snapshot for {commit} (dry run)");
            }
        }

        if !self.dry_run {
            info!("Total benchmarks submitted: {total_submitted}");
        }

        if !failed_commits.is_empty() {
            bail!(
                "Failed to push {} snapshot(s): {}",
                failed_commits.len(),
                failed_commits.join(", ")
            );
        }

        Ok(())
    }
}
