//! Tmux-based Claude session implementation using stream-json mode.
//!
//! Manages a persistent Claude Code process inside a tmux session,
//! using stream-json for reliable I/O instead of TUI parsing.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::{ClaudeSession, SessionConfig};

/// A Claude session running inside a tmux pane with stream-json I/O.
pub struct TmuxSession {
    config: SessionConfig,
    /// The tmux pane target (e.g., "cc-demon-session:0.0")
    pane_target: Arc<Mutex<Option<String>>>,
    /// The Claude process with stream-json I/O
    process: Arc<Mutex<Option<ClaudeProcess>>>,
    /// Current session ID from Claude
    session_id: Arc<Mutex<Option<String>>>,
}

/// Wrapper for the Claude process with stdin/stdout handles.
struct ClaudeProcess {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout_reader: BufReader<tokio::process::ChildStdout>,
}

impl TmuxSession {
    /// Create a new tmux session with Claude running inside using stream-json mode.
    pub async fn new(config: SessionConfig) -> Result<Self> {
        let session = Self {
            config,
            pane_target: Arc::new(Mutex::new(None)),
            process: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
        };

        session.spawn_session().await?;
        Ok(session)
    }

    /// Spawn the Claude process with stream-json I/O.
    async fn spawn_session(&self) -> Result<()> {
        let session_name = &self.config.session_name;

        // Kill any existing session with this name
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", session_name])
            .output()
            .await;

        // Build the claude command
        let mut cmd = Command::new("claude");
        cmd.arg("-p"); // Print mode (non-interactive)
        cmd.arg("--input-format").arg("stream-json");
        cmd.arg("--output-format").arg("stream-json");
        cmd.arg("--verbose"); // Required for stream-json output

        if !self.config.model.is_empty() {
            cmd.arg("--model").arg(&self.config.model);
        }

        cmd.arg("--max-turns")
            .arg(self.config.max_turns.to_string());

        cmd.arg("--max-budget-usd")
            .arg(format!("{:.2}", self.config.max_budget_usd));

        for tool in &self.config.allowed_tools {
            cmd.arg("--allowedTools").arg(tool);
        }

        for tool in &self.config.disallowed_tools {
            cmd.arg("--disallowedTools").arg(tool);
        }

        if !self.config.append_system_prompt.is_empty() {
            cmd.arg("--append-system-prompt")
                .arg(&self.config.append_system_prompt);
        }

        // Set up stdin/stdout pipes
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        eprintln!(
            "[demon] Starting Claude process with stream-json mode for session '{}'",
            session_name
        );

        let mut child = cmd.spawn().context("Failed to spawn Claude process")?;
        eprintln!("[demon] Claude process spawned, waiting for init...");

        let stdin = child.stdin.take().context("Failed to get stdin")?;
        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stdout_reader = BufReader::new(stdout);

        // Store the process
        *self.process.lock().await = Some(ClaudeProcess {
            child,
            stdin,
            stdout_reader,
        });

        // Wait for initialization (read until we get the init message)
        self.wait_for_init().await?;

        // Create a tmux session to track it (for visibility/debugging)
        let _ = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                session_name,
                "-x",
                "200",
                "-y",
                "50",
                "echo",
                &format!(
                    "Claude stream-json session active. Process managed by cc-demon."
                ),
            ])
            .output()
            .await;

        *self.pane_target.lock().await = Some(format!("{}:0.0", session_name));

        eprintln!("[demon] Claude stream-json session ready");
        Ok(())
    }

    /// Wait for Claude to initialize and capture session ID.
    /// Sends an initial ping message to trigger the init response.
    async fn wait_for_init(&self) -> Result<()> {
        let start = Instant::now();
        let timeout = Duration::from_secs(self.config.startup_timeout_secs);

        // First, drain any initial hook messages (non-blocking)
        {
            let mut process_guard = self.process.lock().await;
            let process = process_guard
                .as_mut()
                .context("No Claude process available")?;

            let mut line = String::new();

            // Read initial hook messages with short timeout
            loop {
                line.clear();
                let read_future = process.stdout_reader.read_line(&mut line);
                match tokio::time::timeout(Duration::from_secs(2), read_future).await {
                    Ok(Ok(0)) => anyhow::bail!("Claude process closed unexpectedly"),
                    Ok(Ok(_)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        eprintln!("[demon] Initial message: {}", trimmed.chars().take(80).collect::<String>());

                        // Check for session_id in any message
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if let Some(sid) = json.get("session_id").and_then(|v| v.as_str()) {
                                *self.session_id.lock().await = Some(sid.to_string());
                            }
                        }
                    }
                    Ok(Err(e)) => anyhow::bail!("Error reading from Claude: {}", e),
                    Err(_) => {
                        // Timeout - no more initial messages, proceed to send ping
                        eprintln!("[demon] Done reading initial messages, sending ping...");
                        break;
                    }
                }
            }

            // Send a simple ping message to trigger init
            let ping_msg = serde_json::json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": "ping"
                }
            });
            let json_str = serde_json::to_string(&ping_msg)?;
            process.stdin.write_all(json_str.as_bytes()).await?;
            process.stdin.write_all(b"\n").await?;
            process.stdin.flush().await?;
            eprintln!("[demon] Ping sent, waiting for init and response...");
        }

        // Now wait for init message and the ping response
        let mut got_init = false;
        let mut got_result = false;

        let mut process_guard = self.process.lock().await;
        let process = process_guard
            .as_mut()
            .context("No Claude process available")?;

        let mut line = String::new();

        loop {
            line.clear();
            let read_future = process.stdout_reader.read_line(&mut line);
            let result = tokio::time::timeout(Duration::from_secs(30), read_future).await;

            match result {
                Ok(Ok(0)) => anyhow::bail!("Claude process closed unexpectedly during init"),
                Ok(Ok(_)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        let msg_type = json.get("type").and_then(|v| v.as_str());

                        // Capture session ID
                        if let Some(session_id) = json.get("session_id").and_then(|v| v.as_str()) {
                            *self.session_id.lock().await = Some(session_id.to_string());
                        }

                        match msg_type {
                            Some("system") => {
                                let subtype = json.get("subtype").and_then(|v| v.as_str());
                                eprintln!("[demon] System message: {:?}", subtype);
                                if subtype == Some("init") {
                                    got_init = true;
                                    eprintln!("[demon] Got init message");
                                }
                            }
                            Some("result") => {
                                got_result = true;
                                eprintln!("[demon] Got ping result");
                            }
                            Some("assistant") => {
                                eprintln!("[demon] Got assistant message");
                            }
                            _ => {}
                        }

                        // Done when we have both init and result
                        if got_init && got_result {
                            eprintln!("[demon] Claude initialized with session_id: {:?}",
                                self.session_id.lock().await);
                            return Ok(());
                        }
                    }
                }
                Ok(Err(e)) => anyhow::bail!("Error reading from Claude: {}", e),
                Err(_) => {
                    if start.elapsed() > timeout {
                        anyhow::bail!("Timeout waiting for Claude to initialize");
                    }
                }
            }
        }
    }

    /// Send a JSON message to Claude.
    async fn send_json(&self, msg: &serde_json::Value) -> Result<()> {
        let mut process_guard = self.process.lock().await;
        let process = process_guard
            .as_mut()
            .context("No Claude process available")?;

        let json_str = serde_json::to_string(msg)?;
        process.stdin.write_all(json_str.as_bytes()).await?;
        process.stdin.write_all(b"\n").await?;
        process.stdin.flush().await?;

        Ok(())
    }

    /// Read JSON messages from Claude until we get a result.
    async fn read_until_result(&self, timeout: Duration) -> Result<String> {
        let start = Instant::now();

        let mut process_guard = self.process.lock().await;
        let process = process_guard
            .as_mut()
            .context("No Claude process available")?;

        let mut line = String::new();
        let mut result_text = String::new();

        loop {
            line.clear();
            let read_future = process.stdout_reader.read_line(&mut line);
            let result = tokio::time::timeout(Duration::from_secs(30), read_future).await;

            match result {
                Ok(Ok(0)) => anyhow::bail!("Claude process closed unexpectedly"),
                Ok(Ok(_)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Parse JSON
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        let msg_type = json.get("type").and_then(|v| v.as_str());

                        // Update session ID if present
                        if let Some(sid) = json.get("session_id").and_then(|v| v.as_str()) {
                            *self.session_id.lock().await = Some(sid.to_string());
                        }

                        match msg_type {
                            Some("result") => {
                                // Final result message
                                if let Some(result) = json.get("result").and_then(|v| v.as_str()) {
                                    return Ok(result.to_string());
                                }
                                // Check for error
                                if json.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
                                    let error = json
                                        .get("error")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Unknown error");
                                    anyhow::bail!("Claude error: {}", error);
                                }
                                return Ok(result_text);
                            }
                            Some("assistant") => {
                                // Assistant message - extract text content
                                if let Some(content) = json
                                    .get("message")
                                    .and_then(|m| m.get("content"))
                                    .and_then(|c| c.as_array())
                                {
                                    for block in content {
                                        if block.get("type").and_then(|t| t.as_str())
                                            == Some("text")
                                        {
                                            if let Some(text) =
                                                block.get("text").and_then(|t| t.as_str())
                                            {
                                                result_text = text.to_string();
                                            }
                                        }
                                    }
                                }
                            }
                            Some("system") | Some("user") => {
                                // System/user messages - skip
                            }
                            _ => {
                                // Unknown type - log and continue
                                tracing::debug!("Unknown message type: {:?}", msg_type);
                            }
                        }
                    }
                }
                Ok(Err(e)) => anyhow::bail!("Error reading from Claude: {}", e),
                Err(_) => {
                    if start.elapsed() > timeout {
                        anyhow::bail!(
                            "Timeout after {}s waiting for Claude response",
                            timeout.as_secs()
                        );
                    }
                    // Continue waiting
                }
            }
        }
    }

    /// Check if the Claude process is running.
    async fn process_alive(&self) -> bool {
        let mut process_guard = self.process.lock().await;
        if let Some(ref mut process) = *process_guard {
            // Try to check if process is still running
            match process.child.try_wait() {
                Ok(None) => true,  // Still running
                Ok(Some(_)) => false, // Exited
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Kill the current process and clean up.
    async fn kill_process(&self) -> Result<()> {
        let mut process_guard = self.process.lock().await;
        if let Some(ref mut process) = *process_guard {
            let _ = process.child.kill().await;
        }
        *process_guard = None;

        // Also kill tmux session
        let session_name = &self.config.session_name;
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", session_name])
            .output()
            .await;

        *self.pane_target.lock().await = None;
        *self.session_id.lock().await = None;

        Ok(())
    }
}

#[async_trait::async_trait]
impl ClaudeSession for TmuxSession {
    async fn send_message(&self, msg: &str) -> Result<String> {
        // Ensure process is alive
        if !self.process_alive().await {
            anyhow::bail!("Claude process is not alive");
        }

        // Build user message in stream-json format
        let user_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": msg
            }
        });

        // Send the message
        self.send_json(&user_msg).await?;

        // Wait for response
        let timeout = Duration::from_secs(self.config.response_timeout_secs);
        let response = self.read_until_result(timeout).await?;

        Ok(response)
    }

    async fn is_alive(&self) -> bool {
        self.process_alive().await
    }

    async fn restart(&self) -> Result<()> {
        tracing::info!("Restarting Claude process for session '{}'", self.config.session_name);

        // Kill existing process
        self.kill_process().await?;

        // Small delay before respawn
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Respawn
        self.spawn_session().await
    }

    async fn compact(&self) -> Result<()> {
        tracing::info!("Running /compact on session '{}'", self.config.session_name);
        // Compact is just a special message
        self.send_message("/compact").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_format() {
        let msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": "hello"
            }
        });
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"role\":\"user\""));
    }
}
