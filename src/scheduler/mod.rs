use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local, Utc};
use cron::Schedule;
use std::collections::HashSet;
use std::str::FromStr;
use tokio::time::{sleep, Duration};

use crate::config::{DemonConfig, Job};
use crate::output;

pub async fn run(config: DemonConfig) -> Result<()> {
    tracing::info!("Scheduler started");
    eprintln!("[demon] Scheduler started, checking jobs every 30 seconds");

    // Track which one-shot jobs have already been triggered
    let mut triggered_once_jobs: HashSet<String> = HashSet::new();

    loop {
        let jobs = match config.load_jobs() {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!("Failed to load jobs: {e}");
                eprintln!("[demon] Failed to load jobs: {e}");
                sleep(Duration::from_secs(60)).await;
                continue;
            }
        };

        let now = Utc::now();
        let local_now = now.with_timezone(&Local);

        let total = jobs.len();
        let enabled = jobs.iter().filter(|j| j.enabled).count();
        let disabled = total - enabled;
        eprintln!(
            "[demon] Tick: UTC={} Local={} | jobs: {} total, {} enabled, {} disabled",
            now.format("%Y-%m-%d %H:%M:%S"),
            local_now.format("%Y-%m-%d %H:%M:%S %Z"),
            total,
            enabled,
            disabled,
        );

        for job in &jobs {
            if !job.enabled {
                eprintln!("[demon]   [SKIP] '{}' ({}): disabled", job.name, job.id);
                continue;
            }

            // Skip one-shot jobs that have already been triggered this session
            if job.schedule_type == "once" && triggered_once_jobs.contains(&job.id) {
                eprintln!("[demon]   [SKIP] '{}' ({}): once job already triggered this session", job.name, job.id);
                continue;
            }

            let should_run = match job.schedule_type.as_str() {
                "recurring" => should_run_recurring(job, now),
                "once" => should_run_once(job, now),
                _ => {
                    eprintln!("[demon]   [ERR]  '{}' ({}): unknown schedule_type '{}'", job.name, job.id, job.schedule_type);
                    false
                }
            };

            if should_run {
                // Mark one-shot jobs as triggered to prevent re-execution
                if job.schedule_type == "once" {
                    triggered_once_jobs.insert(job.id.clone());
                }

                let job = job.clone();
                let config = config.clone();
                tokio::spawn(async move {
                    eprintln!("[demon] Executing job: {} ({})", job.name, job.id);
                    tracing::info!("Executing job: {} ({})", job.name, job.id);
                    match execute_job(&job, &config).await {
                        Ok(result) => {
                            tracing::info!("Job '{}' completed successfully", job.id);
                            eprintln!("[demon] Job '{}' completed successfully", job.id);
                            if let Err(e) = output::route(&job, &result, &config).await {
                                tracing::error!("Failed to route output for job '{}': {e}", job.id);
                                eprintln!("[demon] Failed to route output for job '{}': {e}", job.id);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Job '{}' failed: {e}", job.id);
                            eprintln!("[demon] Job '{}' failed: {e}", job.id);
                        }
                    }

                    // Disable one-shot jobs after execution
                    if job.schedule_type == "once" {
                        if let Ok(mut jobs) = config.load_jobs() {
                            if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
                                j.enabled = false;
                                if let Err(e) = config.save_jobs(&jobs) {
                                    tracing::error!("Failed to disable one-shot job '{}': {e}", job.id);
                                } else {
                                    tracing::info!("One-shot job '{}' disabled after execution", job.id);
                                    eprintln!("[demon] One-shot job '{}' disabled after execution", job.id);
                                }
                            }
                        }
                    }
                });
            }
        }

        // Check every 30 seconds
        sleep(Duration::from_secs(30)).await;
    }
}

fn should_run_recurring(job: &Job, now: chrono::DateTime<Utc>) -> bool {
    let schedule = match Schedule::from_str(&job.schedule) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[demon]   [ERR]  '{}' ({}): invalid cron '{}': {e}",
                job.name, job.id, job.schedule
            );
            return false;
        }
    };

    // Check if there's an upcoming event within the next 30 seconds
    if let Some(next) = schedule.upcoming(Utc).next() {
        let diff = next - now;
        let next_local = next.with_timezone(&Local);
        let should = diff.num_seconds() <= 30 && diff.num_seconds() >= 0;
        if should {
            eprintln!(
                "[demon]   [FIRE] '{}' ({}): cron='{}' firing now",
                job.name, job.id, job.schedule,
            );
        } else {
            eprintln!(
                "[demon]   [WAIT] '{}' ({}): cron='{}' next={} (local {}) in {}",
                job.name,
                job.id,
                job.schedule,
                next.format("%Y-%m-%d %H:%M:%S UTC"),
                next_local.format("%H:%M:%S %Z"),
                format_duration(diff.num_seconds()),
            );
        }
        should
    } else {
        eprintln!(
            "[demon]   [ERR]  '{}' ({}): no upcoming schedule found",
            job.name, job.id
        );
        false
    }
}

