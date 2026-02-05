//! Agent & Task system for cc-demon.
//!
//! Provides LLM-based task classification and agent-based execution:
//! - Tasks define triggers with descriptions and keywords
//! - Agents define execution profiles with working directories and permissions
//! - Classifier uses existing gateway session for fast matching
//! - Executor spawns new claude -p with agent-specific configuration
//! - Router sends responses to Telegram and saves to files with path templating

use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::DemonConfig;
use crate::session::SessionManager;

/// Agent execution profile (from agents.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub working_dir: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub fallback_model: String,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_budget")]
    pub max_budget_usd: f64,
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
}

/// Task trigger definition (from tasks.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDefinition {
    pub id: String,
    pub name: String,
    pub agent_id: String,
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default = "default_output_file")]
    pub output_file: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentsFile {
    #[serde(default)]
    agents: Vec<AgentProfile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TasksFile {
    #[serde(default)]
    tasks: Vec<TaskDefinition>,
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

fn default_output_file() -> String {
    "{home}/.demon/task-outputs/{agent}/{date}_{time}_{task}.md".to_string()
}

fn default_true() -> bool {
    true
}

/// Load agents from agents.toml
pub fn load_agents(config: &DemonConfig) -> Result<Vec<AgentProfile>> {
    let path = config.paths.agents_file();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content =
        std::fs::read_to_string(&path).context("Failed to read agents file")?;

    let file: AgentsFile =
        toml::from_str(&content).context("Failed to parse agents file")?;

    Ok(file.agents)
}

/// Load tasks from tasks.toml
pub fn load_tasks(config: &DemonConfig) -> Result<Vec<TaskDefinition>> {
    let path = config.paths.tasks_file();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content =
        std::fs::read_to_string(&path).context("Failed to read tasks file")?;

    let file: TasksFile =
        toml::from_str(&content).context("Failed to parse tasks file")?;

    Ok(file.tasks)
}

/// Expand path template variables: {home}, {agent}, {task}, {date}, {time}
pub fn expand_path_template(
    template: &str,
    task_id: &str,
    agent_id: &str,
) -> PathBuf {
    let home = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .to_string();
    let now = Local::now();

    let expanded = template
        .replace("{home}", &home)
        .replace("{agent}", agent_id)
        .replace("{task}", task_id)
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string());

    // Also expand ~ at the start
    let expanded = if expanded.starts_with("~/") {
        expanded.replacen("~", &home, 1)
    } else {
        expanded
    };

    PathBuf::from(expanded)
}

/// Classify a message to find matching task using keyword matching first,
/// then LLM classification if no keyword match.
pub async fn classify_message(
    message: &str,
    tasks: &[TaskDefinition],
    session_manager: Option<&Arc<SessionManager>>,
) -> Result<Option<TaskDefinition>> {
    let enabled_tasks: Vec<_> = tasks.iter().filter(|t| t.enabled).collect();

    if enabled_tasks.is_empty() {
        return Ok(None);
    }

    let msg_lower = message.to_lowercase();

    // 1. Keyword-first classification (fast path)
    for task in &enabled_tasks {
        for keyword in &task.keywords {
            if msg_lower.contains(&keyword.to_lowercase()) {
                eprintln!(
                    "[demon] Task '{}' matched by keyword '{}'",
                    task.name, keyword
                );
                return Ok(Some((*task).clone()));
            }
        }
    }

    // 2. LLM classification (slow path) - only if persistent session available
    if let Some(session_mgr) = session_manager {
        let task_list = enabled_tasks
            .iter()
            .map(|t| {
                format!(
                    "- {}: {} (keywords: {})",
                    t.id,
                    t.description,
                    t.keywords.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let classification_prompt = format!(
            r#"Classify this user message to determine which task it matches.
Respond with ONLY the task ID or "none" if no clear match.

Available tasks:
{}

User message: "{}"

Task ID:"#,
            task_list, message
        );

        match session_mgr.send_message(&classification_prompt).await {
            Ok(response) => {
                let task_id = response.trim().to_lowercase();

                if task_id == "none" || task_id.is_empty() {
                    eprintln!("[demon] LLM classification returned 'none'");
                    return Ok(None);
                }

                // Find matching task
                if let Some(task) = enabled_tasks.iter().find(|t| t.id.to_lowercase() == task_id) {
                    eprintln!("[demon] Task '{}' matched by LLM classification", task.name);
                    return Ok(Some((*task).clone()));
                }

                eprintln!("[demon] LLM returned unknown task ID: {}", task_id);
            }
            Err(e) => {
                eprintln!("[demon] LLM classification failed: {}", e);
            }
        }
    }

    Ok(None)
}

/// Execute a task using the specified agent profile.
/// Spawns a new claude -p process with agent configuration.
pub async fn execute_task(
    task: &TaskDefinition,
    agent: &AgentProfile,
    message: &str,
    _config: &DemonConfig,
) -> Result<String> {
    tracing::info!(
        "Executing task '{}' with agent '{}': {}",
        task.name,
        agent.name,
        &message[..message.len().min(50)]
    );
    eprintln!(
        "[demon] Executing task '{}' with agent '{}' in {}",
        task.name, agent.name, agent.working_dir
    );

    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("-p");

    // Model
    if !agent.model.is_empty() {
        cmd.arg("--model").arg(&agent.model);
    }

    // Fallback model
    if !agent.fallback_model.is_empty() {
        cmd.arg("--fallback-model").arg(&agent.fallback_model);
    }

    // Allowed tools
    for tool in &agent.allowed_tools {
        cmd.arg("--allowedTools").arg(tool);
    }

    // Disallowed tools
    for tool in &agent.disallowed_tools {
        cmd.arg("--disallowedTools").arg(tool);
    }

    // System prompt
    if !agent.system_prompt.is_empty() {
        cmd.arg("--system-prompt").arg(&agent.system_prompt);
    }

    // Append system prompt
    if !agent.append_system_prompt.is_empty() {
        cmd.arg("--append-system-prompt")
            .arg(&agent.append_system_prompt);
    }

    // MCP config
    if !agent.mcp_config.is_empty() {
        cmd.arg("--mcp-config").arg(&agent.mcp_config);
    }

    // Max turns
    cmd.arg("--max-turns").arg(agent.max_turns.to_string());

    // Max budget
    cmd.arg("--max-budget-usd")
        .arg(format!("{:.2}", agent.max_budget_usd));

    // Output format - use json to parse result
    cmd.arg("--output-format").arg("json");

    // No session persistence for tasks
    cmd.arg("--no-session-persistence");

    // Working directory - expand ~ and template variables
    let working_dir = expand_path_template(&agent.working_dir, &task.id, &agent.id);
    if working_dir.exists() {
        cmd.current_dir(&working_dir);
    } else {
        eprintln!(
            "[demon] Warning: working_dir '{}' does not exist, using current dir",
            working_dir.display()
        );
    }

    // The user message becomes the prompt
    cmd.arg(message);

    // Set up I/O
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Spawn and wait
    let output = cmd.output().await.context("Failed to spawn claude CLI")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stderr.is_empty() {
        eprintln!("[demon] claude stderr: {}", stderr);
    }

    if output.status.success() {
        // Parse JSON to extract result
        match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(json) => {
                let result = json
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&stdout)
                    .to_string();
                Ok(result)
            }
            Err(_) => Ok(stdout.to_string()),
        }
    } else {
        anyhow::bail!(
            "Task execution failed with {}: {}",
            output.status,
            stderr.trim()
        )
    }
}

/// Save task response to file with path templating.
pub fn save_response(
    task: &TaskDefinition,
    agent: &AgentProfile,
    response: &str,
) -> Result<PathBuf> {
    let filepath = expand_path_template(&task.output_file, &task.id, &agent.id);

    // Create parent directories
    if let Some(parent) = filepath.parent() {
        std::fs::create_dir_all(parent).context("Failed to create output directory")?;
    }

    // Write with metadata header
    let content = format!(
        "# Task: {}\n# Agent: {}\n# Date: {}\n\n---\n\n{}",
        task.name,
        agent.name,
        Local::now().format("%Y-%m-%d %H:%M:%S"),
        response
    );

    std::fs::write(&filepath, content).context("Failed to write output file")?;

    eprintln!("[demon] Task output saved to: {}", filepath.display());
    Ok(filepath)
}

/// Main entry point for gateway: classify and execute a task.
/// Returns Ok(Some(response)) if task matched and executed,
/// Ok(None) if no task matched (fallback to normal gateway),
/// Err if execution failed.
pub async fn classify_and_execute(
    message: &str,
    config: &DemonConfig,
    session_manager: Option<&Arc<SessionManager>>,
) -> Result<Option<String>> {
    // Load configs
    let tasks = load_tasks(config)?;
    let agents = load_agents(config)?;

    if tasks.is_empty() || agents.is_empty() {
        eprintln!("[demon] No tasks or agents configured, falling back to gateway");
        return Ok(None);
    }

    // Classify
    let task = match classify_message(message, &tasks, session_manager).await? {
        Some(t) => t,
        None => {
            eprintln!("[demon] No matching task found, falling back to gateway");
            return Ok(None);
        }
    };

    // Find agent
    let agent = agents
        .iter()
        .find(|a| a.id == task.agent_id)
        .context(format!(
            "Agent '{}' not found for task '{}'",
            task.agent_id, task.id
        ))?;

    // Execute
    let response = execute_task(&task, agent, message, config).await?;

    // Save to file
    if let Err(e) = save_response(&task, agent, &response) {
        eprintln!("[demon] Warning: failed to save response: {}", e);
    }

    Ok(Some(response))
}

/// CLI entry point: run a specific task by name.
pub async fn run_task_by_name(
    task_name: &str,
    message: &str,
    config: &DemonConfig,
) -> Result<String> {
    let tasks = load_tasks(config)?;
    let agents = load_agents(config)?;

    // Find task by name or id
    let task = tasks
        .iter()
        .find(|t| t.name == task_name || t.id == task_name)
        .context(format!("Task '{}' not found", task_name))?;

    // Find agent
    let agent = agents
        .iter()
        .find(|a| a.id == task.agent_id)
        .context(format!(
            "Agent '{}' not found for task '{}'",
            task.agent_id, task.id
        ))?;

    // Execute
    let response = execute_task(task, agent, message, config).await?;

    // Save to file
    if let Err(e) = save_response(task, agent, &response) {
        eprintln!("[demon] Warning: failed to save response: {}", e);
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_path_template() {
        let path = expand_path_template(
            "{home}/.demon/output/{agent}/{date}_{task}.md",
            "my-task",
            "my-agent",
        );
        let path_str = path.to_string_lossy();

        assert!(path_str.contains("my-agent"));
        assert!(path_str.contains("my-task"));
        assert!(path_str.contains(".demon/output"));
    }

    #[test]
    fn test_expand_tilde() {
        let path = expand_path_template("~/projects/{agent}", "task", "agent");
        let path_str = path.to_string_lossy();

        // Should not start with ~/
        assert!(!path_str.starts_with("~/"));
    }
}
