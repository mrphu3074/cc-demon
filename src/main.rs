mod cli;
mod config;
mod daemon;
mod formatter;
mod gateway;
mod output;
mod scheduler;
mod session;
mod task;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle daemonization BEFORE starting tokio runtime
    // This is critical because forking with an active async runtime causes issues
    if let Command::Start {
        with_gateway,
        foreground: false,
    } = &cli.command
    {
        // Check if already running before daemonizing
        if daemon::is_running()? {
            println!("Demon is already running (PID: {})", daemon::read_pid()?);
            return Ok(());
        }
        println!("Starting demon daemon...");
        daemon::daemonize(*with_gateway)?;
        println!("Demon started successfully");
        return Ok(());
    }

    // For all other commands, use tokio runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(cli::run(cli))
}
