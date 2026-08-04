# Security Audit Reconciliation - 2026-07-29

## Scope

This evidence reconciles an external static audit of the public alpha.9
snapshot with the current Captain source tree.

- External snapshot: `16ba9bf8535583a392061b2cb5f93d4c97a3eea7`
- Current reconciliation baseline:
  `c45350ee759abd5b2a0c6c318f98e3d9584bffe0`
- The external snapshot object is not present in the local Git object database,
  so findings were relocated by symbol and behavior rather than by stale line
  numbers.
- The private audit body is not copied into this repository.
- This document records the closed remediation and the observed public release
  evidence. It does not strengthen any guarantee beyond the tests and
  publication facts listed here.

The external review was static only. Captain's current baseline was therefore
verified locally before reconciliation:

```text
scripts/gate.sh --clippy-workspace --test-workspace \
  --script-check scripts/gate.sh
```

The gate passed formatting, workspace Clippy with `-D warnings`, the complete
workspace test suite, Bash syntax, and staged/unstaged diff checks. Tests that
bind ephemeral loopback ports require execution outside the Codex filesystem
and network sandbox; the same gate passed there without external network
traffic.

Miri is not published for the installed stable ARM64 macOS toolchain, so it
cannot provide evidence for F1 on this host. The replacement evidence is a
release-profile persistence-failure regression plus a real API concurrency
regression that overlaps readers with serialized writes and compares the live
snapshot with the durable TOML result. This is an explicit platform limit, not
a claim that ordinary release tests are equivalent to Miri.

## Finding Matrix

