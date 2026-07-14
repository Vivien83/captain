---
id: daily_digest
name: Daily Digest
version: 1.0.0
description: Generate a morning briefing with services health, reminders, and metrics
timeout_secs: 30
inputs:
  - name: sections
    type: string
    required: false
outputs:
  - name: digest
    type: string
---

# Daily Digest

Compile a daily briefing for the user. Delivered via configured messaging channel.

## Sections (all enabled by default)

- **services** — health check on monitored URLs
- **reminders** — upcoming reminders for today
- **metrics** — agent metrics (requests, tokens, uptime)
- **memory** — new entities/facts added since yesterday
- **weather** — local weather (if location configured)

## Instructions

1. Query the agent's monitoring endpoint for metrics
2. Check knowledge graph for today's reminders
3. Run service health checks on configured URLs
4. Compile into a formatted digest
5. Send via Telegram/Discord/WebSocket based on configuration

## Schedule

Designed to be triggered by sentinel agent at a configured time (default: 08:00 local).
The sentinel checks for a "daily_digest" scheduled entity in the graph.
