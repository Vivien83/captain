---
id: service_health
name: Service Health
version: 1.0.0
description: Check HTTP endpoint availability and response times
timeout_secs: 15
inputs:
  - name: url
    type: string
    required: true
  - name: expected_status
    type: string
    required: false
outputs:
  - name: result
    type: json
---

# Service Health

Ping an HTTP endpoint and report its health status.

## Instructions

1. Send a GET request to the URL
2. Measure response time
3. Check status code against expected (default: 200)
4. Report: status, response_time_ms, headers, body preview

Can be combined with the reminder skill for periodic health checks.

```bash
curl -s -o /dev/null -w '{"status": %{http_code}, "time_ms": %{time_total}000, "dns_ms": %{time_namelookup}000, "connect_ms": %{time_connect}000, "url": "%{url_effective}"}' "${url}"
```
