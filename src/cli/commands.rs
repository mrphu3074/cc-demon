use anyhow::{Context, Result};
use std::io::Read;

use crate::config::{DemonConfig, Job};
use crate::daemon;
use crate::gateway;
use crate::scheduler;
use crate::task;

pub async fn start(with_gateway: bool, foreground: bool) -> Result<()> {
    let config = DemonConfig::load()?;

    if daemon::is_running()? {
        println!("Demon is already running (PID: {})", daemon::read_pid()?);
        return Ok(());
    }

    if foreground {
        println!("Starting demon in foreground...");
        run_foreground(config, with_gateway).await
    } else {
        println!("Starting demon daemon...");
        daemon::daemonize(with_gateway)?;
        println!("Demon started successfully");
        Ok(())
    }
}

async fn run_foreground(config: DemonConfig, with_gateway: bool) -> Result<()> {
    let _guard = init_logging(&config)?;

    tracing::info!("Demon starting in foreground mode");

    let scheduler_handle = tokio::spawn({
        let config = config.clone();
        async move {
            if let Err(e) = scheduler::run(config).await {
                tracing::error!("Scheduler error: {e}");
            }
        }
    });

    let gateway_handle = if with_gateway {
        Some(tokio::spawn({
            let config = config.clone();
            async move {
                if let Err(e) = gateway::run(config).await {
                    tracing::error!("Gateway error: {e}");
                }
            }
        }))
    } else {
        None
    };

    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down...");

    scheduler_handle.abort();
    if let Some(h) = gateway_handle {
        h.abort();
    }

    Ok(())
}

fn init_logging(config: &DemonConfig) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let log_dir = config.paths.logs_dir();
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "demon.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(non_blocking)
        .init();

    Ok(guard)
}

pub async fn stop() -> Result<()> {
    if !daemon::is_running()? {
        println!("Demon is not running");
        return Ok(());
    }

    let pid = daemon::read_pid()?;
    daemon::stop_daemon(pid)?;
    println!("Demon stopped (was PID: {pid})");
    Ok(())
}

pub async fn status() -> Result<()> {
    let config = DemonConfig::load()?;

    if daemon::is_running()? {
        let pid = daemon::read_pid()?;
        println!("Demon: running (PID: {pid})");
    } else {
        println!("Demon: stopped");
    }

    let jobs = config.load_jobs()?;
    if jobs.is_empty() {
        println!("Jobs: none configured");
    } else {
        println!("\nScheduled Jobs ({}):", jobs.len());
        println!("{:<20} {:<12} {:<10} {}", "ID", "Schedule", "Status", "Name");
        println!("{}", "-".repeat(70));
        for job in &jobs {
            let status = if job.enabled { "enabled" } else { "disabled" };
            let schedule_display = if job.schedule_type == "once" {
                job.once_at.as_deref().unwrap_or("pending")
            } else {
                &job.schedule
            };
            println!("{:<20} {:<12} {:<10} {}", job.id, schedule_display, status, job.name);
        }
    }

    // Show gateway status
    if config.gateway.enabled {
        println!("\nTelegram Gateway: configured");
        println!(
            "  Whitelisted chats: {}",
            config.gateway.allowed_chat_ids.len()
        );
    } else {
        println!("\nTelegram Gateway: not configured");
    }

    Ok(())
}

pub async fn job_add() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("Failed to read job definition from stdin")?;

    let job: Job = toml::from_str(&input).context("Invalid job TOML")?;

    let config = DemonConfig::load()?;
    let mut jobs = config.load_jobs()?;

    // Check for duplicate ID
    if jobs.iter().any(|j| j.id == job.id) {
        anyhow::bail!("Job with ID '{}' already exists", job.id);
    }

    println!("Added job: {} ({})", job.name, job.id);
    println!("  Schedule: {}", job.schedule);
    println!("  Prompt: {}...", &job.prompt[..job.prompt.len().min(60)]);

    jobs.push(job);
    config.save_jobs(&jobs)?;

    // Signal daemon to reload if running
    if daemon::is_running()? {
        let pid = daemon::read_pid()?;
        daemon::signal_reload(pid)?;
        println!("  Daemon notified to reload jobs");
    }

    Ok(())
}

