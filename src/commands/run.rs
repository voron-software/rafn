use anyhow::Result;
use clap::Args;
use std::path::PathBuf;
use uuid::Uuid;

use crate::config_file::Config;
use crate::ingest;
use crate::{api, discovery, framework, git, runner};

#[derive(Args)]
pub struct RunCommand {
    /// gRPC server URL for submitting results
    #[arg(long, env = "RAFN_GRPC_URL", default_value = "http://localhost:50051")]
    grpc_url: String,

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

    /// Parse results but don't submit
    #[arg(long)]
    dry_run: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Quiet mode - suppress output except errors
    #[arg(short, long)]
    quiet: bool,

    /// The benchmark command to execute
    #[arg(required = true, last = true)]
    command: Vec<String>,
}

impl RunCommand {
    pub async fn execute(self) -> Result<()> {
        let config = Config::load().unwrap_or_default();
        let grpc_url = if self.grpc_url == "http://localhost:50051" {
            config.grpc_url
        } else {
            self.grpc_url
        };

        let framework_config = framework::detect_framework(&self.command);
        if framework_config.is_none() && !self.quiet {
            eprintln!(
                "Warning: Unknown benchmark framework, will attempt to find Criterion results"
            );
        }

        let framework_config = framework_config.unwrap_or_else(|| framework::FrameworkConfig {
            framework: framework::Framework::Criterion,
            results_strategy: framework::ResultsStrategy::Directory(PathBuf::from(
                "target/criterion",
            )),
        });

        if !self.quiet {
            println!(
                "Running benchmark with {} framework detection...",
                framework_config.framework
            );
        }

        let result = runner::run_benchmark(&self.command, self.verbose)?;

        if !result.exit_status.success() && !self.quiet {
            eprintln!(
                "Benchmark exited with code {}",
                result.exit_status.code().unwrap_or(-1)
            );
        }

        let results = discovery::discover_results(
            &framework_config.results_strategy,
            self.results_dir.as_deref(),
        )?;

        if results.is_empty() {
            if !self.quiet {
                eprintln!("No benchmark results found");
            }
            std::process::exit(result.exit_status.code().unwrap_or(1));
        }

        if !self.quiet {
            println!("Found {} benchmark result(s)", results.len());
        }

        let (repository, commit, _branch) =
            git::GitInfo::resolve(self.repo, self.commit, self.branch);

        let repository = repository?;
        let commit = commit?;

        if !self.dry_run {
            let mut client = api::IngestClient::connect(grpc_url).await?;

            for bench in &results {
                if !self.quiet {
                    print!("Submitting {}... ", bench.name);
                }

                let json = serde_json::to_string(&bench.data)?;
                let format =
                    ingest::detect_format(&json).unwrap_or_else(|_| "criterion".to_string());
                let parser =
                    ingest::get_parser(&format, Uuid::nil(), repository.clone(), commit.clone());

                match parser {
                    Ok(p) => match p.parse(&json) {
                        Ok(benchmarks) => match client.submit(benchmarks).await {
                            Ok(_) => {
                                if !self.quiet {
                                    println!("ok");
                                }
                            }
                            Err(e) => {
                                if !self.quiet {
                                    println!("failed");
                                }
                                eprintln!("Warning: Failed to submit {}: {}", bench.name, e);
                            }
                        },
                        Err(e) => {
                            if !self.quiet {
                                println!("failed");
                            }
                            eprintln!("Warning: Failed to parse {}: {}", bench.name, e);
                        }
                    },
                    Err(e) => {
                        if !self.quiet {
                            println!("failed");
                        }
                        eprintln!("Warning: No parser for {}: {}", bench.name, e);
                    }
                }
            }
        } else if !self.quiet {
            println!("Dry run - skipping submission");
            for bench in &results {
                println!("  - {}", bench.name);
            }
        }

        std::process::exit(result.exit_status.code().unwrap_or(0));
    }
}
