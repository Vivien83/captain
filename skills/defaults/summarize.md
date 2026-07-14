---
id: summarize
name: Summarize
version: 1.0.0
description: Summarize text, articles, or URLs into concise bullet points
timeout_secs: 30
inputs:
  - name: text
    type: string
    required: true
  - name: style
    type: string
    required: false
outputs:
  - name: summary
    type: string
---

# Summarize

Produce a concise summary of the provided text or URL content.

## Instructions

1. If input is a URL, fetch its content first using curl
2. Extract key points, facts, and conclusions
3. Return a structured summary with bullet points
4. Keep the summary under 300 words unless specified otherwise
5. Style options: "brief" (3-5 bullets), "detailed" (paragraph), "executive" (1 sentence)

The agent uses its LLM capabilities directly — no external command needed.
