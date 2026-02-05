//! Persistent Claude Code session management via tmux.
//!
//! This module provides a way to maintain a long-running Claude Code session
//! in a tmux pane, enabling faster response times by avoiding the 5-10s startup
//! delay of spawning `claude -p` for each message.

mod manager;
mod tmux;

pub use manager::SessionManager;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// Trait for Claude session backends.
/// Abstracts session operations for testability and future extensibility.
#[async_trait::async_trait]
pub trait ClaudeSession: Send + Sync {
    /// Send a message to Claude and wait for the response.
    async fn send_message(&self, msg: &str) -> Result<String>;

    /// Check if the session is still alive.
    async fn is_alive(&self) -> bool;

    /// Restart the session (kill existing and create new).
    async fn restart(&self) -> Result<()>;

    /// Run the /compact command to reduce context size.
    async fn compact(&self) -> Result<()>;
}

/// Request sent through the message queue.
pub struct MessageRequest {
    pub prompt: String,
    pub response_tx: oneshot::Sender<Result<String>>,
}

/// Configuration for persistent session behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Tmux session name (default: "cc-demon-session")
    #[serde(default = "default_session_name")]
    pub session_name: String,

    /// Prompt marker to detect when Claude is ready for input (default: "> ")
    #[serde(default = "default_prompt_marker")]
    pub prompt_marker: String,

    /// Polling interval in milliseconds for checking response completion (default: 200)
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    /// Timeout in seconds for waiting for a response (default: 300)
    #[serde(default = "default_response_timeout_secs")]
    pub response_timeout_secs: u64,

    /// Timeout in seconds for session startup (default: 60)
    #[serde(default = "default_startup_timeout_secs")]
    pub startup_timeout_secs: u64,

    /// Auto-compaction interval in seconds (default: 3600 = 1 hour)
    #[serde(default = "default_compact_interval_secs")]
    pub compact_interval_secs: u64,

    /// Maximum restart attempts before giving up (default: 3)
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,

    /// Claude model to use (default: "sonnet")
    #[serde(default = "default_model")]
    pub model: String,

    /// Maximum turns per session (default: 100)
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Maximum budget in USD per session (default: 10.0)
    #[serde(default = "default_max_budget_usd")]
    pub max_budget_usd: f64,

    /// Tools to allow (empty = default)
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Tools to disallow
    #[serde(default)]
    pub disallowed_tools: Vec<String>,

    /// System prompt to append
    #[serde(default)]
    pub append_system_prompt: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            session_name: default_session_name(),
            prompt_marker: default_prompt_marker(),
            poll_interval_ms: default_poll_interval_ms(),
            response_timeout_secs: default_response_timeout_secs(),
            startup_timeout_secs: default_startup_timeout_secs(),
            compact_interval_secs: default_compact_interval_secs(),
            max_restart_attempts: default_max_restart_attempts(),
            model: default_model(),
            max_turns: default_max_turns(),
            max_budget_usd: default_max_budget_usd(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            append_system_prompt: String::new(),
        }
    }
}

fn default_session_name() -> String {
    "cc-demon-session".to_string()
}

fn default_prompt_marker() -> String {
    "❯".to_string() // Claude Code uses ❯ (U+276F) as prompt
}

fn default_poll_interval_ms() -> u64 {
    200
}

fn default_response_timeout_secs() -> u64 {
    300
}

fn default_startup_timeout_secs() -> u64 {
    60
}

fn default_compact_interval_secs() -> u64 {
    3600
}

fn default_max_restart_attempts() -> u32 {
    3
}

fn default_model() -> String {
    "sonnet".to_string()
}

fn default_max_turns() -> u32 {
    100
}

fn default_max_budget_usd() -> f64 {
    10.0
}
