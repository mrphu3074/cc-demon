use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::formatter::MessageFormat;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemonConfig {
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub defaults: JobDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    pub base_dir: Option<String>,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self { base_dir: None }
    }
}

impl PathsConfig {
    pub fn base_dir(&self) -> PathBuf {
        if let Some(ref base) = self.base_dir {
            PathBuf::from(base)
        } else {
            dirs::home_dir()
                .expect("No home directory found")
                .join(".demon")
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.base_dir().join("config.toml")
    }

    pub fn jobs_file(&self) -> PathBuf {
        self.base_dir().join("jobs.toml")
    }

    pub fn output_dir(&self) -> PathBuf {
        self.base_dir().join("output")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.base_dir().join("logs")
    }

    pub fn pid_file(&self) -> PathBuf {
        self.base_dir().join("demon.pid")
    }

    pub fn agents_file(&self) -> PathBuf {
        self.base_dir().join("agents.toml")
    }

    pub fn tasks_file(&self) -> PathBuf {
        self.base_dir().join("tasks.toml")
    }

    #[allow(dead_code)]
    pub fn task_outputs_dir(&self) -> PathBuf {
        self.base_dir().join("task-outputs")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_budget")]
    pub max_budget_usd: f64,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub append_system_prompt: String,
    /// Seconds of inactivity before starting a new session (default: 3600 = 1 hour)
    /// Only used when use_persistent_session is false
    #[serde(default = "default_session_timeout")]
    pub session_timeout_secs: u64,
    /// Use persistent tmux session instead of spawning claude -p per message
    #[serde(default)]
    pub use_persistent_session: bool,
    /// Tmux session name for persistent session (default: "cc-demon-gateway")
    #[serde(default = "default_tmux_session_name")]
    pub tmux_session_name: String,
    /// Prompt marker to detect response completion (default: "> ")
    #[serde(default = "default_prompt_marker")]
    pub prompt_marker: String,
    /// Auto-compact interval in seconds (default: 3600 = 1 hour)
    #[serde(default = "default_compact_interval")]
    pub compact_interval_secs: u64,
    /// Message format for Telegram (markdownv2, html, plain)
    #[serde(default)]
    pub message_format: MessageFormat,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            allowed_chat_ids: Vec::new(),
            default_model: default_model(),
            max_turns: default_max_turns(),
            max_budget_usd: default_max_budget(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            append_system_prompt: String::new(),
            session_timeout_secs: default_session_timeout(),
            use_persistent_session: false,
            tmux_session_name: default_tmux_session_name(),
            prompt_marker: default_prompt_marker(),
            compact_interval_secs: default_compact_interval(),
            message_format: MessageFormat::default(),
        }
    }
}

fn default_tmux_session_name() -> String {
    "cc-demon-gateway".to_string()
}

fn default_prompt_marker() -> String {
    "❯".to_string() // Claude Code uses ❯ (U+276F) as prompt
}

fn default_compact_interval() -> u64 {
    3600 // 1 hour
}

fn default_session_timeout() -> u64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDefaults {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub fallback_model: String,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_budget")]
    pub max_budget_usd: f64,
    #[serde(default = "default_output_format")]
    pub output_format: String,
}

impl Default for JobDefaults {
    fn default() -> Self {
        Self {
            model: default_model(),
            fallback_model: String::new(),
            max_turns: default_max_turns(),
            max_budget_usd: default_max_budget(),
            output_format: default_output_format(),
        }
    }
}

fn default_model() -> String {
    "sonnet".to_string()
}

fn default_max_turns() -> u32 {
    10
}

fn default_max_budget() -> f64 {
    5.0
}

fn default_output_format() -> String {
    "json".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    #[serde(default = "default_recurring")]
    pub schedule_type: String,
    #[serde(default)]
    pub schedule: String,
    #[serde(default)]
    pub once_at: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub fallback_model: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub append_system_prompt: String,
    #[serde(default)]
    pub mcp_config: String,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_budget")]
    pub max_budget_usd: f64,
    #[serde(default = "default_output_format")]
    pub output_format: String,
    #[serde(default = "default_output_destinations")]
    pub output_destinations: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_recurring() -> String {
    "recurring".to_string()
}

fn default_output_destinations() -> Vec<String> {
    vec!["file".to_string()]
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
struct JobsFile {
    #[serde(default)]
    jobs: Vec<Job>,
}

impl DemonConfig {
    pub fn load() -> Result<Self> {
        let default_paths = PathsConfig::default();
        let config_file = default_paths.config_file();

        if config_file.exists() {
            let content = std::fs::read_to_string(&config_file)
                .context("Failed to read config file")?;
            toml::from_str(&content).context("Failed to parse config file")
        } else {
            Ok(Self::default())
        }
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        let config_file = self.paths.config_file();
        std::fs::create_dir_all(config_file.parent().unwrap())?;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_file, content)?;
        Ok(())
    }

    pub fn load_jobs(&self) -> Result<Vec<Job>> {
        let jobs_file = self.paths.jobs_file();
        if !jobs_file.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&jobs_file)
            .context("Failed to read jobs file")?;
        let file: JobsFile = toml::from_str(&content).context("Failed to parse jobs file")?;
        Ok(file.jobs)
    }

    pub fn save_jobs(&self, jobs: &[Job]) -> Result<()> {
        let jobs_file = self.paths.jobs_file();
        std::fs::create_dir_all(jobs_file.parent().unwrap())?;
        let file = JobsFile {
            jobs: jobs.to_vec(),
        };
        let content = toml::to_string_pretty(&file)?;
        std::fs::write(&jobs_file, content)?;
        Ok(())
    }
}

impl Default for DemonConfig {
    fn default() -> Self {
        Self {
            paths: PathsConfig::default(),
            gateway: GatewayConfig::default(),
            defaults: JobDefaults::default(),
        }
    }
}
