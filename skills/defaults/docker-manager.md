---
id: docker_manager
name: Docker Manager
version: 1.0.0
description: List, inspect, restart, and view logs of Docker containers
timeout_secs: 30
inputs:
  - name: action
    type: string
    required: true
  - name: container
    type: string
    required: false
outputs:
  - name: result
    type: string
---

# Docker Manager

Manage Docker containers on the host system.

## Actions

- `list` — list all containers with status
- `logs <container> [lines]` — tail container logs (default 50 lines)
- `restart <container>` — restart a container
- `stop <container>` — stop a container
- `inspect <container>` — detailed container info
- `stats` — resource usage of running containers

```bash
case "${action}" in
  list)    docker ps -a --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" ;;
  logs)    docker logs --tail ${lines:-50} "${container}" 2>&1 ;;
  restart) docker restart "${container}" ;;
  stop)    docker stop "${container}" ;;
  inspect) docker inspect "${container}" | jq '.[0] | {Name, State, Config: {Image, Env}}' ;;
  stats)   docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}" ;;
esac
```
