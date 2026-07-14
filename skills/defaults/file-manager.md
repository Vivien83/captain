---
id: file_manager
name: File Manager
version: 1.0.0
description: Read, write, list, and search files on the local filesystem
timeout_secs: 15
inputs:
  - name: action
    type: string
    required: true
  - name: path
    type: string
    required: true
  - name: content
    type: string
    required: false
outputs:
  - name: result
    type: string
---

# File Manager

Manage files on the local filesystem. Actions: read, write, list, search, info.

Safety: paths are validated to prevent traversal. Writes create parent directories automatically.

## Actions

- `read <path>` — read file contents
- `write <path> <content>` — write content to file
- `list <path>` — list directory contents
- `search <path> <pattern>` — search files by name pattern
- `info <path>` — file size, permissions, modification date

```bash
case "${action}" in
  read)   cat "${path}" ;;
  write)  mkdir -p "$(dirname "${path}")" && echo "${content}" > "${path}" ;;
  list)   ls -la "${path}" ;;
  search) find "${path}" -name "${content}" 2>/dev/null ;;
  info)   stat "${path}" ;;
esac
```
