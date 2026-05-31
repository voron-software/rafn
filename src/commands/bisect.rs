//! `rafn bisect` — automatically find the commit that introduced a regression.
//!
//! This command is a stub. Implementation is planned for a future release.

use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct BisectCommand {
    /// Known-good commit SHA
    #[arg(long)]
    good: Option<String>,

    /// Known-bad commit SHA
    #[arg(long)]
    bad: Option<String>,

    /// Benchmark name to track during bisect
    #[arg(short, long)]
    name: Option<String>,

    /// The benchmark command to run at each bisect step
    #[arg(last = true)]
    command: Vec<String>,
}

impl BisectCommand {
    pub async fn execute(self) -> Result<()> {
        anyhow::bail!(
            "`rafn bisect` is not yet implemented. \
             It will automatically identify the commit that introduced a regression \
             by running the benchmark at successive commits between --good and --bad."
        )
    }
}
