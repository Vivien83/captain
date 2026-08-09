# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.0-alpha.12 | :white_check_mark: |
| 0.1.0-alpha.11 | :x: |
| 0.1.0-alpha.10 | :x: |
| 0.1.0-alpha.9 | :x: |
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

Captain `0.1.0-alpha.12` is an early-access release. Keep the API bound to
loopback unless authentication, TLS, and a trusted reverse proxy are configured.
Agents can execute tools with the permissions granted to them, so review
capabilities and destructive-action confirmations before connecting untrusted
users or content. Captain accepts Ed25519 and ECDSA P-256 SSH private keys.
RSA SSH is disabled while the upstream Rust implementation remains affected by
the unresolved timing-side-channel advisory RUSTSEC-2023-0071.

## Security Architecture

Captain implements defense-in-depth with the following security controls:

### Access Control
- **Capability-based permissions**: Agents only access resources explicitly granted
- **RBAC multi-user**: Owner/Admin/User/Viewer role hierarchy
- **Privilege escalation prevention**: Child agents cannot exceed parent capabilities
- **API authentication**: deny-by-default Bearer/session middleware with one
  reviewed public allowlist; unauthenticated mode is limited to a loopback bind

### Input Validation
- **Path traversal protection**: `safe_resolve_path()` / `safe_resolve_parent()` on all file operations
- **SSRF protection**: Private IP blocking, DNS resolution checks, cloud metadata endpoint filtering
- **Image validation**: Media type whitelist (png/jpeg/gif/webp), 5MB size limit
- **Advisory skill phrase review**: Skill prompts are checked for a bounded set
  of risky phrases and high-risk matches are conservatively refused. The result
  is an `advisory_heuristic`, not proof that content is safe or malicious.
- **Remote skill marketplace freeze**: remote search and installation are absent
  from API, CLI, and TUI; retained compatibility clients fail before network or
  filesystem access

### Cryptographic Security
- **Ed25519 signed manifests**: Agent identity verification
- **HMAC-SHA256 wire protocol**: Mutual authentication with nonce-based replay protection
- **Secret zeroization**: `Zeroizing<String>` on all API key fields, wiped on drop

### Runtime Isolation
- **WASM dual metering**: Fuel limits + epoch interruption with watchdog thread
- **Guarded host execution boundary**: Agent-controlled subprocesses share
  policy review, normalized heuristic critical-command handling, `env_clear()`
  plus explicit injection, workspace/timeout/output bounds, and command-free
  audit events
- **Exact host posture**: Native host subprocesses report
  `environment_scrub` and `os_isolation: false`; use the explicit Docker or
  WASM backend when an operating-system isolation boundary is required
- **Honest content classification**: Pattern guards are heuristic and Captain
  does not claim end-to-end information-flow provenance tracking

### Network Security
- **GCRA rate limiter**: Cost-aware token buckets per IP
- **Security headers**: CSP, X-Frame-Options, X-Content-Type-Options, HSTS
- **Health redaction**: Public endpoint returns minimal info; full diagnostics require auth
- **Browser request perimeter**: CORS allows only exact loopback or explicitly
  configured HTTP(S) origins with reviewed methods/headers, independently of
  API-key presence; an exact `Host` allowlist rejects DNS rebinding before
  routing

### Audit
- **Versioned SHA-256 hash chain**: Length-prefixed hashes for new entries,
  with read-only compatibility for legacy hashes
- **Immutable recovery epochs**: A corrupt epoch is sealed as invalid; Captain
  opens a `ChainRecovery` epoch without rewriting historical rows
- **Fail-loud persistence**: A failed append does not advance the in-memory tip
  and degrades authenticated health, CLI, TUI, and metrics surfaces
- **Tamper detection**: Chain integrity verification via `/api/audit/verify`;
  no HTTP repair endpoint exists

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

`scripts/dependency-audit.sh` is the release dependency gate. It runs the
configured RustSec audit, then a second audit without exceptions and compares
every finding with a reviewed package/version/advisory baseline. New
vulnerabilities, newly ignored findings, changes to critical parent chains,
reintroduced RSA features, or drift in the two accepted informational warnings
fail the gate.

The only reviewed vulnerability exceptions are RUSTSEC-2026-0194 and
RUSTSEC-2026-0195 on `quick-xml 0.37.5`, reachable solely through the
Windows-only notification backend. That backend uses only XML escaping and
does not call the affected XML parser APIs. The main Tauri/plist path uses
`quick-xml 0.41.0`. This exception is removed as soon as the upstream
notification chain accepts `tauri-winrt-notification >=0.8`.

Host release assets include a deterministic in-toto/SLSA v1 provenance
statement that binds all 20 pre-existing assets to the public Git commit,
tree, and exact `Cargo.lock`. The local publisher verifies the statement before
upload and builds Docker architectures sequentially with BuildKit provenance
before assembling the multi-architecture index. The alpha statement has a
SHA-256 sidecar but no independent signing identity; it is not a claim of SLSA
build-level certification. See [Release Provenance](docs/release-provenance.md).
