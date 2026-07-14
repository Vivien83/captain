---
id: system_info
name: System Info
version: 1.0.0
description: Retrieve system information — CPU, memory, disk, uptime, processes
timeout_secs: 10
inputs: []
outputs:
  - name: info
    type: json
---

# System Info

Gather system metrics for monitoring and diagnostics.

```bash
echo "{\"hostname\": \"$(hostname)\", \"uptime\": \"$(uptime)\", \"disk\": \"$(df -h / | tail -1)\", \"memory\": \"$(free -h 2>/dev/null || vm_stat | head -5)\", \"cpu_count\": $(nproc 2>/dev/null || sysctl -n hw.ncpu), \"load\": \"$(cat /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg)\"}"
```
