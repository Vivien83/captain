---
id: reminder
name: Reminder
version: 1.0.0
description: Create scheduled reminders that trigger at specified times
timeout_secs: 10
inputs:
  - name: message
    type: string
    required: true
  - name: when
    type: string
    required: true
outputs:
  - name: status
    type: string
---

# Reminder

Create a reminder that will be delivered at the specified time via the configured messaging channel (Telegram, Discord, or WebSocket).

## Instructions

1. Parse natural language time expressions: "in 30 minutes", "tomorrow at 9am", "every Monday at 10:00"
2. Store the reminder in the agent's memory graph with a scheduled_at timestamp
3. The sentinel agent monitors for due reminders and delivers them
4. Supports one-time and recurring reminders

## Storage format

Reminders are stored as entities in the knowledge graph:
- type: "reminder"
- properties: message, scheduled_at, recurring, channel
