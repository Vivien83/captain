---
id: ssl_checker
name: SSL Checker
version: 1.0.0
description: Check SSL/TLS certificate expiration and validity
timeout_secs: 15
inputs:
  - name: domain
    type: string
    required: true
outputs:
  - name: result
    type: json
---

# SSL Checker

Verify SSL/TLS certificates for a domain — expiration date, issuer, validity.

## Instructions

1. Connect to the domain on port 443
2. Extract certificate details: subject, issuer, dates, SANs
3. Calculate days until expiration
4. Flag if expiring within 30 days

```bash
echo | openssl s_client -servername "${domain}" -connect "${domain}:443" 2>/dev/null | openssl x509 -noout -subject -issuer -dates -checkend 2592000 2>/dev/null
```
