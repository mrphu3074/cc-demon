use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local, Utc};
use cron::Schedule;
use std::collections::HashSet;
use std::str::FromStr;
use tokio::time::{sleep, Duration};

use crate::config::{DemonConfig, Job};
use crate::output;

pub async fn run(config: DemonConfig) -> Result<()> {
    tracing::info!(component = "scheduler", "Scheduler started, checking jobs every 30 seconds");

    // Track which one-shot jobs have already been triggered
    let mut triggered_once_jobs: HashSet<String> = HashSet::new();

    loop {
        let jobs = match config.load_jobs() {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!(component = "scheduler", error = %e, "Failed to load jobs");
                sleep(Duration::from_secs(60)).await;
                continue;
            }
        };

        let now = Utc::now();
        let local_now = now.with_timezone(&Local);

        let total = jobs.len();
        let enabled = jobs.iter().filter(|j| j.enabled).count();
        let disabled = total - enabled;

        tracing::debug!(
            component = "scheduler",
            utc_time = %now.format("%Y-%m-%d %H:%M:%S"),
            local_time = %local_now.format("%Y-%m-%d %H:%M:%S %Z"),
            jobs_total = total,
            jobs_enabled = enabled,
            jobs_disabled = disabled,
            "Scheduler tick"
        );

        for job in &jobs {
            if !job.enabled {
                tracing::debug!(
                    component = "scheduler",
                    job_id = %job.id,
                    job_name = %job.name,
                    status = "skip",
                    reason = "disabled",
                    "Job skipped"
                );
                continue;
            }

            // Skip one-shot jobs that have already been triggered this session
            if job.schedule_type == "once" && triggered_once_jobs.contains(&job.id) {
                tracing::debug!(
                    component = "scheduler",
                    job_id = %job.id,
                    job_name = %job.name,
                    status = "skip",
                    reason = "once job already triggered",
                    "Job skipped"
                );
                continue;
            }

            let should_run = match job.schedule_type.as_str() {
                "recurring" => should_run_recurring(job, now),
                "once" => should_run_once(job, now),
                _ => {
                    tracing::error!(
                        component = "scheduler",
                        job_id = %job.id,
                        job_name = %job.name,
                        schedule_type = %job.schedule_type,
                        "Unknown schedule type"
                    );
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
                    tracing::info!(
                        component = "scheduler",
                        job_id = %job.id,
                        job_name = %job.name,
                        schedule_type = %job.schedule_type,
                        "Executing job"
                    );

                    match execute_job(&job, &config).await {
                        Ok(result) => {
                            tracing::info!(
                                component = "scheduler",
                                job_id = %job.id,
                                job_name = %job.name,
                                result_len = result.len(),
                                "Job completed successfully"
                            );
                            if let Err(e) = output::route(&job, &result, &config).await {
                                tracing::error!(
                                    component = "scheduler",
                                    job_id = %job.id,
                                    error = %e,
                                    "Failed to route output"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                component = "scheduler",
                                job_id = %job.id,
                                job_name = %job.name,
                                error = %e,
                                "Job execution failed"
                            );
                        }
                    }

                    // Disable one-shot jobs after execution
                    if job.schedule_type == "once" {
                        if let Ok(mut jobs) = config.load_jobs() {
                            if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
                                j.enabled = false;
                                if let Err(e) = config.save_jobs(&jobs) {
                                    tracing::error!(
                                        component = "scheduler",
                                        job_id = %job.id,
                                        error = %e,
                                        "Failed to disable one-shot job"
                                    );
                                } else {
                                    tracing::info!(
                                        component = "scheduler",
                                        job_id = %job.id,
                                        "One-shot job disabled after execution"
                                    );
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
            tracing::error!(
                component = "scheduler",
                job_id = %job.id,
                job_name = %job.name,
                cron = %job.schedule,
                error = %e,
                "Invalid cron expression"
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
            tracing::info!(
                component = "scheduler",
                job_id = %job.id,
                job_name = %job.name,
                cron = %job.schedule,
                status = "fire",
                "Job firing now"
            );
        } else {
            tracing::debug!(
                component = "scheduler",
                job_id = %job.id,
                job_name = %job.name,
                cron = %job.schedule,
                next_utc = %next.format("%Y-%m-%d %H:%M:%S UTC"),
                next_local = %next_local.format("%H:%M:%S %Z"),
                wait_duration = %format_duration(diff.num_seconds()),
                status = "wait",
                "Job waiting"
            );
        }
        should
    } else {
        tracing::error!(
            component = "scheduler",
            job_id = %job.id,
            job_name = %job.name,
            "No upcoming schedule found"
        );
        false
    }
}

fn should_run_once(job: &Job, now: chrono::DateTime<Utc>) -> bool {
    let Some(ref once_at) = job.once_at else {
        tracing::error!(
            component = "scheduler",
            job_id = %job.id,
            job_name = %job.name,
            "Once job missing once_at field"
        );
        return false;
    };

    let target = match parse_datetime(once_at) {
        Some(t) => t,
        None => {
            tracing::error!(
                component = "scheduler",
                job_id = %job.id,
                job_name = %job.name,
                once_at = %once_at,
                "Invalid once_at datetime format"
            );
            return false;
        }
    };

    let diff = (target - now).num_seconds();
    let should = diff <= 30 && diff >= -30;
    let target_local = target.with_timezone(&Local);

    if should {
        tracing::info!(
            component = "scheduler",
            job_id = %job.id,
            job_name = %job.name,
            once_at = %once_at,
            status = "fire",
            "Once job firing now"
        );
    } else if diff > 0 {
        tracing::debug!(
            component = "scheduler",
            job_id = %job.id,
            job_name = %job.name,
            target_utc = %target.format("%Y-%m-%d %H:%M:%S UTC"),
            target_local = %target_local.format("%H:%M:%S %Z"),
            wait_duration = %format_duration(diff),
            status = "wait",
            "Once job waiting"
        );
    } else {
        tracing::warn!(
            component = "scheduler",
            job_id = %job.id,
            job_name = %job.name,
            once_at = %once_at,
            missed_by = %format_duration(-diff),
            status = "miss",
            "Once job missed its scheduled time"
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

    tracing::debug!(
        component = "scheduler",
        job_id = %job.id,
        model = %job.model,
        working_dir = %job.working_dir,
        "Spawning claude CLI"
    );

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
