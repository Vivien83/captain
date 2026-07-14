---
id: translate
name: Translate
version: 1.0.0
description: Translate text between languages with context-aware quality
timeout_secs: 20
inputs:
  - name: text
    type: string
    required: true
  - name: target_language
    type: string
    required: true
  - name: source_language
    type: string
    required: false
outputs:
  - name: translation
    type: string
---

# Translate

Translate text between languages using the agent's LLM capabilities.

## Instructions

1. Auto-detect source language if not specified
2. Preserve formatting, tone, and intent
3. Handle idioms and cultural context appropriately
4. For technical text, maintain terminology consistency
5. Provide alternatives for ambiguous terms when relevant

## Supported patterns

- Simple: "translate 'bonjour' to English"
- Document: "translate this paragraph to Japanese"
- Context-aware: "translate this email to formal German"
