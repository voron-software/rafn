use anyhow::Result;
use clap::{Parser, Subcommand};
use rafn::commands::{
    bench::BenchCommand, bisect::BisectCommand, compare::CompareCommand, config::ConfigCommand,
    init::InitCommand, push::PushCommand, trend::TrendCommand,
};
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

    /// Find the commit that introduced a regression (not yet implemented)
    Bisect(BisectCommand),

    /// Manage configuration
    Config(ConfigCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init(cmd) => cmd.execute().await,
        Commands::Bench(cmd) => cmd.execute().await,
        Commands::Push(cmd) => cmd.execute().await,
        Commands::Trend(cmd) => cmd.execute().await,
        Commands::Compare(cmd) => cmd.execute().await,
        Commands::Bisect(cmd) => cmd.execute().await,
        Commands::Config(cmd) => cmd.execute().await,
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
