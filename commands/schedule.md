---
name: schedule
description: Create a new scheduled job interactively using natural language
argument-hint: "[job description in natural language]"
allowed-tools:
  - Bash
  - Read
  - Write
  - AskUserQuestion
---

Help the user create a new scheduled job for the CC-Demon daemon. The user describes what they want in natural language, and this command converts it into a proper job configuration.

## Workflow

1. **Gather requirements** - Ask the user what they want to schedule. If they provided an argument, use that as the starting point. Key information needed:
   - What should the job do? (the prompt for Claude)
   - When should it run? (natural language like "every weekday at 9am", "every Monday", "tomorrow at 3pm", "every hour")
   - Any specific project/directory context?

2. **Convert schedule to cron** - Convert natural language schedule to cron expression:
   - "every day at 9am" → `0 9 * * *`
   - "every weekday at 9am" → `0 9 * * 1-5`
   - "every Monday at 10am" → `0 10 * * 1`
   - "every hour" → `0 * * * *`
   - "every 30 minutes" → `*/30 * * * *`
   - For one-shot schedules ("tomorrow at 3pm", "in 2 hours"), set `schedule_type = "once"` and compute the ISO 8601 datetime for `once_at`

3. **Design the prompt** - Help the user craft an effective prompt. Consider:
   - What context sources does Claude need? (Jira, Linear, files, git history)
   - What tools should be allowed? (Read, Grep, Glob for read-only; Bash for commands)
   - What output format is best?
   - Should dangerous tools be blocked?

4. **Configure advanced options** - Ask if the user wants to customize:
   - Model (default: sonnet)
   - Max turns (default: 10)
   - Max budget (default: $5.00)
   - Output destinations (file, telegram:<chat_id>)
   - Working directory
   - System prompt additions

5. **Generate job TOML** - Create the job definition as TOML and show it to the user for confirmation

6. **Add the job** - Pipe the TOML to `demon job add` via stdin:
   ```bash
   echo '<TOML content>' | demon job add
   ```

7. **Verify** - Run `demon job list` to confirm the job was added

## Job ID Generation

Generate a short, descriptive kebab-case ID from the job name. Example: "Daily Plan Generator" → "daily-plan-generator"

## Example Interaction

User: "Schedule a daily standup summary every weekday at 8:45am"

Result job:
```toml
id = "daily-standup-summary"
name = "Daily Standup Summary"
schedule_type = "recurring"
schedule = "45 8 * * 1-5"
prompt = "Review my Jira tickets, recent git commits, and calendar to generate a concise standup summary with: what I did yesterday, what I'm doing today, and any blockers."
working_dir = ""
model = "sonnet"
max_turns = 10
max_budget_usd = 5.0
output_format = "json"
output_destinations = ["file"]
enabled = true
```
