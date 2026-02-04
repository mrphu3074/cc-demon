use anyhow::{Context, Result};
use chrono::Local;

use crate::config::{DemonConfig, Job};

pub async fn route(job: &Job, result: &str, config: &DemonConfig) -> Result<()> {
    for dest in &job.output_destinations {
        match dest.as_str() {
            "file" => save_to_file(job, result, config)?,
            d if d.starts_with("telegram:") => {
                let chat_id: i64 = d
                    .strip_prefix("telegram:")
                    .unwrap()
                    .parse()
                    .context("Invalid Telegram chat ID in output destination")?;
                send_to_telegram(job, result, chat_id, config).await?;
            }
            other => {
                tracing::warn!("Unknown output destination '{}' for job '{}'", other, job.id);
            }
        }
    }
    Ok(())
}

fn save_to_file(job: &Job, result: &str, config: &DemonConfig) -> Result<()> {
    let output_dir = config.paths.output_dir().join(&job.id);
    std::fs::create_dir_all(&output_dir)?;

    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let filename = format!("{}.md", timestamp);
    let filepath = output_dir.join(&filename);

    let content = format!(
        "# Job: {}\n\nDate: {}\nPrompt: {}\n\n---\n\n{}",
        job.name,
        Local::now().format("%Y-%m-%d %H:%M:%S"),
        job.prompt,
        extract_result(result)
    );

    std::fs::write(&filepath, content)?;
    tracing::info!("Output saved to: {}", filepath.display());
    Ok(())
}

async fn send_to_telegram(
    job: &Job,
    result: &str,
    chat_id: i64,
    config: &DemonConfig,
) -> Result<()> {
    if config.gateway.bot_token.is_empty() {
        tracing::warn!(
            "Cannot send to Telegram: bot token not configured (job: {})",
            job.id
        );
        return Ok(());
    }

    let bot = teloxide::Bot::new(&config.gateway.bot_token);
    let text = format!("**Job: {}**\n\n{}", job.name, extract_result(result));

    // Split if needed (Telegram 4096 char limit)
    let chunks = split_message(&text, 4000);
    for chunk in chunks {
        teloxide::requests::Requester::send_message(
            &bot,
            teloxide::types::ChatId(chat_id),
            chunk.to_string(),
        )
        .await
        .context("Failed to send Telegram message")?;
    }

    tracing::info!("Output sent to Telegram chat: {}", chat_id);
    Ok(())
}

fn extract_result(result: &str) -> &str {
    // If output is JSON format, try to extract the result field
    // Otherwise return as-is
    if result.trim_start().starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(result) {
            if v.get("result").and_then(|r| r.as_str()).is_some() {
                return result;
            }
        }
    }
    result
}

fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let end = (start + max_len).min(text.len());
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
