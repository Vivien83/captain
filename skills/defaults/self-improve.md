---
id: self_improve
name: Self Improve
version: 1.0.0
description: Create new skills by writing SKILL.md files to the skills directory
timeout_secs: 30
inputs:
  - name: skill_idea
    type: string
    required: true
outputs:
  - name: skill_path
    type: string
---

# Self Improve

The agent can create new skills for itself by generating SKILL.md files.

## Instructions

1. Analyze the skill idea — what capability is needed?
2. Design the skill with proper inputs/outputs
3. Write a SKILL.md file following this format:

```yaml
---
id: unique_snake_case
name: Human Readable Name
version: 1.0.0
description: What it does in one line
timeout_secs: 15
inputs:
  - name: param_name
    type: string
    required: true
outputs:
  - name: result_name
    type: string
---
```

4. Include clear instructions and a bash command if applicable
5. Save to the skills directory
6. The skill becomes available immediately (no restart needed)

## Guidelines

- Keep skills focused — one skill = one capability
- Include proper error handling in bash commands
- Document inputs clearly so other agents can use the skill
- Test the bash command before saving
