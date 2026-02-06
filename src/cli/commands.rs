use anyhow::{Context, Result};
use std::io::Read;
use std::process::{Command, Stdio};

use crate::config::{DemonConfig, Job};
use crate::daemon;
use crate::gateway;
use crate::logging;
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
    let _guard = logging::init_foreground_logging()?;

    tracing::info!(component = "daemon", "Demon starting in foreground mode");

    let scheduler_handle = tokio::spawn({
        let config = config.clone();
        async move {
            if let Err(e) = scheduler::run(config).await {
                tracing::error!(component = "daemon", error = %e, "Scheduler error");
            }
        }
    });

    let gateway_handle = if with_gateway {
        Some(tokio::spawn({
            let config = config.clone();
            async move {
                if let Err(e) = gateway::run(config).await {
                    tracing::error!(component = "daemon", error = %e, "Gateway error");
                }
            }
        }))
    } else {
        None
    };

    tokio::signal::ctrl_c().await?;
    tracing::info!(component = "daemon", "Shutting down...");

    scheduler_handle.abort();
    if let Some(h) = gateway_handle {
        h.abort();
    }

    Ok(())
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

// ==================== Init Command ====================

const DEFAULT_CONFIG: &str = r#"# CC-Demon Configuration
# See: https://github.com/phunguyen/cc-demon for documentation

[paths]
# base_dir = "~/.demon"  # Uncomment to customize

[gateway]
enabled = false
bot_token = ""
allowed_chat_ids = []
default_model = "sonnet"
max_turns = 10
max_budget_usd = 5.0
allowed_tools = []
disallowed_tools = []
append_system_prompt = ""

[defaults]
model = "sonnet"
max_turns = 10
max_budget_usd = 5.0
output_format = "json"
"#;

const DEFAULT_JOBS: &str = r#"# Scheduled Jobs
# Add jobs here to run Claude Code on a schedule
# Example:
#
# [[jobs]]
# id = "daily-standup"
# name = "Daily Standup Summary"
# schedule_type = "recurring"
# schedule = "0 9 * * 1-5"  # Every weekday at 9am
# prompt = "Generate my daily standup summary based on git commits from yesterday"
# working_dir = "/path/to/project"
# model = "sonnet"
# output_destinations = ["file", "telegram:123456789"]
"#;

const DEFAULT_AGENTS: &str = r#"# Agent Definitions
# Define reusable agents for tasks
# Example:
#
# [[agents]]
# id = "code-reviewer"
# name = "Code Review Agent"
# working_dir = "/path/to/project"
# model = "sonnet"
# system_prompt = "You are a code review expert. Review code for bugs, performance issues, and best practices."
# allowed_tools = ["Read", "Grep", "Glob"]
"#;

const DEFAULT_TASKS: &str = r#"# Task Definitions
# Tasks are agent+prompt combinations that can be triggered
# Example:
#
# [[tasks]]
# id = "review-pr"
# agent_id = "code-reviewer"
# description = "Review a pull request for issues"
# prompt_template = "Review the PR: {{message}}"
# enabled = true
"#;

