use anyhow::Result;
use clap::{Parser, Subcommand};
use rafn::commands::{
    bench::BenchCommand, bisect::BisectCommand, compare::CompareCommand, config::ConfigCommand,
    push::PushCommand, trend::TrendCommand,
};

#[derive(Parser)]
#[command(name = "rafn")]
#[command(about = "Benchmark runner and uploader for perfscope")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run benchmarks, save a local snapshot, and show regressions
    Bench(BenchCommand),

    /// Upload local snapshots to the remote server
    Push(PushCommand),

    /// Show benchmark history over time
    Trend(TrendCommand),

    /// Compare benchmarks between two commits
    Compare(CompareCommand),

    /// Find the commit that introduced a regression (not yet implemented)
    Bisect(BisectCommand),

    /// Manage configuration
    Config(ConfigCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Bench(cmd) => cmd.execute().await,
        Commands::Push(cmd) => cmd.execute().await,
        Commands::Trend(cmd) => cmd.execute().await,
        Commands::Compare(cmd) => cmd.execute().await,
        Commands::Bisect(cmd) => cmd.execute().await,
        Commands::Config(cmd) => cmd.execute().await,
    }
}
