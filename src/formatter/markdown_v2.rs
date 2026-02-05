use anyhow::Result;

use super::Formatter;

/// MarkdownV2 formatter for Telegram
/// Converts standard Markdown to Telegram's MarkdownV2 format
pub struct MarkdownV2Formatter;

impl Formatter for MarkdownV2Formatter {
    fn format(&self, text: &str) -> Result<String> {
        Ok(convert_markdown_to_v2(text))
    }

    fn supports_format_aware_split(&self) -> bool {
        true
    }
}

/// Convert standard Markdown to Telegram MarkdownV2 format
///
/// Strategy:
/// 1. Extract and protect code blocks (they need no escaping inside)
/// 2. Process line by line to handle headers
/// 3. Convert **bold** to *bold*
/// 4. Preserve _italic_, `inline code`, and [links](url)
/// 5. Escape special characters only in plain text
fn convert_markdown_to_v2(text: &str) -> String {
    // Step 1: Extract code blocks and replace with placeholders
    let (text_without_code_blocks, code_blocks) = extract_code_blocks(text);

    // Step 2: Process the text
    let mut result = String::with_capacity(text.len() * 2);

    for line in text_without_code_blocks.lines() {
        if !result.is_empty() {
            result.push('\n');
        }

        // Check for placeholder (code block marker)
        if line.starts_with("<<<CODEBLOCK") && line.ends_with(">>>") {
            result.push_str(line);
            continue;
        }

        // Handle headers: # Header -> *Header* (bold)
        if let Some(header_text) = parse_header(line) {
            result.push('*');
            result.push_str(&process_inline_formatting(&header_text));
            result.push('*');
            continue;
        }

        // Handle blockquotes: > text -> (just remove the >)
        let line = if line.starts_with("> ") {
            &line[2..]
        } else if line.starts_with(">") {
            &line[1..]
        } else {
            line
        };

        // Process inline formatting
        result.push_str(&process_inline_formatting(line));
    }

    // Handle trailing newline if original had one
    if text.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }

    // Step 3: Restore code blocks
    restore_code_blocks(&mut result, &code_blocks);

    result
}

/// Extract code blocks and replace with placeholders
fn extract_code_blocks(text: &str) -> (String, Vec<String>) {
    let mut result = String::with_capacity(text.len());
    let mut code_blocks = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '`' && chars.peek() == Some(&'`') {
            chars.next(); // consume second `
            if chars.peek() == Some(&'`') {
                chars.next(); // consume third `

                // Collect the code block content
                let mut code_content = String::from("```");
                let mut found_end = false;

                while let Some(c) = chars.next() {
                    code_content.push(c);
                    if c == '`' && chars.peek() == Some(&'`') {
                        chars.next();
                        if chars.peek() == Some(&'`') {
                            chars.next();
                            code_content.push_str("``");
                            found_end = true;
                            break;
                        } else {
                            code_content.push('`');
                        }
                    }
                }

                if found_end {
                    let placeholder = format!("<<<CODEBLOCK{}>>>", code_blocks.len());
                    code_blocks.push(code_content);
                    result.push_str(&placeholder);
                } else {
                    // Unclosed code block - just add it as-is
                    result.push_str(&code_content);
                }
            } else {
                // Just two backticks
                result.push_str("``");
            }
        } else {
            result.push(ch);
        }
    }

    (result, code_blocks)
}

/// Restore code blocks from placeholders
fn restore_code_blocks(text: &mut String, code_blocks: &[String]) {
    for (i, block) in code_blocks.iter().enumerate() {
        let placeholder = format!("<<<CODEBLOCK{}>>>", i);
        *text = text.replace(&placeholder, block);
    }
}

/// Parse a header line, return the header text if it's a header
fn parse_header(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }

    // Count # characters
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    if hash_count == 0 || hash_count > 6 {
        return None;
    }

    let rest = &trimmed[hash_count..];
    if rest.is_empty() {
        return Some(String::new());
    }

    // Skip the space after #
    let text = if rest.starts_with(' ') {
        &rest[1..]
    } else {
        rest
    };

    Some(text.to_string())
}