| ID | Current status | Current evidence | Required action |
|---|---|---|---|
| F1 | Remediated | Global budget state now uses a coherent `RwLock<BudgetConfig>` snapshot. Updates are serialized, validate finite bounded values, atomically persist the complete candidate, and publish only after success. Both turn paths enforce the live global guard; `captain-api` denies `invalid_reference_casting`. | Keep the focused API concurrency and persistence-failure tests in the local release gate. |
| F2 | Remediated (`f0d3826`) | Nine agent-controlled subprocess surfaces now share `captain-runtime::guarded_exec`: policy review, content-bound permits, scrubbed explicit environments, workspace/time/output bounds, process-tree cleanup, and command-free audit events. A mandatory source audit rejects raw shell constructors and environment mutation in those sinks. | Retain the focused per-sink release tests and `scripts/guarded-exec-audit.sh` in tranche and release gates. |
| F3 | Remediated (T6 + T7) | Session signing uses a unique CSPRNG 32-byte per-install key and managed epoch, never API/password material. Passwords use salted Argon2id PHC strings; successful legacy SHA-256 login atomically migrates before session issuance. Dedicated bounded login state applies per-IP and per-username exponential backoff after five failures. Cookie Secure policy is explicit and proxy-aware. Browser WebSocket/SSE uses 30-second path/IP/epoch-bound single-use tickets; query-string credentials are rejected. Release and real-HTTP tests cover key uniqueness, forgery rejection, epoch invalidation, non-disclosure, migration, salt uniqueness, sixth-attempt `429`, `?token=` rejection, and ticket replay with a still-valid cookie. | Retain T6/T7 focused release tests and keep browser transports on the ticket endpoint. |
| F4 | Remediated (T3) | `middleware::PUBLIC_ALLOWLIST` is the sole global-auth bypass. Operational reads, Config/Terminal, A2A, and provider OAuth are private. The exact UUID-shaped agent ingress keeps its own Bearer auth, and Control consumes protected `401` responses centrally. Release-profile matrix and real HTTP tests cover both credential states and the two-field public health response. | Retain the explicit allowlist matrix, HTTP regressions, ingress matcher tests, and Control web audit. |
| F5 | Remediated (T4) | One boot-time request-origin policy now uses exact loopback, `deployment.public_url`, and `[api].allowed_origins` entries with explicit methods/headers, independently of API-key presence. An outer exact `Host` middleware rejects missing, ambiguous, malformed, or undeclared hosts before routing. Focused Tower-layer tests cover hostile and loopback origins, configured reverse-proxy hosts, preflight allow/deny behavior, and `400` DNS-rebinding rejection. | Retain the focused Types/Kernel/API regressions and keep origin-policy changes restart-bound. |
| F6 | Remediated (T5) | The audit trail is now an honestly named, versioned SHA-256 hash chain. Version 2 uses `u64` big-endian length prefixes, `record` propagates persistence failure before advancing memory, unknown actions retain their raw names, and schema v36 adds immutable recovery epochs without rehashing legacy rows. Startup seals altered history as invalid, opens a transactionally anchored `ChainRecovery` epoch, and exposes degradation through authenticated health, metrics, CLI, and TUI. The HTTP repair handler and route are gone. | Retain the field-boundary, failed-write, restart/tamper, unknown-action, migration, health-detail, and absent-route regressions in the local release gate. |
| F7 | Remediated (T8) | New installations use `Full`/`Safe`; `Open` remains an explicit approval opt-in. Critical-command recognition normalizes case, whitespace, short/long and reordered flags, common wrappers, and nested shell payloads. CLI, TUI, health, status, and Security APIs expose `host_process`, `environment_scrub`, and `os_isolation: false`. Docker/WASM remain separate explicit isolation backends. | Retain focused policy, bypass-normalization, status-surface, installation-config, and DOC2 regressions in the local release gate. |
| F8 | Remediated (T9) | Remote marketplace HTTP routes and TUI actions are absent, the CLI accepts only an existing local directory, and both retained compatibility clients return `RemoteMarketplaceFrozen` before network or filesystem mutation. Publisher-backed integrity remains a prerequisite for reopening. | Retain the typed fail-before-I/O tests, absent-route regression, local-only CLI/TUI tests, and DOC2 source locks. |
| F9 | Remediated (T9) | Prompt-text review returns a report with `advisory_heuristic` assurance. Messages, doctor output, architecture, security guidance, and agent-facing docs state that matches are not proof and an empty report is not proof of safety. The loader conservatively refuses high-risk matches as policy; no marketplace confirmation UX was added. | Retain the exact assurance, conservative loader, doctor, and documentation regressions. |
| F10 | Remediated (`e1d43bf`) | Temporary tokens are scoped by canonical skill path, source digest, and token name, expire after 30 minutes, and are zeroized. Cross-skill non-disclosure and source-change invalidation are covered by Runtime tests. | Retain the source-bound cache-key and expiry regressions. |
| F11 | Remediated (`69e9003`) | `web_fetch`, `web_search`, and `web_research_batch` now fail closed without the Kernel-owned protected `WebToolsContext`; the raw fallback clients are gone. | Keep all three missing-context regressions. |
| F12 | Remediated honestly (`48855ea`) | The unused local `taint` labels were removed. Code, API, CLI, and docs now describe heuristic content guards and explicitly report `provenance_tracking: false`. | Treat real typed provenance propagation as a separate design project. |
| F13 | Remediated (`fc38ff7`) | CapSpec Telegram decision telemetry records decision kind and bounded transport error only; the opaque operator token is absent from success and failure logs. | Keep the source-bound non-disclosure regression. |

## HARDEN11 follow-up — 2026-07-30

This addendum supersedes the **current** F7 execution posture without rewriting
the Alpha 10 evidence above.

- `ExecSecurityMode::default()` is now `Allowlist`.
- `personal_workstation`, `remote_operator`, and `untrusted_execution` are
  typed deployment profiles. Remote operation has an allowlist floor;
  untrusted execution denies agent-controlled host process starts.
- The daemon policy and per-agent policy are intersected before tool discovery
  and dispatch. Agent manifests cannot broaden profile, mode, command lists,
  blocklists, time/output limits, or critical mode.
- `process_start` now requires a content-bound exact-program `ExecPermit`; the
  guarded-exec source audit covers eleven controlled process sinks.
- Docker and WASM are explicit rails. Captain performs no automatic routing
  and no host fallback. An enabled Docker rail under `untrusted_execution`
  must use network `none`, a read-only root and workspace, dropped
  capabilities, and finite CPU, memory, and PID limits.
- CLI Status/Security, full Doctor, Ratatui, Control, `/api/status`,
  `/api/health/detail`, and `/api/security` expose profile, configured and
  effective mode, host permission, explicit routing, and Docker readiness.

F8 and the dependency audit require no reopened implementation: remote
marketplace operations remain structurally frozen before network/filesystem
I/O, and release readiness already runs both the configured and unfiltered
RustSec audits against an exact reviewed baseline.

## Closure Evidence