fn should_run_once(job: &Job, now: chrono::DateTime<Utc>) -> bool {
    let Some(ref once_at) = job.once_at else {
        eprintln!(
            "[demon]   [ERR]  '{}' ({}): once job missing once_at field",
            job.name, job.id
        );
        return false;
    };

    let target = match parse_datetime(once_at) {
        Some(t) => t,
        None => {
            eprintln!(
                "[demon]   [ERR]  '{}' ({}): invalid once_at '{}'",
                job.name, job.id, once_at
            );
            return false;
        }
    };

    let diff = (target - now).num_seconds();
    let should = diff <= 30 && diff >= -30;
    let target_local = target.with_timezone(&Local);

    if should {
        eprintln!(
            "[demon]   [FIRE] '{}' ({}): once_at={} firing now",
            job.name, job.id, once_at,
        );
    } else if diff > 0 {
        eprintln!(
            "[demon]   [WAIT] '{}' ({}): once_at {} (local {}) in {}",
            job.name,
            job.id,
            target.format("%Y-%m-%d %H:%M:%S UTC"),
            target_local.format("%H:%M:%S %Z"),
            format_duration(diff),
        );
    } else {
        eprintln!(
            "[demon]   [MISS] '{}' ({}): once_at {} was {} ago (missed)",
            job.name,
            job.id,
            once_at,
            format_duration(-diff),
        );
    }

    should
}

fn format_duration(secs: i64) -> String {
    let secs = secs.unsigned_abs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Parse a datetime string in various formats:
/// - RFC 3339 / ISO 8601 with timezone: "2026-02-04T15:44:00+07:00"
/// - UTC with Z suffix: "2026-02-04T15:44:00Z"
/// - Naive local time: "2026-02-04T15:44:00"
fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC 3339 (handles +07:00, Z, etc.)
    if let Ok(dt) = DateTime::<FixedOffset>::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try parsing as UTC directly
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }

    // Try as naive local time
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        let local = Local::now().timezone();
        if let Some(local_dt) = naive.and_local_timezone(local).single() {
            return Some(local_dt.with_timezone(&Utc));
        }
    }

    // Try as naive local time with space separator
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        let local = Local::now().timezone();
        if let Some(local_dt) = naive.and_local_timezone(local).single() {
            return Some(local_dt.with_timezone(&Utc));
        }
    }

    None
}

pub async fn execute_job(job: &Job, _config: &DemonConfig) -> Result<String> {
    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("-p");

    // Model
    if !job.model.is_empty() {
        cmd.arg("--model").arg(&job.model);
    }

    // Fallback model
    if !job.fallback_model.is_empty() {
        cmd.arg("--fallback-model").arg(&job.fallback_model);
    }

    // Allowed tools
    for tool in &job.allowed_tools {
        cmd.arg("--allowedTools").arg(tool);
    }

    // Disallowed tools
    for tool in &job.disallowed_tools {
        cmd.arg("--disallowedTools").arg(tool);
    }

    // System prompt
    if !job.system_prompt.is_empty() {
        cmd.arg("--system-prompt").arg(&job.system_prompt);
    }

    // Append system prompt
    if !job.append_system_prompt.is_empty() {
        cmd.arg("--append-system-prompt").arg(&job.append_system_prompt);
    }

    // MCP config
    if !job.mcp_config.is_empty() {
        cmd.arg("--mcp-config").arg(&job.mcp_config);
    }

    // Max turns
    cmd.arg("--max-turns").arg(job.max_turns.to_string());

    // Max budget
    cmd.arg("--max-budget-usd")
        .arg(format!("{:.2}", job.max_budget_usd));

    // Output format
    cmd.arg("--output-format").arg(&job.output_format);

    // No session persistence for cron jobs
    cmd.arg("--no-session-persistence");

    // Working directory
    if !job.working_dir.is_empty() {
        cmd.current_dir(&job.working_dir);
    }

    // The prompt
    cmd.arg(&job.prompt);

    let output = cmd
        .output()
        .await
        .context("Failed to execute claude CLI")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI exited with {}: {}", output.status, stderr)
    }
}