pub async fn job_list() -> Result<()> {
    let config = DemonConfig::load()?;
    let jobs = config.load_jobs()?;

    if jobs.is_empty() {
        println!("No jobs configured");
        return Ok(());
    }

    for job in &jobs {
        let status = if job.enabled { "enabled" } else { "disabled" };
        println!("---");
        println!("ID:       {}", job.id);
        println!("Name:     {}", job.name);
        println!("Status:   {}", status);
        println!("Type:     {}", job.schedule_type);
        println!("Schedule: {}", job.schedule);
        if let Some(ref once_at) = job.once_at {
            println!("Run at:   {}", once_at);
        }
        println!("Model:    {}", job.model);
        println!("Prompt:   {}", job.prompt);
        println!(
            "Output:   {}",
            job.output_destinations.join(", ")
        );
    }

    Ok(())
}

pub async fn job_remove(id: &str) -> Result<()> {
    let config = DemonConfig::load()?;
    let mut jobs = config.load_jobs()?;

    let before = jobs.len();
    jobs.retain(|j| j.id != id);

    if jobs.len() == before {
        anyhow::bail!("Job '{}' not found", id);
    }

    config.save_jobs(&jobs)?;
    println!("Removed job: {}", id);

    if daemon::is_running()? {
        let pid = daemon::read_pid()?;
        daemon::signal_reload(pid)?;
    }

    Ok(())
}

pub async fn job_run(id: &str) -> Result<()> {
    let config = DemonConfig::load()?;
    let jobs = config.load_jobs()?;

    let job = jobs
        .iter()
        .find(|j| j.id == id)
        .context(format!("Job '{}' not found", id))?;

    println!("Running job: {} ({})", job.name, job.id);
    let result = scheduler::execute_job(job, &config).await?;

    println!("\n--- Output ---");
    println!("{}", result);

    Ok(())
}

pub async fn job_toggle(id: &str, enabled: bool) -> Result<()> {
    let config = DemonConfig::load()?;
    let mut jobs = config.load_jobs()?;

    let job = jobs
        .iter_mut()
        .find(|j| j.id == id)
        .context(format!("Job '{}' not found", id))?;

    job.enabled = enabled;
    let name = job.name.clone();
    config.save_jobs(&jobs)?;

    let action = if enabled { "Enabled" } else { "Disabled" };
    println!("{} job: {} ({})", action, name, id);

    if daemon::is_running()? {
        let pid = daemon::read_pid()?;
        daemon::signal_reload(pid)?;
    }

    Ok(())
}

pub async fn gateway_start() -> Result<()> {
    let config = DemonConfig::load()?;
    if config.gateway.bot_token.is_empty() {
        anyhow::bail!("Telegram bot token not configured. Run: demon config set gateway.bot_token <TOKEN>");
    }
    println!("Starting Telegram gateway...");
    gateway::run(config).await
}

pub async fn gateway_stop() -> Result<()> {
    println!("Telegram gateway can only be stopped by stopping the daemon");
    println!("Run: demon stop");
    Ok(())
}

pub async fn gateway_status() -> Result<()> {
    let config = DemonConfig::load()?;
    if config.gateway.bot_token.is_empty() {
        println!("Telegram Gateway: not configured (no bot token)");
    } else {
        println!("Telegram Gateway: configured");
        println!("  Allowed chats: {:?}", config.gateway.allowed_chat_ids);
        println!("  Model: {}", config.gateway.default_model);
        println!(
            "  Max budget: ${:.2}/message",
            config.gateway.max_budget_usd
        );
    }
    Ok(())
}

