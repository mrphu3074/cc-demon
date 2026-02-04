---
name: daemon-ops
description: Use this skill when troubleshooting CC-Demon daemon issues, understanding cron expressions, managing scheduled jobs, configuring the Telegram gateway, or debugging job execution failures. Triggers on questions about "demon daemon", "demon not running", "cron schedule", "telegram bot setup", "job failed", "demon logs", "scheduled task", or "demon config".
version: 0.1.0
---

Provide guidance on CC-Demon daemon operations, troubleshooting, and configuration.

## Daemon Management

The `demon` binary manages the daemon lifecycle:

- `demon start` - Start daemon (daemonizes by default)
- `demon start --foreground` - Run in foreground for debugging
- `demon start --with-gateway` - Start with Telegram gateway
- `demon stop` - Stop daemon gracefully (SIGTERM)
- `demon status` - Show running state, PID, jobs, gateway

**Data locations:**
- Config: `~/.demon/config.toml`
- Jobs: `~/.demon/jobs.toml`
- Output: `~/.demon/output/<job-id>/`
- Logs: `~/.demon/logs/`
- PID: `~/.demon/demon.pid`

## Troubleshooting

### Daemon won't start
1. Check if already running: `demon status`
2. Check for stale PID file: `cat ~/.demon/demon.pid` and verify process exists
3. Remove stale PID: `rm ~/.demon/demon.pid`
4. Check logs: `cat ~/.demon/logs/demon.err`
5. Try foreground mode: `demon start --foreground`

### Jobs not executing
1. Verify daemon is running: `demon status`
2. Check job is enabled: `demon job list`
3. Verify cron expression is correct
4. Check logs for errors: `tail -f ~/.demon/logs/demon.log`
5. Test job manually: `demon job run <job-id>`
6. Verify `claude` is in PATH for the daemon process

### Telegram gateway issues
1. Verify bot token: `demon gateway status`
2. Check chat ID whitelist includes your chat
3. Test bot token: `curl https://api.telegram.org/bot<TOKEN>/getMe`
4. Check logs for connection errors
5. Ensure network access from daemon process

### Job output issues
1. Check output directory: `ls ~/.demon/output/<job-id>/`
2. For Telegram output, verify bot token and chat ID
3. Check job's `output_destinations` configuration
4. Review logs for routing errors

## Cron Expression Reference

Format: `second minute hour day-of-month month day-of-week`

| Expression | Meaning |
|------------|---------|
| `0 9 * * 1-5` | Weekdays at 9:00 AM |
| `0 */2 * * *` | Every 2 hours |
| `30 8 * * 1` | Monday at 8:30 AM |
| `0 0 1 * *` | First of every month at midnight |
| `*/30 * * * *` | Every 30 minutes |

## Service Installation

**Linux (systemd):**
```bash
demon install [--with-gateway]
systemctl --user enable cc-demon
systemctl --user start cc-demon
```

**macOS (launchd):**
```bash
demon install [--with-gateway]
launchctl load ~/Library/LaunchAgents/com.cc-demon.daemon.plist
```

For detailed reference, see files in `${CLAUDE_PLUGIN_ROOT}/skills/daemon-ops/references/`.
