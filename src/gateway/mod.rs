use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

use crate::config::DemonConfig;

/// Tracks an active Claude session for a chat
#[derive(Debug, Clone)]
struct ChatSession {
    session_id: String,
    last_message_at: chrono::DateTime<Utc>,
}

type SessionMap = Arc<Mutex<HashMap<i64, ChatSession>>>;

pub async fn run(config: DemonConfig) -> Result<()> {
    tracing::info!("Starting Telegram gateway");
    eprintln!("[demon] Starting Telegram gateway (session timeout: {}s)", config.gateway.session_timeout_secs);

    if config.gateway.bot_token.is_empty() {
        anyhow::bail!("Telegram bot token is not configured");
    }

    let bot = Bot::new(&config.gateway.bot_token);
    let sessions: SessionMap = Arc::new(Mutex::new(HashMap::new()));

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let config = config.clone();
        let sessions = sessions.clone();
        async move {
            handle_message(bot, msg, &config, &sessions).await;
            Ok(())
        }
    })
    .await;

    Ok(())
}

async fn handle_message(bot: Bot, msg: Message, config: &DemonConfig, sessions: &SessionMap) {
    let chat_id = msg.chat.id.0;

    // Check whitelist
    if !config.gateway.allowed_chat_ids.contains(&chat_id) {
        tracing::warn!("Message from non-whitelisted chat: {chat_id}");
        let _ = bot
            .send_message(msg.chat.id, "This chat is not authorized to use Demon.")
            .await;
        return;
    }

    let Some(text) = msg.text() else {
        return;
    };

    tracing::info!("Received message from chat {chat_id}: {text}");
    eprintln!("[demon] Chat {chat_id}: {text}");

    // Send typing indicator continuously until Claude responds
    // Telegram typing expires after ~5s, so we resend every 4s
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

    // Check for existing session
    let existing_session = {
        let map = sessions.lock().await;
        map.get(&chat_id).cloned()
    };

    let resume_session_id = match existing_session {
        Some(ref session) => {
            let elapsed = (Utc::now() - session.last_message_at).num_seconds() as u64;
            if elapsed < config.gateway.session_timeout_secs {
                eprintln!(
                    "[demon] Chat {chat_id}: resuming session {} (idle {}s)",
                    session.session_id, elapsed
                );
                Some(session.session_id.clone())
            } else {
                eprintln!(
                    "[demon] Chat {chat_id}: session expired (idle {}s > {}s threshold), starting new",
                    elapsed, config.gateway.session_timeout_secs
                );
                None
            }
        }
        None => {
            eprintln!("[demon] Chat {chat_id}: no existing session, starting new");
            None
        }
    };

    // Execute via claude CLI
    let result = execute_prompt(text, resume_session_id.as_deref(), config).await;

    // Stop typing indicator
    typing_handle.abort();

    match result {
        Ok((response, new_session_id)) => {
            // Update session tracking
            if let Some(sid) = new_session_id {
                let mut map = sessions.lock().await;
                map.insert(
                    chat_id,
                    ChatSession {
                        session_id: sid.clone(),
                        last_message_at: Utc::now(),
                    },
                );
                eprintln!("[demon] Chat {chat_id}: session stored: {sid}");
            }

            // Telegram has a 4096 char limit per message
            for chunk in split_message(&response, 4000) {
                let _ = bot.send_message(msg.chat.id, chunk).await;
            }
        }
        Err(e) => {
            tracing::error!("Failed to execute prompt: {e}");
            eprintln!("[demon] Chat {chat_id}: error: {e}");

            // If resume failed, clear the session and hint the user
            if resume_session_id.is_some() {
                let mut map = sessions.lock().await;
                map.remove(&chat_id);
                eprintln!("[demon] Chat {chat_id}: cleared stale session after error");
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

    // Log the full command for debugging
    eprintln!("[demon] Spawning: claude {}", build_args_debug(&cmd));

    // Spawn with piped outputs so we can capture both
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd.spawn().context("Failed to spawn claude CLI")?;
    let pid = child.id().unwrap_or(0);
    eprintln!("[demon] claude process started (PID: {pid})");

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
            eprintln!("[demon] claude process timed out after {timeout_secs}s, killing PID {pid}");
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

    // Always log stderr for debugging
    if !stderr.is_empty() {
        eprintln!("[demon] claude stderr: {stderr}");
    }

    eprintln!(
        "[demon] claude exited with status: {} (stdout: {} bytes)",
        output.status,
        stdout.len()
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
                    eprintln!("[demon] session_id: {sid}");
                }
                Ok((result_text, session_id))
            }
            Err(e) => {
                eprintln!("[demon] Failed to parse JSON response: {e}");
                eprintln!("[demon] Raw stdout: {stdout}");
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

fn build_args_debug(cmd: &tokio::process::Command) -> String {
    format!("{:?}", cmd)
        .replace("Command { std: ", "")
        .chars()
        .take(500)
        .collect()
}

fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let end = (start + max_len).min(text.len());

        // Try to split at a newline if possible
        let split_at = if end < text.len() {
            text[start..end]
                .rfind('\n')
                .map(|i| start + i + 1)
                .unwrap_or(end)
        } else {
            end
        };

        chunks.push(&text[start..split_at]);
        start = split_at;
    }

    chunks
}
