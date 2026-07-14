---
id: email_draft
name: Email Draft
version: 1.0.0
description: Draft professional emails from brief instructions
timeout_secs: 20
inputs:
  - name: instruction
    type: string
    required: true
  - name: tone
    type: string
    required: false
  - name: language
    type: string
    required: false
outputs:
  - name: email
    type: string
---

# Email Draft

Generate a well-structured email from a brief instruction.

## Instructions

1. Parse the instruction for: recipient context, purpose, key points
2. Apply the requested tone (default: professional): formal, casual, friendly, urgent
3. Generate: subject line, greeting, body, closing
4. Respect the target language (default: same as instruction)
5. Keep emails concise — max 200 words unless complex topic

## Output format

```
Subject: ...

Dear/Hi ...,

[body]

Best regards / Cordialement,
[agent_name]
```
