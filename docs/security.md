# Captain Security Architecture

> Report vulnerabilities privately through the
> [GitHub security advisory form](https://github.com/Vivien83/captain/security/advisories/new).
> Do not disclose security issues in public issues. See
> [`SECURITY.md`](../SECURITY.md) for the supported-version and response policy.

This document provides a comprehensive technical reference for every security
system in the Captain Agent Operating System.  All struct names, function
signatures, constant values, and algorithm descriptions are drawn directly from
the source code.

---

## Table of Contents

1.  [Security Overview](#1-security-overview)
2.  [Capability-Based Security](#2-capability-based-security)
3.  [WASM Dual Metering](#3-wasm-dual-metering)
4.  [Versioned Hash-Chain Audit Trail](#4-versioned-hash-chain-audit-trail)
5.  [Heuristic Content Guards](#5-heuristic-content-guards)
6.  [Ed25519 Manifest Signing](#6-ed25519-manifest-signing)
7.  [SSRF Protection](#7-ssrf-protection)
8.  [Secret Zeroization](#8-secret-zeroization)
9.  [OFP Mutual Authentication](#9-ofp-mutual-authentication)
10. [Security Headers](#10-security-headers)
11. [GCRA Rate Limiter](#11-gcra-rate-limiter)
12. [Path Traversal Prevention](#12-path-traversal-prevention)
13. [Subprocess Sandbox](#13-subprocess-sandbox)
14. [Advisory Skill Phrase Review](#14-advisory-skill-phrase-review)
15. [Loop Guard](#15-loop-guard)
16. [Session Repair](#16-session-repair)
17. [Health Endpoint Redaction](#17-health-endpoint-redaction)
18. [Security Configuration](#18-security-configuration)
19. [Security Dependencies](#19-security-dependencies)

---

## 1. Security Overview

Captain implements **defense-in-depth** security. No single mechanism or review
signal is treated as proof of safety; overlapping controls reduce risk and
surface decisions that still require operator review.

| # | System | Crate | Protects Against |
|---|--------|-------|------------------|
| 1 | Capability-Based Security | `captain-types` | Unauthorized actions by agents |
| 2 | WASM Dual Metering | `captain-runtime` | Infinite loops, CPU DoS |
| 3 | Versioned Audit Hash Chain | `captain-runtime` | Tampered audit logs |
| 4 | Heuristic Content Guards | `captain-runtime` | Obvious dangerous shell and secret-bearing URL patterns |
| 5 | Ed25519 Manifest Signing | `captain-types` | Supply chain attacks |
| 6 | SSRF Protection | `captain-runtime` | Server-Side Request Forgery |
| 7 | Secret Zeroization | `captain-runtime`, `captain-channels` | Memory forensics, key leakage |
| 8 | OFP Mutual Auth | `captain-wire` | Unauthorized peer connections |
| 9 | Security Headers | `captain-api` | XSS, clickjacking, MIME sniffing |
| 10 | GCRA Rate Limiter | `captain-api` | API abuse, denial of service |
| 11 | Path Traversal Prevention | `captain-runtime` | Directory traversal attacks |
| 12 | Guarded Host Process Boundary | `captain-runtime` | Secret leakage and unbounded child processes; not OS isolation |
| 13 | Advisory Skill Phrase Review | `captain-skills` | Bounded review signals; not adversarial proof |
| 14 | Loop Guard | `captain-runtime` | Stuck agent tool loops |
| 15 | Session Repair | `captain-runtime` | Corrupted LLM conversation history |
| 16 | Health Endpoint Redaction | `captain-api` | Information leakage |
| 17 | Durable Delegation Lineage | `captain-memory`, `captain-kernel` | Detached recursive delegation escaping depth or token limits |

---

## 2. Capability-Based Security

**Source:** `captain-types/src/capability.rs`

Captain uses capability-based security.  An agent can only perform actions
it has been explicitly granted permission to do.  Capabilities are immutable
after agent creation and are enforced at the kernel level.

### 2.1 Capability Variants

The `Capability` enum defines every permission type:

```rust
pub enum Capability {
    // Filesystem
    FileRead(String),       // Glob pattern, e.g. "/data/*"
    FileWrite(String),

    // Network
    NetConnect(String),     // Host:port pattern, e.g. "*.openai.com:443"
    NetListen(u16),

    // Tools
    ToolInvoke(String),     // Specific tool ID
    ToolAll,                // All tools (dangerous)

    // LLM
    LlmQuery(String),
    LlmMaxTokens(u64),

    // Agent interaction
    AgentSpawn,
    AgentMessage(String),
    AgentKill(String),

    // Memory
    MemoryRead(String),
    MemoryWrite(String),

    // Shell
    ShellExec(String),
    EnvRead(String),

    // OFP Wire Protocol
    OfpDiscover,
    OfpConnect(String),
    OfpAdvertise,

    // Economic
    EconSpend(f64),
    EconEarn,
    EconTransfer(String),
}
```

### 2.2 Pattern Matching

The `capability_matches(granted, required)` function implements glob-style
matching:

- **Exact match:** `"api.openai.com:443"` matches `"api.openai.com:443"`
- **Full wildcard:** `"*"` matches anything
- **Prefix wildcard:** `"*.openai.com:443"` matches `"api.openai.com:443"`
- **Suffix wildcard:** `"api.*"` matches `"api.openai.com"`
- **Middle wildcard:** `"api.*.com"` matches `"api.openai.com"`
- **ToolAll special case:** `ToolAll` grants any `ToolInvoke(_)`
- **Numeric bounds:** `LlmMaxTokens(10000)` grants `LlmMaxTokens(5000)` (granted >= required)

### 2.3 Enforcement Point

In the WASM sandbox, every host call is checked **before** execution by
`check_capability()` in `host_functions.rs`:

```rust
fn check_capability(
    capabilities: &[Capability],
    required: &Capability,
) -> Result<(), serde_json::Value> {
    for granted in capabilities {
        if capability_matches(granted, required) {
            return Ok(());
        }
    }
    Err(json!({"error": format!("Capability denied: {required:?}")}))
}
```

If no granted capability matches the required one, the operation returns a
JSON error immediately -- the tool is never invoked.

### 2.4 Capability Inheritance

When an agent spawns a child agent, `validate_capability_inheritance()` ensures
the child's capabilities are a **subset** of the parent's.  This prevents
privilege escalation:

```rust
pub fn validate_capability_inheritance(
    parent_caps: &[Capability],
    child_caps: &[Capability],
) -> Result<(), String> {
    for child_cap in child_caps {
        let is_covered = parent_caps
            .iter()
            .any(|parent_cap| capability_matches(parent_cap, child_cap));
        if !is_covered {
            return Err(format!(
                "Privilege escalation denied: child requests {:?} \
                 but parent does not have a matching grant",
                child_cap
            ));
        }
    }
    Ok(())
}
```

The `host_agent_spawn()` function in `host_functions.rs` calls
`kernel.spawn_agent_checked(manifest_toml, Some(&state.agent_id), &state.capabilities)`
which invokes this validation before the child is created.

### 2.5 Native Tool and CapSpec Enforcement

Native tools also have a fail-closed dispatch boundary. An agent manifest may
constrain its surface with top-level `tool_allowlist`, `[capabilities].tools`,
and `tool_blocklist`. The kernel filters the catalog shown to the model, and the
central ToolRunner applies the effective allowlist and hard blocklist again
immediately before execution. A hidden or composed call therefore cannot use
catalog visibility as an authorization bypass.

Captain Forge CapSpecs use the reserved `cap_*` namespace. Their effective
authority is the intersection of the caller's native tool grants, the CapSpec's
declared tools and scopes, Captain policy, and exact-revision human approval.
Every primitive DAG node re-enters the central ToolRunner with the original
caller identity and workspace. The readable `.captain` file cannot grant a
primitive that the caller lacks or remove a caller blocklist entry. A durable
run encrypts its initial tool grants, environment boundary, execution policy,
and subagent lineage. Resume intersects that snapshot with current authority,
so a later policy may revoke access but cannot expand the run. Uncertain-node
decisions compare the full run/node/attempt/tool-use identity atomically; stale
Control, API, TUI, or Telegram decisions are rejected. Telegram callback
tokens are only bounded lookup keys, remain behind the channel allowlist, and
must uniquely resolve to the complete current identity before any state change.
Retry and confirmation persist a resume intent in the decision transaction.
Only that intent is recovered at boot; an unrelated interrupted run is never
promoted into authorized work. An abandoned `in_progress` claim returns to
`requested`, while the executor's run lease still prevents duplicate dispatch.
The callback bridge handles Telegram decisions before model/session dispatch. See
[Captain Forge / CapSpec](CAPTAIN_FORGE_CAPSPEC.md) for activation, recovery,
the authority boundary, and process-level certification evidence.

### 2.6 Exact Tool Approval Rules

Dangerous native tool calls use a separate operator boundary. A session or
durable decision is keyed by the agent ID, canonical tool name, and a
domain-separated BLAKE3 digest of the complete, untruncated tool input. The
human-facing preview is a separate bounded field and is never used as the rule
binding. Repeating another command with the same agent and tool therefore
prompts again, even when both previews share the same truncated prefix. The broad
`[approval].allow_always` list remains an explicit administrator compatibility
override; an interactive **always** choice never mutates it.

Durable decisions live in the human-readable
`~/.captain/approval-rules.json`. Captain writes the file through synchronized
atomic replacement, limits it to 256 rules and 1 MiB, validates schema,
identities, unique bindings, bounded reasons, and secret-like content at boot,
and fails boot closed if the file is malformed. Only the action digest is
stored, never the raw command. A durable denial requires an operator reason.
Control, API, TUI, and Telegram can revoke a rule by ID; decisions, automatic
rule applications, and revocations enter the audit hash chain with actor,
source, digest, and rule ID.

### 2.7 Durable Delegation Lineage

Detached delegation does not trust process-local depth alone. While a delegated
model turn executes, the Kernel carries its job ID, root ID, depth, and target
agent in Tokio task-local state. A nested `agent_delegate` request derives its
proposed parent from that state, but the SQLite store is authoritative.

Enqueue uses an immediate transaction and requires the persisted parent to be
`running` with `effect_state = started`, requires the parent target to equal the
new caller, and requires exact root and depth continuity. Root jobs start at
depth 1 and no job may exceed depth 10. These fields are persisted, included in
bounded status events, and survive daemon restarts.

Each root also owns a durable reservation row capped at 500000 tokens. Enqueue
atomically adds the child's requested `max_tokens`; completion, retry, uncertain
recovery, and partial history pruning never decrease it. Idempotent replay is
checked before parent liveness and budget reservation, so a retried tool result
returns the original job without charging twice. Visibility filtering removes
`agent_delegate` from leaf agents as an early guard, but the transactional store
check remains the security boundary.

---

## 3. WASM Dual Metering

**Source:** `captain-runtime/src/sandbox.rs`

Untrusted WASM modules run inside a Wasmtime sandbox with **two
independent** metering mechanisms running simultaneously.

### 3.1 Fuel Metering (Deterministic)

Fuel metering counts WASM instructions.  The engine deducts fuel for every
instruction executed.  When the budget is exhausted, execution traps with
`Trap::OutOfFuel`.

```rust
// SandboxConfig defaults
pub fuel_limit: u64,  // Default: 1_000_000

// Applied at execution time
if config.fuel_limit > 0 {
    store.set_fuel(config.fuel_limit)?;
}
```

After execution, fuel consumed is reported:

```rust
let fuel_remaining = store.get_fuel().unwrap_or(0);
let fuel_consumed = config.fuel_limit.saturating_sub(fuel_remaining);
```

### 3.2 Epoch Interruption (Wall-Clock)

A watchdog thread sleeps for the configured timeout, then increments the
engine epoch.  When the epoch advances past the store's deadline, execution
traps with `Trap::Interrupt`.

```rust
store.set_epoch_deadline(1);
let engine_clone = engine.clone();
let timeout = config.timeout_secs.unwrap_or(30);
let _watchdog = std::thread::spawn(move || {
    std::thread::sleep(std::time::Duration::from_secs(timeout));
    engine_clone.increment_epoch();
});
```

### 3.3 Why Both?

| Property | Fuel | Epoch |
|----------|------|-------|
| **Metric** | Instruction count | Wall-clock time |
| **Precision** | Deterministic, reproducible | Non-deterministic |
| **Catches** | CPU-intensive loops | Host call blocking, I/O waits |
| **Evasion** | Can waste time in host calls | Can busy-loop cheaply |

Together they form a complete defense: fuel catches compute-intensive loops,
while epochs catch host-call abuse or environmental slowdowns.

### 3.4 SandboxConfig

```rust
pub struct SandboxConfig {
    pub fuel_limit: u64,           // Default: 1_000_000
    pub max_memory_bytes: usize,   // Default: 16 MB
    pub capabilities: Vec<Capability>,
    pub timeout_secs: Option<u64>, // Default: 30 seconds
}
```

### 3.5 Error Types

```rust
pub enum SandboxError {
    Compilation(String),
    Instantiation(String),
    Execution(String),
    FuelExhausted,         // Trap::OutOfFuel
    AbiError(String),
}
```

---

## 4. Versioned Hash-Chain Audit Trail

**Sources:** `captain-runtime/src/audit.rs`, `audit_chain.rs`,
`audit_persistence.rs`

Every security-critical action is appended to a tamper-evident SHA-256 hash
chain. This is a linear chain, not a tree structure: it does not provide tree
roots or inclusion proofs. New entries use an injective encoding with a
big-endian `u64` length before every field. Legacy version-1 hashes remain
readable so upgrades do not rewrite historical rows.

### 4.1 Auditable Actions

`AuditAction` covers tool, capability, lifecycle, memory, file, network, shell,
authentication, wire, configuration, learning, and approval activity.
`ChainRecovery` opens a recovery epoch. `Unknown(String)` retains an action
name introduced by a future version instead of silently reclassifying it.

### 4.2 Entry and Hash Format

Each entry stores a global `seq`, its `epoch`, `hash_version`, timestamp,
agent, action, detail, outcome, predecessor digest, and digest. Version 2
hashes every field, including version, epoch, and sequence:

```rust
fn hash_length_prefixed(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field);
}
```

Length-prefixing prevents two different field boundaries from producing the
same encoded input.

### 4.3 Persistence and Failure Policy

For persistent logs, SQLite insertion completes before the in-memory vector or
tip advances. `record()` returns `Result<String, AuditError>`. A database,
schema, sequence, or lock failure discards the candidate entry, emits an error,
and marks audit health degraded. Operations that cannot be rolled back use the
explicit `record_or_alert()` policy; they continue only with a visible health
alert.

### 4.4 Immutable Recovery Epochs

At boot, Captain verifies the active epoch. If verification fails, one SQLite
transaction:

1. marks that epoch `invalid` with its stored terminal digest;
2. creates the next active epoch;
3. appends `ChainRecovery`, whose predecessor and structured detail reference
   the previous terminal digest.

The verifier also requires every sequence at or after the active epoch's
`start_seq` to belong to that epoch. A modified epoch field therefore cannot
make an active entry disappear from verification, and the recovery epoch ID is
chosen above every epoch value present in either metadata or stored entries.

No audit entry is updated or deleted. Historical invalid epochs remain visible,
so overall integrity stays `degraded` even though the new active epoch is
verified and writable. A later restart reuses that epoch rather than creating
another recovery entry.

### 4.5 Verification and Surfaces

`AuditLog::verify_integrity()` verifies the active epoch and fails if any
historical epoch is sealed invalid or a runtime append failed. The authenticated
`/api/health/detail`, `/api/audit/recent`, `/api/audit/verify`, Prometheus
metrics, `captain security status`, `captain doctor`, and the TUI Audit screen
all expose the same redacted state. `/api/health` reports only `ok` or
`degraded`.

There is intentionally no repair method and no `/api/audit/repair` route.

| Method | Description |
|--------|-------------|
| `AuditLog::new()` | Creates an empty in-memory epoch |
| `record(...) -> Result<String, AuditError>` | Persists, then validates an entry |
| `record_or_alert(...)` | Explicit non-rollback policy with health alert |
| `verify_integrity()` | Validates active and historical integrity state |
| `integrity_status()` | Returns redacted epoch and health metadata |
| `tip_hash()` | Returns the active epoch tip |
| `len()` / `is_empty()` | Entry count |
| `recent(n)` | Returns the most recent `n` entries |

---

## 5. Heuristic Content Guards

**Source:** `captain-runtime/src/tools/security.rs`

Captain applies conservative string-pattern checks immediately before selected
shell and network sinks. These checks are defense in depth. They do **not**
track data provenance, propagate labels through model context, or prove that
unmatched content is safe.

### 5.1 Shell Command Guard

Outside explicit full-execution policy, the shell boundary rejects known shell
metacharacter injection forms and a small reviewed list of high-risk patterns
such as network-to-shell pipelines, decode-and-execute, and `eval`. The
diagnostic identifies the matched pattern without logging the complete command.

### 5.2 URL Content Guard

Web fetch, web download, web research, direct browser navigation, and browser
batch navigation reject URLs containing obvious literal credential markers
such as `api_key=`, `token=`, `secret=`, or `password=`. The diagnostic never
echoes the supplied URL. Literal secret scanning and SSRF validation remain
separate controls.

### 5.3 Explicit Limit

These guards are heuristic pattern classification, not information-flow
tracking. Content provenance across prompts, tool outputs, transformations,
and sub-agents is not implemented. A future provenance system requires typed
source metadata and propagation at every boundary; it must not be inferred
from the current checks.

---

## 6. Ed25519 Manifest Signing

**Source:** `captain-types/src/manifest_signing.rs`

Agent manifests define an agent's capabilities, tools, and configuration.
A compromised manifest can grant elevated privileges.  This module provides
Ed25519-based cryptographic signing.

### 6.1 Signing Scheme

1. Compute SHA-256 of the manifest content (raw TOML text).
2. Sign the hash with Ed25519 (via `ed25519-dalek`).
3. Bundle the signature, public key, and content hash into a `SignedManifest` envelope.

### 6.2 SignedManifest Structure

```rust
pub struct SignedManifest {
    pub manifest: String,           // Raw TOML content
    pub content_hash: String,       // Hex SHA-256 of manifest
    pub signature: Vec<u8>,         // Ed25519 signature (64 bytes)
    pub signer_public_key: Vec<u8>, // Ed25519 public key (32 bytes)
    pub signer_id: String,          // Human-readable signer ID
}
```

### 6.3 Signing

```rust
let signing_key = SigningKey::generate(&mut OsRng);
let signed = SignedManifest::sign(manifest_toml, &signing_key, "admin@org.com");
```

Internally:

```rust
pub fn sign(manifest: impl Into<String>, signing_key: &SigningKey, signer_id: impl Into<String>) -> Self {
    let manifest = manifest.into();
    let content_hash = hash_manifest(&manifest);  // SHA-256
    let signature = signing_key.sign(content_hash.as_bytes());
    let verifying_key = signing_key.verifying_key();
    Self {
        manifest,
        content_hash,
        signature: signature.to_bytes().to_vec(),
        signer_public_key: verifying_key.to_bytes().to_vec(),
        signer_id: signer_id.into(),
    }
}
```

### 6.4 Verification

Two-phase verification:

1. **Hash check:** Recompute SHA-256 of `manifest` and compare to `content_hash`.
2. **Signature check:** Verify the Ed25519 signature over `content_hash` using `signer_public_key`.

```rust
pub fn verify(&self) -> Result<(), String> {
    let recomputed = hash_manifest(&self.manifest);
    if recomputed != self.content_hash {
        return Err("content hash mismatch: ...");
    }
    let verifying_key = VerifyingKey::from_bytes(&pk_bytes)?;
    let signature = Signature::from_bytes(&sig_bytes);
    verifying_key.verify(self.content_hash.as_bytes(), &signature)
        .map_err(|e| format!("signature verification failed: {}", e))
}
```

### 6.5 Tamper Detection

- Modifying the manifest content after signing causes a **content hash mismatch**.
- Replacing the public key with a different key causes a **signature verification failure**.
- Both attacks are caught by `verify()`.

---

## 7. SSRF Protection

**Source:** `captain-runtime/src/host_functions.rs`

The `host_net_fetch` function (WASM host call for network requests) includes
comprehensive Server-Side Request Forgery protection.

### 7.1 Scheme Validation

Only `http://` and `https://` schemes are allowed.  All others (`file://`,
`gopher://`, `ftp://`) are blocked immediately:

```rust
if !url.starts_with("http://") && !url.starts_with("https://") {
    return Err(json!({"error": "Only http:// and https:// URLs are allowed"}));
}
```

### 7.2 Hostname Blocklist

Before DNS resolution, these hostnames are blocked:

- `localhost`
- `metadata.google.internal`
- `metadata.aws.internal`
- `instance-data`
- `169.254.169.254` (AWS/GCP metadata endpoint)

### 7.3 DNS Resolution Check

After the hostname blocklist, the function resolves the hostname to IP
addresses and checks **every resolved IP** against private ranges.  This
defeats DNS rebinding attacks:

```rust
let socket_addr = format!("{hostname}:{port}");
if let Ok(addrs) = socket_addr.to_socket_addrs() {
    for addr in addrs {
        let ip = addr.ip();
        if ip.is_loopback() || ip.is_unspecified() || is_private_ip(&ip) {
            return Err(json!({"error": format!(
                "SSRF blocked: {hostname} resolves to private IP {ip}"
            )}));
        }
    }
}
```

### 7.4 Private IP Detection

The `is_private_ip()` function covers:

**IPv4:**
- `10.0.0.0/8` -- RFC 1918
- `172.16.0.0/12` -- RFC 1918
- `192.168.0.0/16` -- RFC 1918
- `169.254.0.0/16` -- Link-local (AWS metadata)

**IPv6:**
- `fc00::/7` -- Unique Local Address
- `fe80::/10` -- Link-local

```rust
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            matches!(
                octets,
                [10, ..] | [172, 16..=31, ..] | [192, 168, ..] | [169, 254, ..]
            )
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80
        }
    }
}
```

### 7.5 Host Extraction

`extract_host_from_url()` parses the URL to extract `host:port` for both
SSRF checking and capability matching:

```
https://api.openai.com/v1/chat  ->  api.openai.com:443
http://localhost:8080/api       ->  localhost:8080
http://example.com              ->  example.com:80
```

### 7.6 Protected Web Context

The native `web_fetch`, `web_search`, and `web_research_batch` tools execute
only through the `WebToolsContext` built by the kernel. If that protected
context is unavailable, the request fails closed with an explicit runtime
error. Captain does not create a fallback HTTP client with weaker redirect or
SSRF rules. `web_download` uses its own bounded download engine, which applies
the same URL and resolved-address checks before every redirect.

---

## 8. Secret Zeroization

**Source:** All LLM driver modules, channel adapters, and web search modules.

Captain uses `Zeroizing<String>` from the `zeroize` crate on every field
that holds secret material.  When the value is dropped, its memory is
overwritten with zeros, preventing secrets from lingering in memory.

### 8.1 How It Works

`Zeroizing<T>` is a smart-pointer wrapper from the `zeroize` crate.  It
implements `Deref<Target=T>` for transparent usage and `Drop` for automatic
zeroization:

```rust
// On Drop, the inner String's buffer is overwritten with zeros
let key = Zeroizing::new("sk-secret-key".to_string());
// Use key transparently via Deref
client.post(url).header("authorization", format!("Bearer {}", &*key));
// When key goes out of scope, memory is zeroed
```

### 8.2 Fields Using Zeroization

**LLM Drivers** (`captain-runtime/src/drivers/`):

| Driver | Field |
|--------|-------|
| `AnthropicDriver` | `api_key: Zeroizing<String>` |
| `GeminiDriver` | `api_key: Zeroizing<String>` |
| `OpenAiCompatDriver` | `api_key: Zeroizing<String>` |

**Channel Adapters** (`captain-channels/src/`):

| Adapter | Field(s) |
|---------|----------|
| `DiscordAdapter` | `token: Zeroizing<String>` |
| `EmailAdapter` | `password: Zeroizing<String>` |
| `BlueskyAdapter` | `app_password: Zeroizing<String>` |
| `DingTalkAdapter` | `access_token: Zeroizing<String>`, `secret: Zeroizing<String>` |
| `FeishuAdapter` | `app_secret: Zeroizing<String>` |
| `FlockAdapter` | `bot_token: Zeroizing<String>` |
| `GitterAdapter` | `token: Zeroizing<String>` |
| `GotifyAdapter` | `app_token: Zeroizing<String>`, `client_token: Zeroizing<String>` |

**Web Search** (`captain-runtime/src/web_search.rs`):

```rust
fn resolve_api_key(env_var: &str) -> Option<Zeroizing<String>> {
    std::env::var(env_var).ok().filter(|k| !k.is_empty()).map(Zeroizing::new)
}
```

**Embedding** (`captain-runtime/src/embedding.rs`):

| Struct | Field |
|--------|-------|
| `EmbeddingClient` | `api_key: Zeroizing<String>` |

### 8.3 Why It Matters

Without zeroization, secrets remain in memory after use until the OS
reclaims the page.  An attacker with access to a core dump, swap file, or
memory forensics tool can recover API keys.  `Zeroizing<String>` ensures
the secret is overwritten as soon as it is no longer needed.

### 8.4 Native Vault Master Key

`vault.enc` is encrypted with AES-256-GCM, but its master key is not stored in
that file or in Captain configuration. Captain uses the platform credential
store directly: macOS Keychain, Windows Credential Manager, or Linux Secret
Service. Every new key is read back and compared before vault initialization
can succeed. A storage error fails closed and no code path prints the generated
key.

`CAPTAIN_VAULT_KEY` is an explicit base64-encoded 32-byte override for
headless and CI environments. It takes precedence when supplied; operators are
responsible for injecting it through their secret manager rather than a
committed environment file.

The obsolete local `.keyring` format is migration-only. Captain decodes and
validates it, persists and verifies the same key in the native store, then
removes the weak copy. Failed persistence leaves the legacy copy untouched; a
native/legacy mismatch refuses both automatic deletion and overwrite.
An unreadable or malformed legacy copy is likewise never ignored silently;
unlock stops with an operator-facing remediation and leaves the file intact.

Every vault mutation now holds a bounded cross-process lock around the full
reload, change, and durable atomic-write cycle. A CLI command and the daemon
therefore merge changes made from stale in-memory snapshots instead of letting
the last writer erase unrelated credentials. The lock wait fails after ten
seconds with a retryable error; a crashed process releases its OS lock, and the
lock file itself contains no secret material.

### 8.5 External Secret Files

Production deployments can map logical credential keys to read-only files in
`$CAPTAIN_HOME/secret-sources.toml`. This supports container secrets, systemd
credentials, and secret-manager sidecars without copying values into Captain's
configuration or process environment.

The mapping is authoritative and fail-closed. If a configured source is
missing, unreadable, unsafe, empty, oversized, or malformed, Captain does not
fall back to `secrets.env`, `vault.enc`, `.env`, or an environment variable
with the same key. The registry accepts only absolute `type = "file"` entries;
arbitrary command execution is intentionally unsupported.

At load and read time Captain enforces a versioned strict schema, bounded
registry/source sizes, regular-file type, UTF-8 single-line values, and Unix
write permissions. Both registry and secret reads use one opened descriptor to
avoid path-swap races. Group/world-writable files are rejected; a read-only
group/world-readable mount is reported as a warning. Resolved values use
`Zeroizing<String>`.

The registry and all configured source targets join the kernel's canonical
file-tool blocklist. Canonical path checks, including symlink resolution, keep
them inaccessible even when a user adds their parent directory as a workspace.
Readiness surfaces disclose only key, source type, readiness, and stable
error/warning codes. Values and individual source paths are never serialized.
Local writes to an externally managed key are refused, and signed outbound
event webhooks and per-agent callbacks fail closed rather than sending an
unsigned request when their mapped secret is unavailable. A durable per-agent
callback remains queued while its authoritative URL or signing secret is
temporarily unavailable. Per-agent ingress authentication uses the same
resolver and constant-time comparison.

---

## 9. OFP Mutual Authentication

**Source:** `captain-wire/src/peer.rs`

The Captain Wire Protocol (OFP) uses HMAC-SHA256 with nonce-based mutual
authentication over TCP connections.

### 9.1 Pre-Shared Key Requirement

OFP refuses to start without a `shared_secret`:

```rust
if config.shared_secret.is_empty() {
    return Err(WireError::HandshakeFailed(
        "OFP requires shared_secret. Set [network] shared_secret in config.toml".into(),
    ));
}
```

### 9.2 HMAC Functions

```rust
type HmacSha256 = Hmac<Sha256>;

fn hmac_sign(secret: &str, data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key size");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

fn hmac_verify(secret: &str, data: &[u8], signature: &str) -> bool {
    let expected = hmac_sign(secret, data);
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into()
}
```

**Constant-time comparison** (`subtle::ConstantTimeEq`) prevents
timing side-channel attacks.

### 9.3 Handshake Protocol

**Initiator (client):**

1. Generate a random UUID nonce.
2. Compute `auth_data = nonce + node_id`.
3. Compute `auth_hmac = hmac_sign(shared_secret, auth_data)`.
4. Send `Handshake { node_id, node_name, protocol_version, agents, nonce, auth_hmac }`.

**Responder (server):**

1. Receive the `Handshake` message.
2. Verify the incoming HMAC: `hmac_verify(shared_secret, nonce + node_id, auth_hmac)`.
3. If verification fails, return error code 403.
4. Generate a new UUID nonce for the ack.
5. Compute `ack_auth_data = ack_nonce + self.node_id`.
6. Compute `ack_hmac = hmac_sign(shared_secret, ack_auth_data)`.
7. Send `HandshakeAck { node_id, node_name, protocol_version, agents, nonce: ack_nonce, auth_hmac: ack_hmac }`.

**Initiator (verification):**

1. Receive `HandshakeAck`.
2. Verify: `hmac_verify(shared_secret, ack_nonce + node_id, ack_hmac)`.
3. If verification fails, return `WireError::HandshakeFailed`.

### 9.4 Security Properties

| Property | How It Is Achieved |
|----------|-------------------|
| **Mutual authentication** | Both sides prove knowledge of the shared secret |
| **Replay protection** | Random UUID nonces per handshake |
| **Timing-attack resistance** | `subtle::ConstantTimeEq` for HMAC comparison |
| **Mandatory secret** | OFP refuses to start with an empty `shared_secret` |
| **Message size limit** | `MAX_MESSAGE_SIZE = 16 MB` prevents memory DoS |
| **Protocol version check** | `PROTOCOL_VERSION` mismatch returns `WireError::VersionMismatch` |

---

## 10. Security Headers

**Source:** `captain-api/src/middleware.rs`

The `security_headers` middleware is applied to **all** API responses:

```rust
pub async fn security_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-xss-protection", "0".parse().unwrap());
    headers.insert("content-security-policy", /* CSP policy */);
    headers.insert("referrer-policy", "strict-origin-when-cross-origin".parse().unwrap());
    headers.insert("cache-control", "no-store, no-cache, must-revalidate".parse().unwrap());
    response
}
```

| Header | Value | Protects Against |
|--------|-------|------------------|
| `X-Content-Type-Options` | `nosniff` | MIME type sniffing attacks |
| `X-Frame-Options` | `DENY` | Clickjacking via iframes |
| `X-XSS-Protection` | `0` | Disable legacy filters that can mutate otherwise safe markup |
| `Content-Security-Policy` | See below | XSS, code injection, data exfiltration |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | Referrer leakage |
| `Cache-Control` | `no-store, no-cache, must-revalidate` | Sensitive data caching |

### 10.1 CSP Breakdown

| Directive | Value | Purpose |
|-----------|-------|---------|
| `default-src` | `'self'` | Deny all external resources by default |
| `script-src` | `'self'` | Only finite embedded same-origin script assets |
| `script-src-attr` | `'none'` | Reject HTML event-handler attributes |
| `style-src` | `'self' 'unsafe-inline'` | Same-origin styles plus bounded first-party dynamic layout |
| `img-src` | `'self' data: blob:` | Same-origin and local generated images |
| `connect-src` | `'self'` plus loopback `ws:`/`wss:` | Same-origin API and declared local realtime transports |
| `font-src` | `'self'` | No external font authority |
| `worker-src` / `manifest-src` | `'self'` | Embedded PWA worker and manifest only |
| `object-src` | `'none'` | Block all plugins (Flash, Java, etc.) |
| `base-uri` | `'none'` | Prevent base tag hijacking |
| `frame-ancestors` | `'none'` | Prevent framing in modern browsers |

Control's import map and the Terminal/Config inline script bundles were
removed. All browser JavaScript is now served from the finite embedded
`/assets/app/` map, so the executable policy needs neither `unsafe-inline` nor
`unsafe-eval`. Inline CSS remains explicit because first-party components use
bounded dynamic width/visibility values; attacker-controlled Markdown cannot
carry `style`.

LLM Markdown is parsed and then sanitized with a fixed passive tag and
attribute allowlist. Forms, inputs, SVG, styles, frames, objects, embeds,
scripts, data attributes, event handlers, and unsafe URL protocols are
discarded. Links are restricted to HTTP(S), `mailto`, or `tel`, then receive
`target="_blank"` with `rel="noopener noreferrer"`. Tool output and session
labels use Preact text nodes. `scripts/control-xss-smoke.mjs` runs malicious
probes through all three paths in Chromium under the production CSP.
| `form-action` | `'self'` | Restrict form submission targets |

---

## 11. GCRA Rate Limiter

**Source:** `captain-api/src/rate_limiter.rs`

Captain uses the Generic Cell Rate Algorithm (GCRA) for cost-aware API
rate limiting via the `governor` crate.

### 11.1 Algorithm

GCRA is a leaky-bucket variant that tracks a single "virtual scheduling time"
(TAT -- Theoretical Arrival Time) per key.  Each request consumes a number of
tokens proportional to its cost.  The bucket refills at a constant rate.

**Budget:** 500 tokens per minute per IP address.

```rust
pub fn create_rate_limiter() -> Arc<KeyedRateLimiter> {
    Arc::new(RateLimiter::keyed(Quota::per_minute(NonZeroU32::new(500).unwrap())))
}
```

### 11.2 Operation Costs

Each API operation has a configurable token cost:

```rust
pub fn operation_cost(method: &str, path: &str) -> NonZeroU32 {
    match (method, path) {
        (_, "/api/health")                            => 1,
        ("GET", "/api/status")                        => 1,
        ("GET", "/api/version")                       => 1,
        ("GET", "/api/tools")                         => 1,
        ("GET", "/api/agents")                        => 2,
        ("GET", "/api/skills")                        => 2,
        ("GET", "/api/peers")                         => 2,
        ("GET", "/api/config")                        => 2,
        ("GET", "/api/usage")                         => 3,
        ("GET", p) if p.starts_with("/api/audit")     => 5,
        ("POST", "/api/agents")                       => 50,
        ("POST", p) if p.contains("/message")         => 30,
        ("POST", p) if p.contains("/run")             => 100,
        ("POST", "/api/skills/uninstall")             => 10,
        ("PUT", p) if p.contains("/update")           => 10,
        _                                             => 5,
    }
}
```

The cost hierarchy is intentional: read-only health checks cost 1 token while
expensive operations like workflow runs cost 100, meaning a client can perform
500 health checks per minute but only 5 workflow runs.

### 11.3 Middleware

```rust
pub async fn gcra_rate_limit(
    State(limiter): State<Arc<KeyedRateLimiter>>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let ip = /* extract from ConnectInfo, default 127.0.0.1 */;
    let cost = operation_cost(&method, &path);

    if limiter.check_key_n(&ip, cost).is_err() {
        tracing::warn!(ip, cost, path, "GCRA rate limit exceeded");
        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("retry-after", "60")
            .body(/* JSON error */)
            .unwrap_or_default();
    }
    next.run(request).await
}
```

### 11.4 Rate Limiter Type

```rust
pub type KeyedRateLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;
```

The `DashMapStateStore` provides concurrent per-IP state with automatic stale
entry cleanup.

---

## 12. Path Traversal Prevention

**Source:** `captain-runtime/src/host_functions.rs`

Two functions provide defense-in-depth against directory traversal.

### 12.1 safe_resolve_path (for reads)

Used for `fs_read` and `fs_list` operations where the target file must exist:

```rust
fn safe_resolve_path(path: &str) -> Result<std::path::PathBuf, serde_json::Value> {
    let p = Path::new(path);

    // Phase 1: Reject any path with ".." components
    for component in p.components() {
        if matches!(component, Component::ParentDir) {
            return Err(json!({"error": "Path traversal denied: '..' components forbidden"}));
        }
    }

    // Phase 2: Canonicalize to resolve symlinks and normalize
    std::fs::canonicalize(p)
        .map_err(|e| json!({"error": format!("Cannot resolve path: {e}")}))
}
```

### 12.2 safe_resolve_parent (for writes)

Used for `fs_write` operations where the target file may not exist yet:

```rust
fn safe_resolve_parent(path: &str) -> Result<std::path::PathBuf, serde_json::Value> {
    let p = Path::new(path);

    // Phase 1: Reject ".." in any component
    for component in p.components() {
        if matches!(component, Component::ParentDir) {
            return Err(json!({"error": "Path traversal denied: '..' components forbidden"}));
        }
    }

    // Phase 2: Canonicalize the parent directory
    let parent = p.parent().filter(|par| !par.as_os_str().is_empty())
        .ok_or_else(|| json!({"error": "Invalid path: no parent directory"}))?;
    let canonical_parent = std::fs::canonicalize(parent)?;

    // Phase 3: Belt-and-suspenders check on filename
    let file_name = p.file_name()
        .ok_or_else(|| json!({"error": "Invalid path: no file name"}))?;
    if file_name.to_string_lossy().contains("..") {
        return Err(json!({"error": "Path traversal denied in file name"}));
    }

    Ok(canonical_parent.join(file_name))
}
```

### 12.3 Enforcement Order

1. **Capability check** runs first with the raw path.
2. **Path traversal check** runs second.
3. **Operation** runs only if both pass.

This ordering ensures that even if a capability is misconfigured with a broad
pattern like `"*"`, path traversal is still blocked.

---

## 13. Guarded Subprocess Boundary

**Sources:** `captain-runtime/src/guarded_exec.rs`,
`captain-runtime/src/subprocess_env_scrub.rs`,
`captain-runtime/src/subprocess_guard.rs`

Agent-controlled execution has one shared boundary. It covers the shell tool,
goal checks and recovery, Markdown skill capabilities, `execute_code`,
workflow shell actions, static skill checks, package wrappers, Hand dependency
installation, WASM host execution, and supervised `process_start`.

Before a process can start, `guarded_exec` applies the active execution policy,
critical-pattern decision, literal-secret and `secrets.env` guards, executable
path validation, an explicit workspace, bounded runtime/output, and structured
audit events that never contain the command body. The interactive shell tool
can return a one-shot approval requirement. Unattended surfaces cannot invent
an approval: critical content fails closed.

The permit binding is separate from presentation. Shell permits bind to the
exact reviewed content and surface. Direct-program permits bind to a
domain-separated SHA-256 preimage beginning with
`captain.exec-permit.program.v1`, followed by a big-endian `u64` length and the
raw executable bytes, a big-endian `u64` argument count, then a big-endian
`u64` length and raw bytes for every argument. The review string is a readable,
escaped representation only; it grants no authority. The binary encoding is
injective across executable and argument boundaries even when an input
contains NUL.

### 13.1 Environment Clearing

Every covered process is constructed or configured by `guarded_exec`. It calls
`env_clear()` first, restores only the safe host allowlist and any
caller-authorized variable names, then applies explicit per-call values.
Credentials intentionally injected for one skill are therefore available to
that skill, while every unrelated value loaded into the daemon from
`secrets.env` remains absent.

### 13.2 Safe Environment Variables

**All platforms:**

```rust
pub const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
];
```

**Windows-only:**

```rust
pub const SAFE_ENV_VARS_WINDOWS: &[&str] = &[
    "USERPROFILE", "SYSTEMROOT", "APPDATA", "LOCALAPPDATA",
    "COMSPEC", "WINDIR", "PATHEXT",
];
```

Variables not in these lists, not explicitly allowlisted by name, and not
provided as explicit per-call values are **never** passed to the child process.
This means `OPENAI_API_KEY`, `GEMINI_API_KEY`, database credentials, and all
other daemon secrets are stripped by default.

### 13.3 Executable Path Validation

```rust
pub fn validate_executable_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    for component in p.components() {
        if let std::path::Component::ParentDir = component {
            return Err(format!(
                "executable path '{}' contains '..' component which is not allowed",
                path
            ));
        }
    }
    Ok(())
}
```

This prevents an agent from escaping its working directory via crafted paths
like `../../bin/dangerous`.

### 13.4 Execution Policy and Regression Guard

Allowlist mode rejects shell metacharacters before direct argument parsing and
requires the executable to be explicitly allowed. Full mode still applies the
configured blocklist and critical-pattern policy. The structural default is
`allowlist`; guided local setup writes an explicit
`personal_workstation`/`full` choice for a trusted single-user host.
`remote_operator` imposes effective allowlist semantics, while
`untrusted_execution` denies all agent-controlled host starts. An agent policy
is intersected with the daemon policy before visibility and dispatch, including
mode, allowlists, blocklists, limits, critical mode, and profile. `open` can
return a content-bound approval requirement, and `paranoid` requests approval
for every shell-affecting operation.

Critical-command recognition normalizes case, whitespace, short/long flag
forms, common wrappers, and nested `sh`/`bash`/`zsh`/`dash -c` or `eval`
payloads before structural checks. This is a
`normalized_lexical_heuristic`, not proof that arbitrary shell code is safe.
WASM host execution passes arguments directly without shell parsing, but it
uses the same content review, environment scrub, output bound, timeout, and
audit lifecycle.

`scripts/guarded-exec-audit.sh` is mandatory in both tranche and release gates.
It fails if a covered sink creates a raw process or mutates a child environment
outside `guarded_exec`; a new literal `bash`, `sh`, or `cmd` constructor
anywhere else also requires an explicit fixed-command review marker.

### 13.5 Exact Host Isolation Posture

The native execution backend reports:

```json
{
  "profile": "untrusted_execution",
  "backend": "host_process",
  "isolation_level": "environment_scrub",
  "os_isolation": false,
  "environment_scrub": true,
  "dangerous_command_guard": "normalized_lexical_heuristic",
  "configured_policy_mode": "full",
  "policy_mode": "deny",
  "host_execution_allowed": false,
  "isolation_routing": "explicit_only",
  "explicit_isolation_backends": ["docker_exec", "wasm_agent"]
}
```

`env_clear()`, workspace validation, process-tree cleanup, and runtime/output
bounds are host-process protections. They are not namespace, seccomp,
Landlock, chroot, or container isolation. Operators must choose the explicit
Docker or WASM backend when an operating-system isolation boundary is required.
Captain never auto-routes and never falls back to the host. An enabled Docker
rail for `untrusted_execution` must use network `none`, a read-only root,
dropped capabilities, a read-only workspace mount, and finite CPU, memory, and
PID limits; daemon availability remains a live invocation-time check.

---

## 14. Advisory Skill Phrase Review

**Source:** `captain-skills/src/verify.rs`

The `SkillVerifier` provides manifest review signals through `security_scan()`
and a separately qualified prompt-text report through
`scan_prompt_content_advisory()`.

### 14.1 Manifest Security Scan

`SkillVerifier::security_scan(manifest)` inspects a skill's declared
requirements:

| Check | Severity | Trigger |
|-------|----------|---------|
| Node.js runtime | Warning | `runtime_type == SkillRuntime::Node` |
| Shell execution capability | Critical | Capability contains `shellexec` or `shell_exec` |
| Unrestricted network | Warning | Capability contains `netconnect(*)` |
| Shell tool | Critical | Tool is `shell_exec` or `bash` |
| Filesystem write tool | Warning | Tool is `file_write` or `file_delete` |
| Too many tools | Info | More than 10 tools required |

### 14.2 Prompt-Text Heuristic

`SkillVerifier::scan_prompt_content_advisory(content)` performs lowercase
phrase matching in skill prompt text. Every result carries
`assurance = advisory_heuristic`. A finding is not proof of an attack, and an
empty report is not proof of safety. The registry applies a conservative policy
that refuses high-risk matches; this policy does not upgrade the scanner's
assurance.

**Critical -- Prompt override attempts:**

```
"ignore previous instructions", "ignore all previous",
"disregard previous", "forget your instructions",
"you are now", "new instructions:", "system prompt override",
"ignore the above", "do not follow", "override system"
```

**Warning -- Data exfiltration patterns:**

```
"send to http", "send to https", "post to http", "post to https",
"exfiltrate", "forward all", "send all data",
"base64 encode and send", "upload to"
```

**Warning -- Shell command references:**

```
"rm -rf", "chmod ", "sudo "
```

**Info -- Excessive length:**

Content over 50,000 bytes triggers an info-level warning about potential LLM
performance degradation.

### 14.3 SHA256 Checksum Verification

```rust
pub fn verify_checksum(data: &[u8], expected_sha256: &str) -> bool {
    let actual = Self::sha256_hex(data);
    actual == expected_sha256.to_lowercase()
}
```

Remote marketplace compatibility is frozen because Captain cannot currently
bind downloaded content to a reviewed publisher identity. Its API routes and
TUI actions are absent, the CLI accepts only an existing local directory, and
retained compatibility clients fail before network or filesystem access.
SHA256 can detect change only when the expected digest comes from a trusted
channel; a downloaded self-declared digest is not publisher authentication.

### 14.4 Warning Structure

```rust
pub struct SkillWarning {
    pub severity: WarningSeverity,  // Info, Warning, Critical
    pub message: String,
}
```

---

## 15. Loop Guard

**Source:** `captain-runtime/src/loop_guard.rs`

The `LoopGuard` tracks tool calls within a single agent loop execution to
detect when the agent is stuck calling the same tool repeatedly.

### 15.1 Configuration

```rust
pub struct LoopGuardConfig {
    pub warn_threshold: u32,         // Default: 3
    pub block_threshold: u32,        // Default: 5
    pub global_circuit_breaker: u32, // Default: 30
}
```

### 15.2 Detection Algorithm

1. For each tool call, compute SHA-256 of `tool_name + "|" + serialized_params`.
2. Increment the count for that hash in a `HashMap<String, u32>`.
3. Increment `total_calls`.
4. Return a graduated verdict:

```rust
pub fn check(&mut self, tool_name: &str, params: &serde_json::Value) -> LoopGuardVerdict {
    self.total_calls += 1;

    // Global circuit breaker
    if self.total_calls > self.config.global_circuit_breaker {
        return LoopGuardVerdict::CircuitBreak(/* ... */);
    }

    let hash = Self::compute_hash(tool_name, params);
    let count = self.call_counts.entry(hash).or_insert(0);
    *count += 1;

    if *count >= self.config.block_threshold {
        LoopGuardVerdict::Block(/* ... */)
    } else if *count >= self.config.warn_threshold {
        LoopGuardVerdict::Warn(/* ... */)
    } else {
        LoopGuardVerdict::Allow
    }
}
```

### 15.3 Verdict Types

| Verdict | Meaning | Action |
|---------|---------|--------|
| `Allow` | Normal operation | Run the tool |
| `Warn(msg)` | Same call repeated >= 3 times | Run, append warning to result |
| `Block(msg)` | Same call repeated >= 5 times | Skip execution, return error |
| `CircuitBreak(msg)` | > 30 total tool calls | Terminate the entire agent loop |

### 15.4 Hash Computation

```rust
fn compute_hash(tool_name: &str, params: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tool_name.as_bytes());
    hasher.update(b"|");
    let params_str = serde_json::to_string(params).unwrap_or_default();
    hasher.update(params_str.as_bytes());
    hex::encode(hasher.finalize())
}
```

Note: `serde_json::to_string` produces deterministic output (object keys are
sorted), ensuring that semantically identical parameters produce the same hash.

### 15.5 Key Property

Calls with **different parameters** are tracked separately.  An agent that
calls `web_search` with 10 different queries will not trigger the guard, but
an agent that calls `web_search({"query": "test"})` 5 times will be blocked.

---

## 16. Session Repair

**Source:** `captain-runtime/src/session_repair.rs`

Before sending message history to the LLM, this module validates and repairs
common structural issues that would cause API errors.

### 16.1 Three-Phase Repair

```rust
pub fn validate_and_repair(messages: &[Message]) -> Vec<Message>
```

**Phase 1 -- Collect ToolUse IDs:**

Scan all messages for `ContentBlock::ToolUse { id, .. }` blocks and collect
their IDs into a `HashSet<String>`.

**Phase 2 -- Filter orphans and empties:**

- **Orphaned ToolResults:** `ContentBlock::ToolResult { tool_use_id, .. }`
  blocks where `tool_use_id` is not in the ToolUse ID set are dropped.
- **Empty messages:** Messages with empty text or no content blocks are
  dropped.

**Phase 3 -- Merge consecutive same-role messages:**

The Anthropic API requires strict role alternation (user, assistant, user,
assistant...).  If two consecutive messages have the same role, they are
merged into a single message with combined content blocks.

### 16.2 Why Each Repair Is Needed

| Issue | Cause | Effect Without Repair |
|-------|-------|----------------------|
| Orphaned ToolResult | Compaction or truncation removed the ToolUse | API error: "tool_use_id not found" |
| Empty messages | Cancelled generation, empty user submission | API error: empty content |
| Consecutive same-role | Manual history editing, session repair itself | API error: role alternation violation |

### 16.3 Content Merging

When merging consecutive same-role messages, both are converted to block
format and concatenated:

```rust
fn merge_content(dst: &mut MessageContent, src: MessageContent) {
    let dst_blocks = content_to_blocks(std::mem::replace(dst, MessageContent::Text(String::new())));
    let src_blocks = content_to_blocks(src);
    let mut combined = dst_blocks;
    combined.extend(src_blocks);
    *dst = MessageContent::Blocks(combined);
}
```

---

## 17. Health Endpoint Redaction

**Source:** `captain-api/src/routes.rs`

Captain provides two health endpoints with different information levels.

### 17.1 Public Endpoint: `GET /api/health`

**No authentication required.**  Returns only liveness information:

```json
{
    "status": "ok",
    "version": "0.1.0"
}
```

This endpoint does not expose agent count, database details, configuration
warnings, uptime, or any internal system information.  It is suitable for
load balancer health checks.

### 17.2 Detail Endpoint: `GET /api/health/detail`

**Requires authentication.**  Returns full diagnostics:

```json
{
    "status": "ok",
    "version": "0.1.0",
    "uptime_seconds": 3600,
    "failure_count": 4,
    "panic_count": 0,
    "restart_count": 2,
    "agent_count": 15,
    "database": "connected",
    "audit": {
        "valid": true,
        "status": "healthy",
        "active_epoch": 0,
        "active_epoch_valid": true,
        "invalid_epochs": [],
        "entry_count": 42,
        "tip_hash": "8f6d..."
    },
    "config_warnings": []
}
```

### 17.3 Deny-by-Default Authentication Perimeter

The global authentication bypass is one typed `PUBLIC_ALLOWLIST`. It contains
only the Control boot/static files, minimal health and version responses, the
browser login/check/logout endpoints, and the exact per-agent ingress route.
Every other method/path pair is private. In particular, `/terminal`, `/config`,
the A2A agent card and task routes, operational status, agents, sessions,
approvals, logs, budgets, providers, and the GitHub Copilot OAuth flow require
global authentication.

The per-agent ingress exception is not anonymous access. Only
`POST /hooks/agents/{uuid}/ingress` bypasses the global middleware, and its
handler applies the agent-specific Bearer token, body bounds, idempotency, and
rate limit before an agent turn. Malformed IDs and extra path segments remain
behind global authentication.

When both the API key and browser authentication are disabled, the auth layer
allows local development requests. Daemon startup separately refuses a
non-loopback bind without an API key. That deployment boundary does not add
paths to the public allowlist.

### 17.4 Browser Origin and Host Perimeter

CORS is restrictive regardless of whether a daemon API key exists. The
default exact origins are `http://localhost:{api_port}`,
`http://127.0.0.1:{api_port}`, and the IPv6 loopback equivalent. Captain
allows only the reviewed `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`, and
`OPTIONS` methods and the `Accept`, `Authorization`, `Content-Type`, and
`X-Filename` request headers. It never switches to wildcard origins, methods,
or headers because authentication is absent or present.

Operators can extend the list with exact HTTP(S) origins in
`[api].allowed_origins`. `deployment.public_url` is an additional explicit
origin for a declared reverse-proxy deployment. Invalid entries are ignored
fail-closed and reported without logging their raw value. Policy changes
require a daemon restart because CORS and request middleware are built at
server startup.

Every request also passes an exact `Host` check before routing. Loopback hosts,
the concrete non-wildcard listen address, hosts derived from configured
origins, and the host in `deployment.public_url` are accepted. Missing,
ambiguous, malformed, and undeclared hosts return `400`. This check is the DNS
rebinding boundary; CORS alone is not treated as one.

---

## 18. Security Configuration

### 18.1 config.toml Reference

```toml
# API Authentication
api_key = "your-secret-api-key"  # Empty = localhost-only mode

[api]
allowed_origins = []  # Exact additional HTTP(S) origins; restart after changes

[auth]
enabled = true
allow_unauthenticated_loopback = false
session_ttl_hours = 72
session_cookie_secure = "auto"  # auto, always, or never for explicit local HTTP

# OFP Wire Protocol
[network]
shared_secret = "your-pre-shared-key"  # Required for OFP

# WASM Sandbox
[sandbox]
fuel_limit = 1000000       # CPU instruction budget per execution
timeout_secs = 30          # Wall-clock timeout per execution
max_memory_bytes = 16777216 # 16 MB max WASM memory

# Rate Limiting
# 500 tokens/minute/IP (not currently configurable via config.toml)
# Web login also has bounded per-IP + per-username exponential backoff.
# Active login blocks are never evicted under the 4096-key capacity limit.
# Full active maps trigger a logged five-second global fail-closed backoff.

# Web Search SSRF Protection
[web]
# SSRF protection is always on and cannot be disabled
```

Web passwords are salted Argon2id PHC strings. Legacy SHA-256 hashes migrate
atomically after one successful login. Browser session cookies are HttpOnly,
SameSite=Strict, and `Secure` according to `session_cookie_secure`. Browser
WebSocket/SSE connections use 30-second path/IP/epoch-bound one-time tickets;
protected routes never authenticate from a `token` query parameter.

Login attempts use separate process-local IP and normalized-username maps,
bounded to 4,096 keys each. Capacity pressure may evict only a record with no
active retry delay; an active block is retained. If every slot is actively
blocked, Captain logs limiter saturation and applies one shared five-second
`429` backoff. A restart clears this bounded state, so an Internet-facing
deployment must also enforce login limits at its reverse proxy, firewall, WAF,
or equivalent edge.

Protected API and web routes fail closed when neither a daemon API key nor
browser-session auth is configured. Credentialless development access requires
the explicit `auth.allow_unauthenticated_loopback = true` opt-out and the
actual client must be loopback. Missing peer metadata and remote clients behind
a declared local reverse proxy are denied. Ambiguous or multi-hop
`X-Forwarded-For` values are also denied for this credentialless mode.
`captain setup` is the supported remediation and always writes the fail-closed
value.

### 18.2 Environment Variables for Secrets

| Variable | Used By |
|----------|---------|
| `OPENAI_API_KEY` | OpenAI-compat driver |
| `ANTHROPIC_API_KEY` | Anthropic driver |
| `GEMINI_API_KEY` or `GOOGLE_API_KEY` | Gemini driver |
| `DEEPSEEK_API_KEY` | DeepSeek provider |
| `GROQ_API_KEY` | Groq provider |
| `BRAVE_API_KEY` | Brave web search |
| `TAVILY_API_KEY` | Tavily web search |
| `PERPLEXITY_API_KEY` | Perplexity web search |

All environment variable API keys are wrapped in `Zeroizing<String>` when
loaded into driver structs.

### 18.3 Capability Declaration (Agent Manifest)

Capabilities are declared in the agent's TOML manifest:

```toml
[agent]
name = "my-agent"

[[capabilities]]
type = "FileRead"
value = "/data/*"

[[capabilities]]
type = "NetConnect"
value = "*.openai.com:443"

[[capabilities]]
type = "ToolInvoke"
value = "web_search"

[[capabilities]]
type = "LlmMaxTokens"
value = 4096
```

### 18.4 Loop Guard Tuning

The default `LoopGuardConfig` values are:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `warn_threshold` | 3 | Identical calls before warning |
| `block_threshold` | 5 | Identical calls before blocking |
| `global_circuit_breaker` | 30 | Total calls before circuit break |

### 18.5 Subprocess Environment Allowlists

Agent-controlled child processes must enter through `guarded_exec`; callers do
not configure a lower-level sandbox helper directly. The boundary clears the
environment, restores the fixed safe defaults, then adds only names explicitly
listed in `allowed_env_vars` and values intentionally supplied for that call.
This is environment scrubbing, not operating-system isolation.

---

## 19. Security Dependencies

| Crate | Purpose |
|-------|---------|
| `sha2` | SHA-256 hashing (audit trail, loop guard, SSRF, checksums) |
| `hmac` | HMAC-SHA256 for OFP authentication |
| `hex` | Hex encoding/decoding of hashes and signatures |
| `subtle` | Constant-time comparison (`ConstantTimeEq`) for HMAC verification |
| `ed25519-dalek` | Ed25519 signing/verification for manifest signing |
| `rand` | Cryptographic RNG for key generation (`OsRng`) |
| `zeroize` | `Zeroizing<T>` wrapper for automatic secret memory wiping |
| `governor` | GCRA rate limiting algorithm |
| `wasmtime` | WASM sandbox with fuel + epoch metering |
| `uuid` | Nonce generation for OFP handshakes |
| `chrono` | ISO-8601 timestamps for audit entries |
| `reqwest` | HTTP client (used inside SSRF-protected `host_net_fetch`) |

### 19.1 Why These Specific Crates

- **sha2/hmac:** Part of the RustCrypto project, audited, widely used in production Rust.
- **ed25519-dalek:** De facto standard Ed25519 library in Rust, extensively audited.
- **subtle:** Provides constant-time operations to prevent timing side-channels.
- **zeroize:** Official RustCrypto approach to zeroing secrets; integrates with `Drop`.
- **governor:** Battle-tested GCRA implementation with `DashMap`-backed concurrent state.

### 19.2 Dependency Audit Policy

Release readiness executes `scripts/dependency-audit.sh`. The script does not
trust the configured ignore list by itself:

1. the normal RustSec audit must contain no unreviewed vulnerability;
2. an unfiltered audit from outside the repository re-exposes every configured
   exception;
3. package names, versions, advisory IDs, enabled features, and direct parent
   chains must match the reviewed baseline exactly;
4. `bincode`, `rsa`, `pkcs1`, and `num-bigint-dig` must remain absent;
5. `russh` and both resolved `ssh-key` versions must have no RSA feature.

`fastembed 5.13.2` remains pinned to `ort 2.0.0-rc.11` because Captain ships
ONNX Runtime 1.23.2 on every release target. FastEmbed 5.17 requires the ORT
rc13/ONNX Runtime 1.28 ABI; that upgrade is deferred until all five release
targets certify the native runtime together. Its `number_prefix 0.4.0`
warning is therefore accepted transitively and pinned exactly.

`spin 0.9.8` is yanked but not covered by a vulnerability advisory. It is
present only through `flume 0.12.0`, itself required by `mdns-sd 0.20.3`, and
is pinned as an explicit release warning rather than hidden.

The Email rail pins `imap 3.0.0-alpha.15` and its maintained
`imap-proto 0.16.7` parser. This removes the `imap-proto 0.10.2`
future-incompatibility warnings and the unsound `lexical-core 0.7.6` parser
chain. The prerelease keeps the synchronous IMAP contract used by Captain and
is pinned exactly until upstream publishes 3.0 stable. The adapter explicitly
selects implicit TLS for every configured IMAP port, preserving the previous
transport contract and preventing an automatic switch to STARTTLS on custom
ports. The dependency gate also requires its sole parser parent, `native-tls`
feature, and vendored TLS path to remain exact.

The two reviewed vulnerability exceptions are RUSTSEC-2026-0194 and
RUSTSEC-2026-0195 on `quick-xml 0.37.5`. The only parent is
`tauri-winrt-notification 0.7.2` in the Windows desktop notification chain.
That crate calls `quick_xml::escape::escape` only; it does not call the
affected `NsReader`, `Attributes`, or `try_get_attribute` parser paths. The
other plist/Tauri path has been upgraded to `quick-xml 0.41.0`. The exceptions
must be removed when `notify-rust` accepts
`tauri-winrt-notification >=0.8`.

RSA SSH private keys and RSA-only server host keys are intentionally
unsupported while RUSTSEC-2023-0071 has no fixed upstream implementation.
Captain accepts Ed25519 and ECDSA P-256 private keys and returns an actionable
error when an unsupported key is imported or loaded.

### 19.3 Release Provenance

The local release publisher generates one in-toto Statement v1 with a SLSA
provenance v1 predicate after all five host targets have completed. It binds
the 20 archive, checksum, manifest, and installer assets to the public Git
commit and tree, the exact `Cargo.lock`, local toolchain identity, target set,
and platform-manifest timestamps. Its verifier recomputes the complete subject
set and rejects any modified or mixed-revision release.

Docker `linux/amd64` and `linux/arm64` are built and pushed sequentially by
digest with BuildKit provenance `mode=max`. The combined version/channel index
is created only after both digests are remotely inspectable, and verification
requires both image platforms plus their attestation manifests.

The alpha host attestation and sidecar are not independently signed and do not
claim SLSA build-level certification. This limitation, plus the ad-hoc macOS
signature and absent Windows Authenticode signature, remains public in
[`docs/release-provenance.md`](release-provenance.md).

---

## Threat Model Summary

| Threat | Mitigated By |
|--------|-------------|
| Agent requests unauthorized file access | Capability-based security (Section 2) |
| Agent spawns child with elevated privileges | Capability inheritance validation (Section 2.4) |
| WASM skill runs infinite loop | Dual metering: fuel + epoch (Section 3) |
| Attacker tampers with audit log | Versioned hash chain and immutable recovery epochs (Section 4) |
| Obvious dangerous shell/URL text | Heuristic content guards (Section 5) |
| Literal secret exfiltration through native tools | Literal-secret scanner plus URL marker guard (Sections 5.2 and 8) |
| Tampered agent manifest | Ed25519 signing (Section 6) |
| SSRF to cloud metadata | Private IP + hostname blocking + DNS check (Section 7) |
| API key recovery from memory dump | Zeroizing<String> (Section 8) |
| Unauthorized peer-to-peer connections | HMAC-SHA256 mutual auth (Section 9) |
| XSS / clickjacking on API | Security headers (Section 10) |
| API brute force / DoS | GCRA rate limiter (Section 11) |
| Path traversal via `../` | safe_resolve_path / safe_resolve_parent (Section 12) |
| Secret leakage to child processes | env_clear() + allowlist (Section 13) |
| Agent manifest broadens daemon execution authority | Symmetric policy intersection plus non-bypassable deployment profile (Section 13.4) |
| Untrusted code reaches host execution | `untrusted_execution` denies host starts; Docker/WASM require explicit invocation with no host fallback (Section 13.5) |
| Untrusted skill source | Remote marketplace frozen; complete local source review plus manifest policy and advisory phrase findings (Section 14) |
| Agent stuck in tool loop | LoopGuard with graduated response (Section 15) |
| Corrupted LLM session history | Session repair (Section 16) |
| Information leakage from health endpoint | Redacted public endpoint (Section 17) |
| Timing attacks on HMAC verification | subtle::ConstantTimeEq (Section 9.2) |
| Shell injection via metacharacters | Command::new (no shell) + env_clear (Section 13.4) |
| DNS rebinding for SSRF bypass | Resolved IP check, not hostname check (Section 7.3) |
