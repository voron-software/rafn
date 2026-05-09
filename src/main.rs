use anyhow::Result;
use clap::{Parser, Subcommand};
use rafn::commands::{
    compare::CompareCommand, config::ConfigCommand, export::ExportCommand, ingest::IngestCommand,
    query::QueryCommand, run::RunCommand, trend::TrendCommand,
};

#[derive(Parser)]
#[command(name = "rafn")]
#[command(about = "Benchmark uploader for perfscope")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run benchmarks and auto-submit results
    Run(RunCommand),

    /// Manually ingest benchmark files
    Ingest(IngestCommand),

    /// Query benchmarks from the server
    Query(QueryCommand),

    /// Compare benchmarks between two commits
    Compare(CompareCommand),

    /// Show time-series trend data for a benchmark
    Trend(TrendCommand),

    /// Export benchmark data
    Export(ExportCommand),

    /// Manage configuration
    Config(ConfigCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(cmd) => cmd.execute().await,
        Commands::Ingest(cmd) => cmd.execute().await,
        Commands::Query(cmd) => cmd.execute().await,
        Commands::Compare(cmd) => cmd.execute().await,
        Commands::Trend(cmd) => cmd.execute().await,
        Commands::Export(cmd) => cmd.execute().await,
        Commands::Config(cmd) => cmd.execute().await,
    }
}
