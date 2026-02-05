# CC-Demon

<p align="center">
  <img src="images/logo.png" alt="CC-Demon Logo" width="640">
</p>

**Daemon scheduler and Telegram gateway for Claude Code.**

CC-Demon fills two gaps in Claude Code:

1. **Scheduled Jobs** - Run Claude Code sessions on a cron schedule (e.g., "every weekday at 9am, generate my daily plan")
2. **Telegram Gateway** - Chat with Claude Code through Telegram, with whitelisted access control

The name "Demon" is a play on "Daemon" - the background process that powers it all.

## Features

- **Natural language scheduling** - Say "every weekday at 9am" instead of writing cron expressions
- **Full Claude CLI power** - Each job can configure model, tools, budget, MCP servers, and more
- **Telegram integration** - Receive messages, run Claude Code, send back results
- **Flexible output routing** - Save to files, send to Telegram, or both
- **Service installation** - Runs as systemd (Linux), launchd (macOS), or Task Scheduler (Windows)
- **One-shot & recurring** - Schedule both recurring cron jobs and one-time delayed tasks

## Installation

### Pre-built Binary

```bash
curl -fsSL https://raw.githubusercontent.com/phunguyen/cc-demon/main/scripts/install.sh | bash
```

### Build from Source

Requires Rust toolchain:

```bash
git clone https://github.com/phunguyen/cc-demon
cd cc-demon
cargo build --release
cp target/release/demon ~/.local/bin/
```

### Claude Code Plugin

Add to your Claude Code plugins:

```bash
claude plugin add /path/to/cc-demon
```

## Quick Start

### 1. Configure

```
/demon:config
```

Set up your Telegram bot token and defaults.

### 2. Schedule a Job

```
/demon:schedule every weekday at 9am generate my daily action plan
```

Claude will help you craft the perfect prompt and configure the job.

### 3. Start the Daemon

```
/demon:start
```

### 4. Check Status

```
/demon:status
```

## Plugin Commands

| Command | Description |
|---------|-------------|
| `/demon:start` | Start the daemon (with optional Telegram gateway) |
| `/demon:stop` | Stop the daemon |
| `/demon:status` | Show daemon status, jobs, and gateway info |
| `/demon:schedule` | Create a scheduled job using natural language |
| `/demon:config` | Configure Telegram token, defaults, and paths |

## CLI Usage

The `demon` binary can also be used directly:

```bash
demon start [--with-gateway] [--foreground]
demon stop
demon status
demon job add          # Reads TOML from stdin
demon job list
demon job remove <id>
demon job run <id>     # Run a job immediately
demon job enable <id>
demon job disable <id>
demon gateway start
demon gateway status
demon install [--with-gateway]
demon uninstall
```

## Configuration

Config file: `~/.demon/config.toml`

```toml
[gateway]
enabled = true
bot_token = "YOUR_BOT_TOKEN"
allowed_chat_ids = [123456789]
default_model = "sonnet"
max_turns = 10
max_budget_usd = 5.0

[defaults]
model = "sonnet"
max_turns = 10
max_budget_usd = 5.0
output_format = "json"
```

See `skills/daemon-ops/references/config-reference.md` for the full configuration reference.

## Architecture

```
demon daemon (Rust binary)
├── Scheduler Engine
│   ├── Cron parser (recurring jobs)
│   ├── One-shot scheduler (delayed tasks)
│   └── Job executor (spawns claude -p)
├── Telegram Gateway
│   ├── Bot listener (whitelisted chats)
│   └── Response handler (spawns claude -p)
├── Config Manager (TOML)
└── Output Router
    ├── File writer (~/.demon/output/)
    └── Telegram sender
```

## Requirements

- Claude Code CLI (`claude`) in PATH
- Valid `ANTHROPIC_API_KEY` environment variable
- Rust toolchain (for building from source)
- Telegram Bot token (for gateway feature)

## License

Apache-2.0