/// Process inline formatting (bold, italic, code, links)
fn process_inline_formatting(text: &str) -> String {
    let mut result = String::with_capacity(text.len() * 2);
    let mut chars = text.chars().peekable();
    let mut in_inline_code = false;

    while let Some(ch) = chars.next() {
        match ch {
            // Inline code - no escaping needed inside
            '`' => {
                result.push('`');
                in_inline_code = !in_inline_code;
            }

            // Inside inline code - pass through as-is
            _ if in_inline_code => {
                result.push(ch);
            }

            // **bold** -> *bold*
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second *
                result.push('*');

                // Collect bold content until closing **
                let mut bold_content = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '*' {
                        chars.next();
                        if chars.peek() == Some(&'*') {
                            chars.next(); // consume closing **
                            break;
                        } else {
                            bold_content.push('*');
                        }
                    } else {
                        bold_content.push(chars.next().unwrap());
                    }
                }
                result.push_str(&escape_plain_text(&bold_content));
                result.push('*');
            }

            // __bold__ -> *bold*
            '_' if chars.peek() == Some(&'_') => {
                chars.next(); // consume second _
                result.push('*');

                // Collect bold content until closing __
                let mut bold_content = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '_' {
                        chars.next();
                        if chars.peek() == Some(&'_') {
                            chars.next(); // consume closing __
                            break;
                        } else {
                            bold_content.push('_');
                        }
                    } else {
                        bold_content.push(chars.next().unwrap());
                    }
                }
                result.push_str(&escape_plain_text(&bold_content));
                result.push('*');
            }

            // _italic_ -> _italic_
            '_' => {
                result.push('_');

                // Collect italic content until closing _
                let mut italic_content = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '_' {
                        chars.next();
                        break;
                    } else {
                        italic_content.push(chars.next().unwrap());
                    }
                }
                result.push_str(&escape_plain_text(&italic_content));
                result.push('_');
            }

            // *italic* -> _italic_
            '*' => {
                result.push('_');

                // Collect italic content until closing *
                let mut italic_content = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '*' {
                        chars.next();
                        break;
                    } else {
                        italic_content.push(chars.next().unwrap());
                    }
                }
                result.push_str(&escape_plain_text(&italic_content));
                result.push('_');
            }

            // [link text](url)
            '[' => {
                if let Some((link_text, url)) = try_parse_link(&mut chars) {
                    result.push('[');
                    result.push_str(&escape_link_text(&link_text));
                    result.push_str("](");
                    result.push_str(&escape_url(&url));
                    result.push(')');
                } else {
                    result.push_str(&escape_char('['));
                }
            }

            // Plain text - escape special characters
            _ => {
                result.push_str(&escape_char(ch));
            }
        }
    }

    result
}

/// Escape a single character if needed for MarkdownV2
fn escape_char(ch: char) -> String {
    // Characters that need escaping in MarkdownV2 plain text
    // Note: We don't escape * _ ` [ ] ( ) as they're handled by formatting logic
    match ch {
        '\\' => "\\\\".to_string(),
        '~' => "\\~".to_string(),
        '>' => "\\>".to_string(),
        '#' => "\\#".to_string(),
        '+' => "\\+".to_string(),
        '-' => "\\-".to_string(),
        '=' => "\\=".to_string(),
        '|' => "\\|".to_string(),
        '{' => "\\{".to_string(),
        '}' => "\\}".to_string(),
        '.' => "\\.".to_string(),
        '!' => "\\!".to_string(),
        _ => ch.to_string(),
    }
}

/// Escape plain text for use inside formatting
fn escape_plain_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 10);
    for ch in text.chars() {
        result.push_str(&escape_char(ch));
    }
    result
}

/// Escape text inside a markdown link [text]
fn escape_link_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 10);
    for ch in text.chars() {
        // In link text, escape ] and \
        if ch == ']' || ch == '\\' {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}

/// Escape characters in URL (only ) and \ need escaping)
fn escape_url(url: &str) -> String {
    let mut result = String::with_capacity(url.len() + 5);
    for ch in url.chars() {
        if ch == ')' || ch == '\\' {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}

/// Try to parse a markdown link starting after [
fn try_parse_link(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<(String, String)> {
    let mut link_text = String::new();

    // Collect link text until ]
    loop {
        match chars.peek() {
            Some(&']') => {
                chars.next();
                break;
            }
            Some(&c) => {
                link_text.push(c);
                chars.next();
            }
            None => return None,
        }
    }

    // Expect (
    if chars.peek() != Some(&'(') {
        return None;
    }
    chars.next();

    // Collect URL until )
    let mut url = String::new();
    let mut paren_depth = 1;
    loop {
        match chars.peek() {
            Some(&')') => {
                paren_depth -= 1;
                chars.next();
                if paren_depth == 0 {
                    break;
                }
                url.push(')');
            }
            Some(&'(') => {
                paren_depth += 1;
                url.push('(');
                chars.next();
            }
            Some(&c) => {
                url.push(c);
                chars.next();
            }
            None => return None,
        }
    }

    Some((link_text, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        let input = "Hello world";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_bold_conversion() {
        let input = "This is **bold** text";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "This is *bold* text");
    }

    #[test]
    fn test_underscore_bold() {
        let input = "This is __bold__ text";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "This is *bold* text");
    }

    #[test]
    fn test_italic() {
        let input = "This is _italic_ text";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "This is _italic_ text");
    }

    #[test]
    fn test_inline_code() {
        let input = "Use `cargo build` to compile";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "Use `cargo build` to compile");
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {\n    println!(\"Hello\");\n}\n```";
        let result = convert_markdown_to_v2(input);
        // Code inside block should not be escaped
        assert!(result.contains("println!(\"Hello\")"));
    }

    #[test]
    fn test_link() {
        let input = "Check out [Rust](https://rust-lang.org)";
        let result = convert_markdown_to_v2(input);
        assert!(result.contains("[Rust](https://rust-lang.org)"));
    }

    #[test]
    fn test_header() {
        let input = "# Header Text";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "*Header Text*");
    }

    #[test]
    fn test_list_items() {
        let input = "- Item 1\n- Item 2";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "\\- Item 1\n\\- Item 2");
    }

    #[test]
    fn test_numbered_list() {
        let input = "1. First\n2. Second";
        let result = convert_markdown_to_v2(input);
        assert_eq!(result, "1\\. First\n2\\. Second");
    }
}
