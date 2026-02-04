mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "demon", version, about = "Daemon scheduler and Telegram gateway for Claude Code")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the demon daemon
    Start {
        /// Start with Telegram gateway enabled
        #[arg(long)]
        with_gateway: bool,
        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the demon daemon
    Stop,
    /// Show daemon status and scheduled jobs
    Status,
    /// Manage scheduled jobs
    Job {
        #[command(subcommand)]
        action: JobAction,
    },
    /// Manage Telegram gateway
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// Install as system service (systemd/launchd)
    Install {
        /// Start with gateway enabled
        #[arg(long)]
        with_gateway: bool,
    },
    /// Uninstall system service
    Uninstall,
}

#[derive(Subcommand)]
pub enum JobAction {
    /// Add a new scheduled job (reads TOML from stdin)
    Add,
    /// List all jobs
    List,
    /// Remove a job by ID
    Remove {
        /// Job ID
        id: String,
    },
    /// Run a job immediately
    Run {
        /// Job ID
        id: String,
    },
    /// Enable a disabled job
    Enable {
        /// Job ID
        id: String,
    },
    /// Disable a job
    Disable {
        /// Job ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum GatewayAction {
    /// Start Telegram gateway
    Start,
    /// Stop Telegram gateway
    Stop,
    /// Show gateway status
    Status,
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Start {
            with_gateway,
            foreground,
        } => commands::start(with_gateway, foreground).await,
        Command::Stop => commands::stop().await,
        Command::Status => commands::status().await,
        Command::Job { action } => match action {
            JobAction::Add => commands::job_add().await,
            JobAction::List => commands::job_list().await,
            JobAction::Remove { id } => commands::job_remove(&id).await,
            JobAction::Run { id } => commands::job_run(&id).await,
            JobAction::Enable { id } => commands::job_toggle(&id, true).await,
            JobAction::Disable { id } => commands::job_toggle(&id, false).await,
        },
        Command::Gateway { action } => match action {
            GatewayAction::Start => commands::gateway_start().await,
            GatewayAction::Stop => commands::gateway_stop().await,
            GatewayAction::Status => commands::gateway_status().await,
        },
        Command::Install { with_gateway } => commands::install(with_gateway).await,
        Command::Uninstall => commands::uninstall().await,
    }
}