pub async fn init(with_gateway: bool) -> Result<()> {
    use std::io::{self, Write};

    let paths = crate::config::PathsConfig::default();
    let base_dir = paths.base_dir();

    println!("Initializing CC-Demon in {}...", base_dir.display());

    // Create base directory
    if !base_dir.exists() {
        std::fs::create_dir_all(&base_dir)?;
        println!("  Created {}", base_dir.display());
    } else {
        println!("  {} already exists", base_dir.display());
    }

    // Create subdirectories
    let subdirs = ["output", "logs"];
    for subdir in &subdirs {
        let path = base_dir.join(subdir);
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
            println!("  Created {}/", subdir);
        }
    }

    // Create config files (only if they don't exist)
    let files = [
        ("config.toml", DEFAULT_CONFIG),
        ("jobs.toml", DEFAULT_JOBS),
        ("agents.toml", DEFAULT_AGENTS),
        ("tasks.toml", DEFAULT_TASKS),
    ];

    for (filename, content) in &files {
        let path = base_dir.join(filename);
        if !path.exists() {
            std::fs::write(&path, content)?;
            println!("  Created {}", filename);
        } else {
            println!("  {} already exists (skipped)", filename);
        }
    }

    println!("\nInitialization complete!");

    // Interactive gateway setup
    if with_gateway {
        println!("\n--- Telegram Gateway Setup ---\n");
        println!("To set up the Telegram gateway, you'll need a bot token from @BotFather.\n");
        println!("Steps to create a Telegram bot:");
        println!("  1. Open Telegram and search for @BotFather");
        println!("  2. Send /newbot and follow the prompts");
        println!("  3. Copy the bot token (looks like: 123456:ABC-DEF...)");
        println!();

        print!("Enter your bot token (or press Enter to skip): ");
        io::stdout().flush()?;
        let mut token = String::new();
        io::stdin().read_line(&mut token)?;
        let token = token.trim();

        if !token.is_empty() {
            println!("\nNow you need to find your chat ID(s).");
            println!("To find your chat ID:");
            println!("  1. Send any message to your new bot");
            println!("  2. Open: https://api.telegram.org/bot{}/getUpdates", token);
            println!("  3. Look for \"chat\":{{\"id\":XXXXXXXX}} in the response");
            println!("  4. That number is your chat ID");
            println!();

            print!("Enter chat ID(s) (comma-separated, or press Enter to skip): ");
            io::stdout().flush()?;
            let mut chat_ids_input = String::new();
            io::stdin().read_line(&mut chat_ids_input)?;
            let chat_ids_input = chat_ids_input.trim();

            // Parse chat IDs
            let chat_ids: Vec<i64> = chat_ids_input
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            // Load existing config, update gateway settings, and save
            let config_path = paths.config_file();
            let mut config = if config_path.exists() {
                let content = std::fs::read_to_string(&config_path)?;
                toml::from_str(&content).unwrap_or_default()
            } else {
                crate::config::DemonConfig::default()
            };

            config.gateway.enabled = true;
            config.gateway.bot_token = token.to_string();
            if !chat_ids.is_empty() {
                config.gateway.allowed_chat_ids = chat_ids.clone();
            }

            let content = toml::to_string_pretty(&config)?;
            std::fs::write(&config_path, content)?;

            println!("\nGateway configured!");
            println!("  Bot token: {}...", &token[..token.len().min(20)]);
            if !chat_ids.is_empty() {
                println!("  Allowed chat IDs: {:?}", chat_ids);
            }
            println!("\nStart the daemon with gateway: demon start --with-gateway");
        } else {
            println!("\nSkipping gateway setup. You can configure it later with:");
            println!("  demon init --with-gateway");
            println!("  # or edit ~/.demon/config.toml directly");
        }
    } else {
        println!("\nTo configure Telegram gateway, run: demon init --with-gateway");
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

// ==================== Logs Command ====================

pub async fn logs(
    follow: bool,
    tail: Option<usize>,
    level: Option<String>,
    raw: bool,
) -> Result<()> {
    let log_file = logging::log_file_path();

    if !log_file.exists() {
        println!("No log file found at: {}", log_file.display());
        println!("\nLogs are created when the daemon runs.");
        println!("Start the daemon with: demon start");
        return Ok(());
    }

    // Show log file info
    let size = logging::log_size()?;
    if !raw && !follow {
        println!(
            "Log file: {} ({})",
            log_file.display(),
            logging::format_size(size)
        );
        println!();
    }

    if raw {
        // Raw mode: just cat or tail the file
        return logs_raw(&log_file, follow, tail).await;
    }

    // Check if hl is available
    if !is_hl_available() {
        println!("Error: 'hl' log viewer is not installed.");
        println!();
        println!("hl is a fast and powerful log viewer for JSON logs.");
        println!("Install it from: https://github.com/pamburus/hl");
        println!();
        println!("Installation options:");
        #[cfg(target_os = "macos")]
        println!("  brew install pamburus/tap/hl  (recommended)");
        println!("  cargo install hl");
        println!("  Download binary from GitHub releases");
        println!();
        println!("Or use --raw flag to view logs without hl:");
        println!("  demon logs --raw");
        return Ok(());
    }

    // Build hl command
    let mut cmd = Command::new("hl");

    // Add level filter if specified
    if let Some(ref lvl) = level {
        cmd.arg("--level").arg(lvl);
    }

    // Add follow flag
    if follow {
        cmd.arg("--follow");
    }

    // Add tail count
    if let Some(n) = tail {
        cmd.arg("--tail").arg(n.to_string());
    }

    // Add the log file path
    cmd.arg(&log_file);

    // Execute hl and let it take over stdout/stderr
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd.status().context("Failed to run hl")?;

    if !status.success() {
        anyhow::bail!("hl exited with status: {}", status);
    }

    Ok(())
}

/// Check if hl is available in PATH.
fn is_hl_available() -> bool {
    Command::new("hl")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Raw log output without hl.
async fn logs_raw(
    log_file: &std::path::Path,
    follow: bool,
    tail: Option<usize>,
) -> Result<()> {
    #[cfg(unix)]
    {
        if follow {
            // Use tail -f for following
            let mut cmd = Command::new("tail");
            cmd.arg("-f");
            if let Some(n) = tail {
                cmd.arg("-n").arg(n.to_string());
            }
            cmd.arg(log_file);

            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            let status = cmd.status().context("Failed to run tail")?;
            if !status.success() {
                anyhow::bail!("tail exited with status: {}", status);
            }
        } else if let Some(n) = tail {
            // Just tail the last N lines
            let mut cmd = Command::new("tail");
            cmd.arg("-n").arg(n.to_string());
            cmd.arg(log_file);

            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            let status = cmd.status().context("Failed to run tail")?;
            if !status.success() {
                anyhow::bail!("tail exited with status: {}", status);
            }
        } else {
            // Cat the entire file
            let content = std::fs::read_to_string(log_file)?;
            print!("{}", content);
        }
    }

    #[cfg(not(unix))]
    {
        use std::io::{BufRead, BufReader};

        if follow {
            anyhow::bail!("--raw --follow is not supported on Windows. Use hl instead, or omit --follow.");
        }

        let file = std::fs::File::open(log_file)?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;

        let lines_to_show = if let Some(n) = tail {
            let start = lines.len().saturating_sub(n);
            &lines[start..]
        } else {
            &lines[..]
        };

        for line in lines_to_show {
            println!("{}", line);
        }
    }

    Ok(())
}
