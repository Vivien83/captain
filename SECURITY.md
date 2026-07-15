# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.0-alpha.3 | :white_check_mark: |
| 0.1.0-alpha.2 | :x: |
| 0.1.0-alpha.1 | :x: |
| Development snapshots | :x: |

## Reporting a Vulnerability

If you discover a security vulnerability in Captain, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

### How to Report

1. Open a [private GitHub security advisory](https://github.com/Vivien83/captain/security/advisories/new).
2. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Affected versions
   - Potential impact assessment
   - Suggested fix (if any)

If GitHub private vulnerability reporting is unavailable, do not disclose the
issue publicly. Open a non-sensitive issue asking a maintainer to establish a
private contact channel.

### What to Expect

- **Acknowledgment target** within 48 hours
- **Initial assessment target** within 7 days
- **Fix timeline** communicated after triage
- **Credit** given in the advisory (unless you prefer anonymity)

### Scope

The following are in scope for security reports:

- Authentication/authorization bypass
- Remote code execution
- Path traversal / directory traversal
- Server-Side Request Forgery (SSRF)
- Privilege escalation between agents or users
- Information disclosure (API keys, secrets, internal state)
- Denial of service via resource exhaustion
- Supply chain attacks via skill ecosystem
- WASM sandbox escapes

## Early-Access Deployment Boundary

Captain `0.1.0-alpha.3` is an early-access release. Keep the API bound to
loopback unless authentication, TLS, and a trusted reverse proxy are configured.
Agents can execute tools with the permissions granted to them, so review
capabilities and destructive-action confirmations before connecting untrusted
users or content. Prefer Ed25519 SSH keys for remote access.

## Security Architecture

Captain implements defense-in-depth with the following security controls:

### Access Control
- **Capability-based permissions**: Agents only access resources explicitly granted
- **RBAC multi-user**: Owner/Admin/User/Viewer role hierarchy
- **Privilege escalation prevention**: Child agents cannot exceed parent capabilities
- **API authentication**: Bearer token with loopback bypass for local CLI

### Input Validation
- **Path traversal protection**: `safe_resolve_path()` / `safe_resolve_parent()` on all file operations
- **SSRF protection**: Private IP blocking, DNS resolution checks, cloud metadata endpoint filtering
- **Image validation**: Media type whitelist (png/jpeg/gif/webp), 5MB size limit
- **Prompt injection scanning**: Skill content scanned for override attempts and data exfiltration

### Cryptographic Security
- **Ed25519 signed manifests**: Agent identity verification
- **HMAC-SHA256 wire protocol**: Mutual authentication with nonce-based replay protection
- **Secret zeroization**: `Zeroizing<String>` on all API key fields, wiped on drop

### Runtime Isolation
- **WASM dual metering**: Fuel limits + epoch interruption with watchdog thread
- **Subprocess sandbox**: Environment isolation (`env_clear()`), restricted PATH
- **Taint tracking**: Information flow labels prevent untrusted data in privileged operations

### Network Security
- **GCRA rate limiter**: Cost-aware token buckets per IP
- **Security headers**: CSP, X-Frame-Options, X-Content-Type-Options, HSTS
- **Health redaction**: Public endpoint returns minimal info; full diagnostics require auth
- **CORS policy**: Restricted to localhost when no API key configured

### Audit
- **Merkle hash chain**: Tamper-evident audit trail for all agent actions
- **Tamper detection**: Chain integrity verification via `/api/audit/verify`

## Dependencies

Security-critical dependencies are pinned and audited:

| Dependency | Purpose |
|------------|---------|
| `ed25519-dalek` | Manifest signing |
| `sha2` | Hash chain, checksums |
| `hmac` | Wire protocol authentication |
| `subtle` | Constant-time comparison |
| `zeroize` | Secret memory wiping |
| `rand` | Cryptographic randomness |
| `governor` | Rate limiting |
