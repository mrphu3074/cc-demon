---
name: status
description: Show CC-Demon daemon status, scheduled jobs, and gateway info
allowed-tools:
  - Bash
  - Read
---

Display the current status of the CC-Demon daemon.

Steps:
1. Run `demon status` and display the output
2. Run `demon job list` to show all scheduled jobs with their details
3. Run `demon gateway status` to show Telegram gateway configuration
4. If the daemon is not running, suggest starting it with `/demon:start`
5. If no jobs are configured, suggest creating one with `/demon:schedule`

Format the output clearly with sections for daemon status, jobs, and gateway.
