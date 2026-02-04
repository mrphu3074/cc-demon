---
name: config
description: Configure CC-Demon settings (Telegram bot token, defaults, output directory)
argument-hint: "[setting to configure]"
allowed-tools:
  - Bash
  - Read
  - Write
  - AskUserQuestion
---

Help the user configure CC-Demon settings. The configuration file is stored at `~/.demon/config.toml`.

## Configuration Sections

### Telegram Gateway
- `gateway.bot_token` - Telegram Bot API token (from @BotFather)
- `gateway.allowed_chat_ids` - Whitelisted Telegram chat IDs (DMs and groups)
- `gateway.default_model` - Model for gateway responses (default: sonnet)
- `gateway.max_turns` - Max turns per gateway response (default: 10)
- `gateway.max_budget_usd` - Max budget per gateway message (default: $5.00)
- `gateway.allowed_tools` - Tools the gateway can use
- `gateway.disallowed_tools` - Tools to block in gateway
- `gateway.append_system_prompt` - Additional system prompt for gateway

### Job Defaults
- `defaults.model` - Default model for new jobs
- `defaults.max_turns` - Default max turns
- `defaults.max_budget_usd` - Default max budget
- `defaults.output_format` - Default output format (json or text)

### Paths
- `paths.base_dir` - Base directory for demon data (default: ~/.demon)

## Workflow

1. Read the current config from `~/.demon/config.toml` (create if missing)
2. If the user specified what to configure, go directly to that section
3. Otherwise, show current configuration and ask what they want to change
4. For Telegram setup, guide through:
   a. Getting a bot token from @BotFather
   b. Finding chat IDs (suggest sending a message to the bot and checking `https://api.telegram.org/bot<TOKEN>/getUpdates`)
   c. Setting allowed chat IDs
5. Write the updated config back to `~/.demon/config.toml`
6. If the daemon is running, notify user to restart it for changes to take effect

## Example Config

```toml
[gateway]
enabled = true
bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
allowed_chat_ids = [123456789, -987654321]
default_model = "sonnet"
max_turns = 10
max_budget_usd = 5.0
allowed_tools = ["Read", "Grep", "Glob"]
disallowed_tools = ["Bash(rm *)", "Bash(sudo *)"]
append_system_prompt = ""

[defaults]
model = "sonnet"
max_turns = 10
max_budget_usd = 5.0
output_format = "json"
```