The complete release-readiness gate passed from clean source before packaging:
RustSec on 1,096 dependencies, public export and Gitleaks, 649 DOC2 controls,
107 release-document controls, 126 workflow controls, the Runtime, Kernel,
API, and CLI suites, and the 20-check isolated daemon smoke.

The five host bundles were then built in five separate processes, one target
at a time, with an independent checksum, platform manifest, disk checkpoint,
and load checkpoint after every target. The deterministic provenance statement
binds its 20 subjects to public source commit
`48f898a9e4d38e8b8c7627644b66e22076a39364`, tree
`ba37632b5e2b6a3923a2241e1d38d7903bdb95f1`, and the exact `Cargo.lock`.
Together with the statement checksum, the GitHub Release contains exactly 22
assets whose GitHub-reported SHA-256 digests match the local files.

Docker AMD64 completed, pushed, and was remotely inspected before ARM64
started. Both architecture images include BuildKit provenance. The immutable
version and moving `alpha` channel resolve to OCI index
`sha256:c54d1319b5173ca55540dc69e0f965a31b51cdfccb497ca77882882a16b4e477`;
anonymous execution returned `captain 0.1.0-alpha.10` on both architectures.
The annotated tag object `b58f7561d0014228cc523b1770b5c411b017ef52`
dereferences to the source commit above. The GitHub Actions API returned zero
runs.

The host provenance statement is checksum-bound but not independently signed,
and guarded host execution is not OS isolation. Those disclosed limits remain
unchanged by closure.

## Additional Confirmed Gap

The per-turn memory opt-out gap is remediated by `c0a9e4c`: the shared
successful-turn finalizer gates episodic embedding and semantic fragment
storage before either streaming or non-streaming completion. The resumable
transcript and mandatory operational/audit records remain intentionally
retained.

Loading `secrets.env` into the long-lived daemon environment also increases the
blast radius of every missed process boundary. The execution remediation must
make child environment construction explicit; it must not rely on future
callers remembering to remove inherited values.

## Accepted Existing Controls

The following controls were rechecked conceptually and remain outside
remediation unless a targeted test disproves them:

- canonical workspace and symlink confinement;
- redirect-by-redirect SSRF checks in the primary fetch engine;
- deny-by-default inbound channel allowlists;
- authenticated peer messages with replay protection;
- zip path confinement;
- unpredictable external-content delimiters;
- parameterized SQL values;
- TLS verification;
- private permissions for secrets and durable sensitive state;
- CapSpec durable checkpoints, uncertain-effect handling, and exclusive
  recovery.

## Delivery Order

Each numbered tranche below must be an atomic commit with its own
`scripts/gate.sh` invocation. A tranche is not complete when only its source
compiles.

1. Remove immediate leaks and unsafe fallbacks: F13, F11, F10, honest F12
   terminology, and the memory opt-out gap.
2. Replace the unsafe budget mutation and certify concurrent reads/writes.
3. Add independent session signing state and session epochs.
4. Migrate passwords and login controls after the session-secret contract is
   live.
5. Close operational read APIs, then make CORS and Host policy fail closed
   across API, Control, terminal Web, and Desktop.
6. Replace repairable audit claims with a durable append-only epoch model.
7. Unify process execution and apply safer default execution policy without
   claiming unavailable OS isolation.
8. Freeze the unverified remote marketplace and reclassify its scanner.
9. Run local dependency, public-boundary, documentation, install, migration,
   browser, release-readiness, and bundle audits.

## Local-Only CI Decision

Automatic GitHub `push`, `pull_request`, or scheduled workflows are not enabled
as part of this remediation. The operator explicitly requires local builds and
audits to avoid hosted CI billing. Existing manual workflows remain available,
but release evidence is produced by versioned local scripts.

This is an operational adaptation of the external CI recommendation, not a
waiver of its checks. Formatting, strict Clippy, workspace tests, dependency
audit, public-boundary checks, install smoke, and bundle verification must all
run locally before publication.

## Exit Criteria

This reconciliation is closed because:

- every confirmed finding above has a linked atomic commit and focused
  regression proof;
- the complete local strict gate passes from a clean worktree;
- dependency and secret audits have recorded outcomes;
- docs describe implemented guarantees and known limits without security
  shorthand that implies stronger isolation or provenance;
- all release bundles are built and checksummed locally;
- no automatic GitHub workflow was used to produce those artifacts.
