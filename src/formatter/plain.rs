use anyhow::Result;

use super::Formatter;

/// Plain text formatter - passes text through without modification
pub struct PlainFormatter;

impl Formatter for PlainFormatter {
    fn format(&self, text: &str) -> Result<String> {
        Ok(text.to_string())
    }
}
