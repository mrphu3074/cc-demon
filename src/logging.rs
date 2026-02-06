//! Centralized logging configuration for cc-demon.
//!
//! Provides structured JSON logging to ~/.demon/logs/demon.jsonl,
//! compatible with the `hl` log viewer (https://github.com/pamburus/hl).

use anyhow::{Context, Result};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::PathsConfig;

/// Returns the path to the JSONL log file.
pub fn log_file_path() -> PathBuf {
    PathsConfig::default().logs_dir().join("demon.jsonl")
}

/// Initialize logging for daemon mode.
/// Logs structured JSON to ~/.demon/logs/demon.jsonl.
/// Returns a guard that must be held for the duration of the program.
pub fn init_daemon_logging() -> Result<WorkerGuard> {
    let paths = PathsConfig::default();
    let log_dir = paths.logs_dir();
    fs::create_dir_all(&log_dir).context("Failed to create logs directory")?;

    let log_file = log_file_path();

    // Open file for appending (create if doesn't exist)
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .context("Failed to open log file")?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .json()
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_writer(non_blocking)
        )
        .init();

    Ok(guard)
}

/// Initialize logging for foreground mode.
/// Logs structured JSON to file AND human-readable format to stderr.
/// Returns a guard that must be held for the duration of the program.
pub fn init_foreground_logging() -> Result<WorkerGuard> {
    let paths = PathsConfig::default();
    let log_dir = paths.logs_dir();
    fs::create_dir_all(&log_dir).context("Failed to create logs directory")?;

    let log_file = log_file_path();

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .context("Failed to open log file")?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .json()
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_writer(non_blocking)
        )
        .with(
            fmt::layer()
                .with_target(true)
                .with_writer(std::io::stderr)
        )
        .init();

    Ok(guard)
}

/// Rotate the log file by renaming current and starting fresh.
/// Returns the path to the rotated file.
#[allow(dead_code)]
pub fn rotate_log() -> Result<PathBuf> {
    let log_file = log_file_path();
    if !log_file.exists() {
        anyhow::bail!("No log file to rotate");
    }

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let rotated = log_file.with_extension(format!("{}.jsonl", timestamp));
    fs::rename(&log_file, &rotated).context("Failed to rotate log file")?;

    // Create empty new log file
    File::create(&log_file).context("Failed to create new log file")?;

    Ok(rotated)
}

/// Clear the log file (truncate to zero bytes).
#[allow(dead_code)]
pub fn clear_log() -> Result<()> {
    let log_file = log_file_path();
    if log_file.exists() {
        File::create(&log_file).context("Failed to clear log file")?;
    }
    Ok(())
}

/// Get the size of the current log file in bytes.
pub fn log_size() -> Result<u64> {
    let log_file = log_file_path();
    if log_file.exists() {
        Ok(fs::metadata(&log_file)?.len())
    } else {
        Ok(0)
    }
}

/// Format bytes as human-readable size.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}
