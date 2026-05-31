//! `rafn push` — upload local snapshots to the remote gRPC service.
//!
//! When `backend = "local"` is set in `rafn.toml`, this command is a no-op
//! (snapshots are intended to stay local only).

use anyhow::Result;
use clap::Args;

use crate::config::{Backend, RepoConfig};
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

    /// Quiet mode — suppress output except errors
    #[arg(short, long)]
    quiet: bool,
}

impl PushCommand {
    pub async fn execute(self) -> Result<()> {
        let repo_config = RepoConfig::load()?;

        if repo_config.backend == Backend::Local {
            if !self.quiet {
                println!("Backend is set to \"local\" — nothing to push.");
            }
            return Ok(());
        }

        let remote = store::remote_backend_for_push(repo_config, self.grpc_url.clone());
        let local_store = store::LocalBackend::default();

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
            if !self.quiet {
                println!("No snapshots found to push.");
            }
            return Ok(());
        }

        if !self.quiet {
            if self.dry_run {
                println!("Dry run — snapshots will not be submitted.");
            }
            println!("Pushing to {}", remote.grpc_url());
        };

        let mut client = if self.dry_run {
            None
        } else {
            Some(remote.connect_ingest().await?)
        };

        let mut total_submitted = 0u32;
        for commit in &commits {
            let benchmarks = match local_store.load(commit)? {
                Some(b) => b,
                None => {
                    if !self.quiet {
                        eprintln!("Warning: No snapshot found for commit {commit}, skipping");
                    }
                    continue;
                }
            };

            if !self.quiet {
                print!(
                    "  Pushing snapshot for {commit} ({} benchmarks)... ",
                    benchmarks.len()
                );
            }

            if let Some(ref mut c) = client {
                match c.submit(benchmarks).await {
                    Ok(count) => {
                        total_submitted += count;
                        if !self.quiet {
                            println!("ok ({count} ingested)");
                        }
                    }
                    Err(e) => {
                        if !self.quiet {
                            println!("failed");
                        }
                        eprintln!("Error pushing {commit}: {e}");
                    }
                }
            } else if !self.quiet {
                // dry_run
                println!("skipped (dry run)");
            }
        }

        if !self.quiet && !self.dry_run {
            println!("\nTotal benchmarks submitted: {total_submitted}");
        }

        Ok(())
    }
}
