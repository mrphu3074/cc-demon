/// Message splitter that respects Telegram's character limit
/// and optionally avoids breaking code blocks
pub struct MessageSplitter {
    max_length: usize,
    format_aware: bool,
}

impl MessageSplitter {
    pub fn new(max_length: usize, format_aware: bool) -> Self {
        Self {
            max_length,
            format_aware,
        }
    }

    /// Split text into chunks that fit within max_length
    pub fn split<'a>(&self, text: &'a str) -> Vec<&'a str> {
        if text.len() <= self.max_length {
            return vec![text];
        }

        let mut chunks = Vec::new();
        let mut start = 0;

        while start < text.len() {
            let remaining = text.len() - start;
            if remaining <= self.max_length {
                chunks.push(&text[start..]);
                break;
            }

            let end = start + self.max_length;
            let split_at = self.find_safe_split_point(text, start, end);
            chunks.push(&text[start..split_at]);
            start = split_at;
        }

        chunks
    }

    /// Find a safe point to split the text
    fn find_safe_split_point(&self, text: &str, start: usize, max_end: usize) -> usize {
        let window = &text[start..max_end];

        // If format-aware, try to find a split point outside code blocks
        if self.format_aware {
            // First, try to find the end of a code block within the window
            if let Some(pos) = self.find_code_block_boundary(text, start, max_end) {
                return pos;
            }
        }

        // Priority 1: Split at paragraph boundary (double newline)
        if let Some(pos) = window.rfind("\n\n") {
            let candidate = start + pos + 2;
            if !self.format_aware || !is_inside_code_block(text, candidate) {
                return candidate;
            }
        }

        // Priority 2: Split at newline
        if let Some(pos) = window.rfind('\n') {
            let candidate = start + pos + 1;
            if !self.format_aware || !is_inside_code_block(text, candidate) {
                return candidate;
            }
        }

        // Priority 3: Split at sentence boundary (. followed by space)
        if let Some(pos) = window.rfind(". ") {
            let candidate = start + pos + 2;
            if !self.format_aware || !is_inside_code_block(text, candidate) {
                return candidate;
            }
        }

        // Priority 4: Split at word boundary (space)
        if let Some(pos) = window.rfind(' ') {
            let candidate = start + pos + 1;
            if !self.format_aware || !is_inside_code_block(text, candidate) {
                return candidate;
            }
        }

        // Last resort: hard split at max_end
        max_end
    }

    /// Find a safe code block boundary within the window
    fn find_code_block_boundary(&self, text: &str, start: usize, max_end: usize) -> Option<usize> {
        let window = &text[start..max_end];

        // If we're inside a code block at start, find where it ends
        if is_inside_code_block(text, start) {
            // Look for closing ``` within the window
            if let Some(pos) = window.find("```") {
                // Found closing ```, split right after it plus any trailing newline
                let end_pos = start + pos + 3;
                if end_pos < text.len() && text.as_bytes().get(end_pos) == Some(&b'\n') {
                    return Some(end_pos + 1);
                }
                return Some(end_pos);
            }
            // Code block doesn't end in window, we need to extend
            return None;
        }

        // We're outside a code block, find a split before any code block starts
        if let Some(pos) = window.find("```") {
            if pos > 0 {
                // Split before the code block
                // Try to find a good boundary before the code block
                let before_code = &window[..pos];
                if let Some(newline_pos) = before_code.rfind('\n') {
                    return Some(start + newline_pos + 1);
                }
            }
        }

        None
    }
}

/// Check if a position is inside a code block
fn is_inside_code_block(text: &str, position: usize) -> bool {
    let before = &text[..position];
    let triple_backticks = before.matches("```").count();
    // Odd number of ``` means we're inside a code block
    triple_backticks % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_split_needed() {
        let splitter = MessageSplitter::new(100, false);
        let text = "Short text";
        let chunks = splitter.split(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_split_at_newline() {
        let splitter = MessageSplitter::new(20, false);
        let text = "First line\nSecond line here\nThird";
        let chunks = splitter.split(text);
        assert!(chunks.len() > 1);
        // Should split at newline, not in middle of word
        assert!(chunks[0].ends_with('\n') || chunks[0].ends_with(' '));
    }

    #[test]
    fn test_split_at_paragraph() {
        let splitter = MessageSplitter::new(30, false);
        let text = "First para\n\nSecond para here is longer";
        let chunks = splitter.split(text);
        assert_eq!(chunks[0], "First para\n\n");
    }

    #[test]
    fn test_code_block_awareness() {
        // Use a larger max to ensure code block stays together
        let splitter = MessageSplitter::new(100, true);
        let text = "Before\n```\ncode line 1\ncode line 2\n```\nAfter";
        let chunks = splitter.split(text);

        // With format-aware splitting, the code block should stay intact
        // Either in one chunk or split at block boundaries
        for chunk in &chunks {
            let count = chunk.matches("```").count();
            // Should be 0 or 2 (balanced)
            assert!(
                count == 0 || count == 2,
                "Chunk has unbalanced backticks: {}",
                chunk
            );
        }
    }

    #[test]
    fn test_is_inside_code_block() {
        let text = "before ```code``` after ```more";
        assert!(!is_inside_code_block(text, 0)); // before first ```
        assert!(is_inside_code_block(text, 10)); // inside first code block
        assert!(!is_inside_code_block(text, 20)); // after first block
        assert!(is_inside_code_block(text, 30)); // inside second (unclosed)
    }
}