pub async fn install(with_gateway: bool) -> Result<()> {
    let exe_path = std::env::current_exe()?;
    let exe_str = exe_path.to_string_lossy();

    #[cfg(target_os = "linux")]
    {
        let gateway_flag = if with_gateway { " --with-gateway" } else { "" };
        let service = format!(
            r#"[Unit]
Description=CC-Demon - Claude Code Daemon Scheduler & Gateway
After=network.target

[Service]
Type=forking
ExecStart={exe_str} start{gateway_flag}
ExecStop={exe_str} stop
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
"#
        );

        let service_path = dirs::home_dir()
            .context("No home directory")?
            .join(".config/systemd/user/cc-demon.service");

        std::fs::create_dir_all(service_path.parent().unwrap())?;
        std::fs::write(&service_path, service)?;

        println!("Installed systemd user service at: {}", service_path.display());
        println!("Enable with: systemctl --user enable cc-demon");
        println!("Start with:  systemctl --user start cc-demon");
    }

    #[cfg(target_os = "macos")]
    {
        let gateway_arg = if with_gateway {
            "\n    <string>--with-gateway</string>"
        } else {
            ""
        };
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.cc-demon.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_str}</string>
        <string>start</string>
        <string>--foreground</string>{gateway_arg}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/cc-demon.out.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/cc-demon.err.log</string>
</dict>
</plist>
"#
        );

        let plist_path = dirs::home_dir()
            .context("No home directory")?
            .join("Library/LaunchAgents/com.cc-demon.daemon.plist");

        std::fs::write(&plist_path, plist)?;
        println!("Installed launchd service at: {}", plist_path.display());
        println!("Load with: launchctl load {}", plist_path.display());
    }

    #[cfg(target_os = "windows")]
    {
        println!("Windows service installation:");
        println!("  Use Task Scheduler to create a task that runs:");
        println!("  {} start --foreground{}", exe_str, if with_gateway { " --with-gateway" } else { "" });
        println!("  Set trigger: At log on");
        println!("  Set action: Start a program");
    }

    Ok(())
}

pub async fn uninstall() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let service_path = dirs::home_dir()
            .context("No home directory")?
            .join(".config/systemd/user/cc-demon.service");

        if service_path.exists() {
            std::fs::remove_file(&service_path)?;
            println!("Removed systemd service: {}", service_path.display());
            println!("Run: systemctl --user daemon-reload");
        } else {
            println!("No systemd service found");
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist_path = dirs::home_dir()
            .context("No home directory")?
            .join("Library/LaunchAgents/com.cc-demon.daemon.plist");

        if plist_path.exists() {
            println!("Unload with: launchctl unload {}", plist_path.display());
            std::fs::remove_file(&plist_path)?;
            println!("Removed launchd service: {}", plist_path.display());
        } else {
            println!("No launchd service found");
        }
    }

    #[cfg(target_os = "windows")]
    {
        println!("Remove the cc-demon task from Task Scheduler manually");
    }

    Ok(())
}

// ==================== Task Commands ====================

pub async fn task_run(task_name: &str, message: &str) -> Result<()> {
    let config = DemonConfig::load()?;

    // Use default message if empty
    let msg = if message.is_empty() {
        "Execute the task as configured"
    } else {
        message
    };

    println!("Running task: {}", task_name);
    let result = task::run_task_by_name(task_name, msg, &config).await?;

    println!("\n--- Output ---");
    println!("{}", result);

    Ok(())
}

pub async fn task_list() -> Result<()> {
    let config = DemonConfig::load()?;
    let tasks = task::load_tasks(&config)?;

    if tasks.is_empty() {
        println!("No tasks configured.");
        println!("\nCreate ~/.demon/tasks.toml with task definitions.");
        return Ok(());
    }

    println!("Configured Tasks ({}):", tasks.len());
    println!(
        "{:<20} {:<20} {:<10} {}",
        "ID", "Agent", "Status", "Description"
    );
    println!("{}", "-".repeat(80));

    for t in tasks {
        let status = if t.enabled { "enabled" } else { "disabled" };
        println!(
            "{:<20} {:<20} {:<10} {}",
            t.id,
            t.agent_id,
            status,
            t.description.chars().take(30).collect::<String>()
        );
    }

    Ok(())
}

pub async fn agent_list() -> Result<()> {
    let config = DemonConfig::load()?;
    let agents = task::load_agents(&config)?;

    if agents.is_empty() {
        println!("No agents configured.");
        println!("\nCreate ~/.demon/agents.toml with agent definitions.");
        return Ok(());
    }

    println!("Configured Agents ({}):", agents.len());
    println!(
        "{:<20} {:<15} {:<40} {}",
        "ID", "Model", "Working Dir", "Name"
    );
    println!("{}", "-".repeat(90));

    for a in agents {
        let working_dir = if a.working_dir.len() > 38 {
            format!("...{}", &a.working_dir[a.working_dir.len() - 35..])
        } else {
            a.working_dir.clone()
        };
        println!(
            "{:<20} {:<15} {:<40} {}",
            a.id, a.model, working_dir, a.name
        );
    }

    Ok(())
}
