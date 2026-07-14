---
id: port_scanner
name: Port Scanner
version: 1.0.0
description: Scan open ports on a host for security auditing
timeout_secs: 30
inputs:
  - name: host
    type: string
    required: true
  - name: ports
    type: string
    required: false
outputs:
  - name: result
    type: json
---

# Port Scanner

Scan common ports on a target host to identify open services.

## Instructions

1. Default scan: common ports (22, 80, 443, 3000, 3100, 5432, 6379, 8080, 8443)
2. Custom range: "ports=1-1024" or "ports=80,443,8080"
3. Report: port, status (open/closed), service name
4. Only scan hosts you own or have permission to scan

```bash
for port in ${ports:-22 80 443 3000 3100 5432 6379 8080 8443}; do
  (echo >/dev/tcp/${host}/${port}) 2>/dev/null && echo "${port} open" || echo "${port} closed"
done
```
