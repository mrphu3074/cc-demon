use anyhow::{Context, Result};
use teloxide::prelude::*;
use teloxide::types::ChatId;

use crate::formatter::{create_formatter, MessageFormat, MessageSplitter};

/// Telegram client wrapper that handles message formatting and splitting
pub struct TelegramClient {
    bot: Bot,
    format: MessageFormat,
    max_chunk_size: usize,
}

impl TelegramClient {
    /// Create a new TelegramClient with the specified format
    pub fn new(bot: Bot, format: MessageFormat) -> Self {
        Self {
            bot,
            format,
            max_chunk_size: 4000, // Leave margin below 4096 Telegram limit
        }
    }

    /// Send a message with formatting, automatically splitting if needed
    /// Falls back to plain text if formatting fails
    pub async fn send_formatted_message(&self, chat_id: ChatId, text: &str) -> Result<()> {
        let formatter = create_formatter(self.format);

        // Format the text, with fallback to plain text on error
        let formatted = match formatter.format(text) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Formatting failed: {}, using plain text", e);
                eprintln!("[demon] Formatting failed: {}, falling back to plain text", e);
                text.to_string()
            }
        };

        // Split into chunks
        let splitter = MessageSplitter::new(
            self.max_chunk_size,
            formatter.supports_format_aware_split(),
        );
        let chunks = splitter.split(&formatted);

        // Send each chunk
        for chunk in chunks {
            let result = if let Some(parse_mode) = self.format.as_parse_mode() {
                self.bot
                    .send_message(chat_id, chunk)
                    .parse_mode(parse_mode)
                    .await
            } else {
                self.bot.send_message(chat_id, chunk).await
            };

            match result {
                Ok(_) => {}
                Err(e) => {
                    // If MarkdownV2 fails, try plain text
                    if self.format != MessageFormat::Plain {
                        tracing::warn!(
                            "Formatted send failed: {}, retrying as plain text",
                            e
                        );
                        eprintln!("[demon] Formatted send failed, retrying as plain text");
                        self.bot
                            .send_message(chat_id, chunk)
                            .await
                            .context("Failed to send message even as plain text")?;
                    } else {
                        return Err(e).context("Failed to send Telegram message");
                    }
                }
            }
        }

        Ok(())
    }
}
