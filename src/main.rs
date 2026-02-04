mod cli;
mod config;
mod daemon;
mod gateway;
mod output;
mod scheduler;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    cli::run(cli).await
}
