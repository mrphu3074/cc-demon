# CC-Demon Configuration Reference

## Full Config Example (`~/.demon/config.toml`)

```toml
[paths]
# Base directory for all demon data (default: ~/.demon)
# base_dir = "/custom/path"

[gateway]
# Enable Telegram gateway
enabled = true
# Bot token from @BotFather
bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
# Whitelisted chat IDs (positive = DM, negative = group)
allowed_chat_ids = [123456789, -987654321]
# Default model for gateway responses
default_model = "sonnet"
# Max agentic turns per message
max_turns = 10
# Max USD spend per message
max_budget_usd = 5.0
# Tools gateway sessions can use
allowed_tools = ["Read", "Grep", "Glob", "Bash(git *)"]
# Tools to block
disallowed_tools = ["Bash(rm *)", "Bash(sudo *)", "Write", "Edit"]
# Additional system prompt for gateway
append_system_prompt = "Keep responses concise. Format for Telegram readability."

[defaults]
# Default model for new jobs
model = "sonnet"
# Default fallback model
fallback_model = ""
# Default max turns per job
max_turns = 10
# Default max budget per job
max_budget_usd = 5.0
# Default output format (json or text)
output_format = "json"
```

## Job Definition Fields (`~/.demon/jobs.toml`)

```toml
[[jobs]]
# Required fields
id = "unique-job-id"           # Unique identifier (kebab-case)
name = "Human Readable Name"    # Display name
prompt = "The prompt text"      # Prompt sent to claude -p

# Schedule (one of these patterns)
schedule_type = "recurring"     # "recurring" or "once"
schedule = "0 9 * * 1-5"       # Cron expression (for recurring)
once_at = "2025-01-15T09:00:00" # ISO 8601 datetime (for once)

# Optional fields with defaults
working_dir = "/path/to/project"  # Working directory for claude session
model = "sonnet"                  # Model to use
fallback_model = ""               # Fallback if primary unavailable
allowed_tools = ["Read", "Grep"]  # Tools to pre-approve
disallowed_tools = ["Bash(rm *)"] # Tools to block
system_prompt = ""                # Replace default system prompt
append_system_prompt = ""         # Append to default system prompt
mcp_config = ""                   # Path to MCP config JSON
max_turns = 10                    # Max agentic turns
max_budget_usd = 5.0              # Max USD spend
output_format = "json"            # Output format (json or text)
output_destinations = ["file"]    # Where to send output
enabled = true                    # Whether job is active
```

### Output Destinations

- `"file"` - Save to `~/.demon/output/<job-id>/<timestamp>.md`
- `"telegram:<chat_id>"` - Send to Telegram chat (requires gateway configured)

Multiple destinations can be combined:
```toml
output_destinations = ["file", "telegram:123456789"]
```

## Environment Variables

The daemon inherits the environment from the user session (when started manually) or from the service configuration. Key variables:

- `ANTHROPIC_API_KEY` - Required for claude CLI
- `CLAUDE_CONFIG_DIR` - Custom Claude config location
- `RUST_LOG` - Logging level for the daemon (e.g., `info`, `debug`)
