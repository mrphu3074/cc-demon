mod markdown_v2;
mod plain;
mod splitter;

pub use markdown_v2::MarkdownV2Formatter;
pub use plain::PlainFormatter;
pub use splitter::MessageSplitter;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Supported message formats for Telegram
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MessageFormat {
    /// Telegram MarkdownV2 format (default)
    #[default]
    #[serde(rename = "markdownv2")]
    MarkdownV2,
    /// HTML format
    Html,
    /// Plain text (no formatting)
    Plain,
}

impl MessageFormat {
    /// Returns the teloxide ParseMode string if applicable
    pub fn as_parse_mode(&self) -> Option<teloxide::types::ParseMode> {
        match self {
            Self::MarkdownV2 => Some(teloxide::types::ParseMode::MarkdownV2),
            Self::Html => Some(teloxide::types::ParseMode::Html),
            Self::Plain => None,
        }
    }
}

/// Trait for message formatters
pub trait Formatter: Send + Sync {
    /// Format text for the target platform
    fn format(&self, text: &str) -> Result<String>;

    /// Whether this formatter supports format-aware splitting
    fn supports_format_aware_split(&self) -> bool {
        false
    }
}

/// Create a formatter for the given message format
pub fn create_formatter(format: MessageFormat) -> Box<dyn Formatter> {
    match format {
        MessageFormat::MarkdownV2 => Box::new(MarkdownV2Formatter),
        MessageFormat::Html => Box::new(PlainFormatter), // TODO: HtmlFormatter
        MessageFormat::Plain => Box::new(PlainFormatter),
    }
}
