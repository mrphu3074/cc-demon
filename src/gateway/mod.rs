mod telegram_client;

use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

use crate::config::DemonConfig;
use crate::session::{SessionConfig, SessionManager};
use crate::task;

pub use telegram_client::TelegramClient;

/// Tracks an active Claude session for a chat
#[derive(Debug, Clone)]
struct ChatSession {
    session_id: String,
    last_message_at: chrono::DateTime<Utc>,
}

type SessionMap = Arc<Mutex<HashMap<i64, ChatSession>>>;

/// Shared state for the gateway, including optional persistent session manager.
struct GatewayState {
    config: DemonConfig,
    sessions: SessionMap,
    session_manager: Option<Arc<SessionManager>>,
}

pub async fn run(config: DemonConfig) -> Result<()> {
    tracing::info!(component = "gateway", "Starting Telegram gateway");

    if config.gateway.bot_token.is_empty() {
        anyhow::bail!("Telegram bot token is not configured");
    }

    // Initialize persistent session manager if enabled
    let session_manager = if config.gateway.use_persistent_session {
        tracing::info!(
            component = "gateway",
            tmux_session = %config.gateway.tmux_session_name,
            compact_interval_secs = config.gateway.compact_interval_secs,
            "Starting with persistent session"
        );

        let session_config = SessionConfig {
            session_name: config.gateway.tmux_session_name.clone(),
            prompt_marker: config.gateway.prompt_marker.clone(),
            poll_interval_ms: 200,
            response_timeout_secs: config.gateway.max_turns as u64 * 30 + 60,
            startup_timeout_secs: 60,
            compact_interval_secs: config.gateway.compact_interval_secs,
            max_restart_attempts: 3,
            model: config.gateway.default_model.clone(),
            max_turns: config.gateway.max_turns * 10, // Higher limit for persistent session
            max_budget_usd: config.gateway.max_budget_usd * 10.0, // Higher budget for persistent session
            allowed_tools: config.gateway.allowed_tools.clone(),
            disallowed_tools: config.gateway.disallowed_tools.clone(),
            append_system_prompt: config.gateway.append_system_prompt.clone(),
        };

        let manager = SessionManager::new(session_config)
            .await
            .context("Failed to initialize persistent session manager")?;

        tracing::info!(component = "gateway", "Persistent session manager initialized");
        Some(Arc::new(manager))
    } else {
        tracing::info!(
            component = "gateway",
            session_timeout_secs = config.gateway.session_timeout_secs,
            "Starting with spawn mode"
        );
        None
    };

    let bot = Bot::new(&config.gateway.bot_token);
    let sessions: SessionMap = Arc::new(Mutex::new(HashMap::new()));

    let state = Arc::new(GatewayState {
        config,
        sessions,
        session_manager,
    });

    tracing::info!(component = "gateway", "Telegram bot ready, waiting for messages");

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let state = state.clone();
        async move {
            tracing::debug!(component = "gateway", "Received raw message from Telegram");
            handle_message_with_state(bot, msg, &state).await;
            Ok(())
        }
    })
    .await;

    Ok(())
}

