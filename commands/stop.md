---
name: stop
description: Stop the CC-Demon daemon process
allowed-tools:
  - Bash
---

Stop the running CC-Demon daemon.

Steps:
1. Check if the daemon is running with `demon status`
2. If not running, inform the user
3. Stop the daemon with `demon stop`
4. Verify it stopped by running `demon status` again
5. Show confirmation to the user
