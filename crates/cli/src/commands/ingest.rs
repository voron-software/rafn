use anyhow::{Context, Result};
use clap::Args;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::api::IngestClient;
use crate::config_file::Config;
use crate::git;

#[derive(Args)]
pub struct IngestCommand {
    /// Repository name (auto-detected from git if not specified)
    #[arg(short, long, env = "RAFN_REPO")]
    repo: Option<String>,

    /// Commit SHA (auto-detected from git if not specified)
    #[arg(short, long, env = "RAFN_COMMIT")]
    commit: Option<String>,

    /// Branch name (auto-detected from git if not specified)
    #[arg(short, long, env = "RAFN_BRANCH")]
    branch: Option<String>,

    /// gRPC server URL (defaults to config file value)
    #[arg(long)]
    grpc_url: Option<String>,

    /// Parse and show results without submitting
    #[arg(long)]
    dry_run: bool,

    /// Benchmark files to ingest
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

impl IngestCommand {
    pub async fn execute(self) -> Result<()> {
        let config = Config::load()?;

        let (repository, commit_sha, branch) =
            git::GitInfo::resolve(self.repo.or(config.default_repo), self.commit, self.branch);

        let repo = repository?;
        let commit = commit_sha?;
        let grpc_url = self.grpc_url.unwrap_or(config.grpc_url);

        println!(
            "Ingesting {} file(s) to repository '{}'",
            self.files.len(),
            repo
        );
        println!("Commit: {}", commit);
        if let Some(branch_name) = &branch {
            println!("Branch: {}", branch_name);
        }
        println!();

        let mut client = if !self.dry_run {
            Some(IngestClient::connect(grpc_url).await?)
        } else {
            None
        };

        let mut success_count = 0;
        let mut error_count = 0;

        for file_path in &self.files {
            print!("Processing {}... ", file_path.display());

            match process_file(file_path, &repo, &commit, client.as_mut(), self.dry_run).await {
                Ok(count) => {
                    println!("ok ({} benchmark(s))", count);
                    success_count += count;
                }
                Err(e) => {
                    println!("failed");
                    eprintln!("  Error: {}", e);
                    error_count += 1;
                }
            }
        }

        println!();
        println!("Summary:");
        println!("  Files processed: {}/{}", success_count, self.files.len());
        if error_count > 0 {
            println!("  Errors: {}", error_count);
        }

        if error_count > 0 {
            std::process::exit(1);
        }

        Ok(())
    }
}

async fn process_file(
    file_path: &PathBuf,
    repository: &str,
    commit_sha: &str,
    client: Option<&mut IngestClient>,
    dry_run: bool,
) -> Result<usize> {
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let format =
        ingest::detect_format(&content).context("Could not auto-detect benchmark format")?;

    let parser = ingest::get_parser(
        &format,
        Uuid::nil(),
        repository.to_string(),
        commit_sha.to_string(),
    )?;

    let benchmarks = parser.parse(&content)?;
    let count = benchmarks.len();

    if count == 0 {
        anyhow::bail!("No benchmarks found in file");
    }

    if !dry_run
        && let Some(c) = client
    {
        c.submit(benchmarks).await?;
    }

    Ok(count)
}