/// Handle message with gateway state (supports both persistent and spawn modes).
async fn handle_message_with_state(bot: Bot, msg: Message, state: &GatewayState) {
    let chat_id = msg.chat.id.0;

    // Check whitelist
    if !state.config.gateway.allowed_chat_ids.contains(&chat_id) {
        tracing::warn!(
            component = "gateway",
            chat_id = chat_id,
            "Message from non-whitelisted chat"
        );
        let _ = bot
            .send_message(msg.chat.id, "This chat is not authorized to use Demon.")
            .await;
        return;
    }

    let Some(text) = msg.text() else {
        return;
    };

    tracing::info!(
        component = "gateway",
        chat_id = chat_id,
        message_len = text.len(),
        "Received message"
    );

    // Send typing indicator continuously until Claude responds
    let typing_bot = bot.clone();
    let typing_chat_id = msg.chat.id;
    let typing_handle = tokio::spawn(async move {
        loop {
            let _ = typing_bot
                .send_chat_action(typing_chat_id, teloxide::types::ChatAction::Typing)
                .await;
            tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;
        }
    });

    // Check for /task prefix - route to task system
    if let Some(task_msg) = text.strip_prefix("/task ") {
        tracing::info!(
            component = "gateway",
            chat_id = chat_id,
            "Task command detected"
        );

        match task::classify_and_execute(
            task_msg.trim(),
            &state.config,
            state.session_manager.as_ref(),
        )
        .await
        {
            Ok(Some(response)) => {
                // Task executed successfully
                typing_handle.abort();

                tracing::info!(
                    component = "gateway",
                    chat_id = chat_id,
                    response_len = response.len(),
                    "Task executed successfully"
                );

                // Send response using TelegramClient
                let client =
                    TelegramClient::new(bot.clone(), state.config.gateway.message_format);
                if let Err(e) = client.send_formatted_message(msg.chat.id, &response).await {
                    tracing::error!(
                        component = "gateway",
                        chat_id = chat_id,
                        error = %e,
                        "Failed to send task response"
                    );
                    let _ = bot
                        .send_message(msg.chat.id, format!("Error sending response: {}", e))
                        .await;
                }
                return;
            }
            Ok(None) => {
                // No matching task - fall through to normal gateway
                tracing::debug!(
                    component = "gateway",
                    chat_id = chat_id,
                    "No matching task, falling back to gateway"
                );
            }
            Err(e) => {
                // Task execution failed
                typing_handle.abort();
                tracing::error!(
                    component = "gateway",
                    chat_id = chat_id,
                    error = %e,
                    "Task execution failed"
                );
                let _ = bot
                    .send_message(msg.chat.id, format!("Task error: {}", e))
                    .await;
                return;
            }
        }
    }

    // Use persistent session if available, otherwise fall back to spawn mode
    let result = if let Some(ref session_manager) = state.session_manager {
        tracing::debug!(
            component = "gateway",
            chat_id = chat_id,
            "Using persistent session"
        );
        session_manager
            .send_message(text)
            .await
            .map(|response| (response, None))
    } else {
        // Fall back to original spawn mode
        let existing_session = {
            let map = state.sessions.lock().await;
            map.get(&chat_id).cloned()
        };

        let resume_session_id = match existing_session {
            Some(ref session) => {
                let elapsed = (Utc::now() - session.last_message_at).num_seconds() as u64;
                if elapsed < state.config.gateway.session_timeout_secs {
                    tracing::debug!(
                        component = "gateway",
                        chat_id = chat_id,
                        session_id = %session.session_id,
                        idle_secs = elapsed,
                        "Resuming existing session"
                    );
                    Some(session.session_id.clone())
                } else {
                    tracing::debug!(
                        component = "gateway",
                        chat_id = chat_id,
                        idle_secs = elapsed,
                        timeout_secs = state.config.gateway.session_timeout_secs,
                        "Session expired, starting new"
                    );
                    None
                }
            }
            None => {
                tracing::debug!(
                    component = "gateway",
                    chat_id = chat_id,
                    "No existing session, starting new"
                );
                None
            }
        };

        execute_prompt(text, resume_session_id.as_deref(), &state.config, chat_id).await
    };

    // Stop typing indicator
    typing_handle.abort();

    match result {
        Ok((response, new_session_id)) => {
            // Update session tracking (only for spawn mode)
            if let Some(ref sid) = new_session_id {
                let mut map = state.sessions.lock().await;
                map.insert(
                    chat_id,
                    ChatSession {
                        session_id: sid.clone(),
                        last_message_at: Utc::now(),
                    },
                );
                tracing::debug!(
                    component = "gateway",
                    chat_id = chat_id,
                    session_id = %sid,
                    "Session stored"
                );
            }

            // Send formatted message using TelegramClient
            let client = TelegramClient::new(bot.clone(), state.config.gateway.message_format);
            if let Err(e) = client.send_formatted_message(msg.chat.id, &response).await {
                tracing::error!(
                    component = "gateway",
                    chat_id = chat_id,
                    error = %e,
                    "Failed to send formatted message"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                component = "gateway",
                chat_id = chat_id,
                error = %e,
                "Failed to execute prompt"
            );

            // If using spawn mode and resume failed, clear the session
            if state.session_manager.is_none() {
                let mut map = state.sessions.lock().await;
                map.remove(&chat_id);
                tracing::debug!(
                    component = "gateway",
                    chat_id = chat_id,
                    "Cleared stale session after error"
                );
            }

            let _ = bot
                .send_message(msg.chat.id, format!("Error: {e}"))
                .await;
        }
    }
}

/// Execute a prompt via claude CLI, optionally resuming a session.
/// Returns (response_text, Option<session_id>).
async fn execute_prompt(
    prompt: &str,
    resume_session_id: Option<&str>,
    config: &DemonConfig,
    chat_id: i64,
) -> Result<(String, Option<String>)> {
    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("-p");

    // Resume existing session if available
    if let Some(session_id) = resume_session_id {
        cmd.arg("--resume").arg(session_id);
    }

    if !config.gateway.default_model.is_empty() {
        cmd.arg("--model").arg(&config.gateway.default_model);
    }

    for tool in &config.gateway.allowed_tools {
        cmd.arg("--allowedTools").arg(tool);
    }

    for tool in &config.gateway.disallowed_tools {
        cmd.arg("--disallowedTools").arg(tool);
    }

    if !config.gateway.append_system_prompt.is_empty() {
        cmd.arg("--append-system-prompt")
            .arg(&config.gateway.append_system_prompt);
    }

    cmd.arg("--max-turns")
        .arg(config.gateway.max_turns.to_string());
    cmd.arg("--max-budget-usd")
        .arg(format!("{:.2}", config.gateway.max_budget_usd));

    // Always use JSON output to capture session_id
    cmd.arg("--output-format").arg("json");

    cmd.arg(prompt);

    tracing::debug!(
        component = "gateway",
        chat_id = chat_id,
        model = %config.gateway.default_model,
        resume_session = resume_session_id.is_some(),
        "Spawning claude CLI"
    );

    // Spawn with piped outputs so we can capture both
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd.spawn().context("Failed to spawn claude CLI")?;
    let pid = child.id().unwrap_or(0);

    tracing::debug!(
        component = "gateway",
        chat_id = chat_id,
        pid = pid,
        "Claude process started"
    );

    // Wait with timeout
    let timeout_secs = config.gateway.max_turns as u64 * 30 + 60;
    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await;

    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            anyhow::bail!("claude process error: {e}");
        }
        Err(_) => {
            // Timeout - kill the process by PID
            tracing::warn!(
                component = "gateway",
                chat_id = chat_id,
                pid = pid,
                timeout_secs = timeout_secs,
                "Claude process timed out, killing"
            );
            #[cfg(unix)]
            {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            anyhow::bail!("claude timed out after {timeout_secs}s");
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Log stderr for debugging
    if !stderr.is_empty() {
        tracing::debug!(
            component = "gateway",
            chat_id = chat_id,
            stderr = %stderr,
            "Claude stderr output"
        );
    }

    tracing::debug!(
        component = "gateway",
        chat_id = chat_id,
        status = %output.status,
        stdout_len = stdout.len(),
        "Claude process exited"
    );

    if output.status.success() {
        // Parse JSON to extract result and session_id
        match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(json) => {
                let result_text = json
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&stdout)
                    .to_string();
                let session_id = json
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if let Some(ref sid) = session_id {
                    tracing::debug!(
                        component = "gateway",
                        chat_id = chat_id,
                        session_id = %sid,
                        "Got session ID from response"
                    );
                }

                Ok((result_text, session_id))
            }
            Err(e) => {
                tracing::warn!(
                    component = "gateway",
                    chat_id = chat_id,
                    error = %e,
                    "Failed to parse JSON response, using raw output"
                );
                Ok((stdout.to_string(), None))
            }
        }
    } else {
        anyhow::bail!(
            "claude exited with {} | stderr: {} | stdout: {}",
            output.status,
            stderr.trim(),
            &stdout[..stdout.len().min(200)]
        )
    }
}
