---
name: start
description: Start the CC-Demon daemon process (scheduler and optional Telegram gateway)
argument-hint: "[--with-gateway]"
allowed-tools:
  - Bash
  - Read
---

Start the CC-Demon daemon. Check if the `demon` binary is available in PATH or at `${CLAUDE_PLUGIN_ROOT}/bin/demon`.

Steps:
1. Check if the daemon is already running by executing `demon status`
2. If already running, inform the user with the PID and current status
3. Ask if the user wants to enable the Telegram gateway with `--with-gateway`
4. Start the daemon with `demon start` (add `--with-gateway` if requested)
5. Verify it started successfully by running `demon status`
6. Show the user the status output

If the `demon` binary is not found, inform the user they need to install it:
- Build from source: `cd ${CLAUDE_PLUGIN_ROOT} && cargo build --release`
- Or download a pre-built binary from the releases page

If starting with gateway, verify that the Telegram bot token is configured first by checking `demon gateway status`.
