use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rafn::commands::{
    bench::BenchCommand,
    bisect::{BisectCommand, BisectStepCommand},
    compare::CompareCommand,
    config::ConfigCommand,
    init::InitCommand,
    push::PushCommand,
    trend::TrendCommand,
};
use tracing::error;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "rafn")]
#[command(about = "Lightweight benchmark uploader")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold a repo for cloud-backed benchmark tracking
    Init(InitCommand),

    /// Run benchmarks, save a local snapshot, and show regressions
    Bench(BenchCommand),

    /// Upload local snapshots to the remote server
    Push(PushCommand),

    /// Show benchmark history over time
    Trend(TrendCommand),

    /// Compare benchmarks between two commits
    Compare(CompareCommand),

    /// Find the commit that introduced a regression via `git bisect`
    Bisect(BisectCommand),

    /// Classify the checked-out commit for `git bisect run` (internal)
    #[command(name = "bisect-step", hide = true)]
    BisectStep(BisectStepCommand),

    /// Manage configuration
    Config(ConfigCommand),
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let cli = Cli::parse();

    let result: Result<()> = match cli.command {
        Commands::Init(cmd) => cmd.execute().await,
        Commands::Bench(cmd) => cmd.execute().await,
        Commands::Push(cmd) => cmd.execute().await,
        Commands::Trend(cmd) => cmd.execute().await,
        Commands::Compare(cmd) => cmd.execute().await,
        // Bisect commands own their exit codes: `git bisect run` and the
        // documented `rafn bisect` contract both need more than success/failure.
        Commands::Bisect(cmd) => return cmd.execute(),
        Commands::BisectStep(cmd) => return cmd.execute(),
        Commands::Config(cmd) => cmd.execute().await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .init();
}
