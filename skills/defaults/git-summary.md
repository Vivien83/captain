---
id: git_summary
name: Git Summary
version: 1.0.0
description: Summarize recent git activity — commits, branches, changes
timeout_secs: 15
inputs:
  - name: repo_path
    type: string
    required: false
  - name: days
    type: string
    required: false
outputs:
  - name: summary
    type: string
---

# Git Summary

Generate a summary of recent git repository activity.

## Instructions

1. Show recent commits (last N days, default 7)
2. List active branches
3. Show files changed statistics
4. Identify contributors

```bash
cd "${repo_path:-.}" && echo "=== Last ${days:-7} days ===" && git log --oneline --since="${days:-7} days ago" --pretty=format:"%h %s (%an, %ar)" && echo "" && echo "=== Branches ===" && git branch -a --sort=-committerdate | head -10 && echo "=== Stats ===" && git diff --stat HEAD~10..HEAD 2>/dev/null
```
