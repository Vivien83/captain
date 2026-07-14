# Captain Architecture

This document describes the internal architecture of Captain, the open-source Agent Operating System built in Rust. It covers the crate structure, kernel boot sequence, agent lifecycle, memory substrate, LLM driver abstraction, capability-based security model, the OFP wire protocol, the security hardening stack, the channel and skill systems, and the agent stability subsystems.

> DOC2 status: current architecture reference. Avoid treating historical
> roadmap counts as runtime truth; use `captain status`, `captain models
> providers`, `captain models list`, and the release gates for live validation.

## Table of Contents

- [Crate Structure](#crate-structure)
- [Kernel Boot Sequence](#kernel-boot-sequence)
- [Agent Lifecycle](#agent-lifecycle)
- [Agent Loop Stability](#agent-loop-stability)
- [Memory Substrate](#memory-substrate)
- [LLM Driver Abstraction](#llm-driver-abstraction)
- [Model Catalog](#model-catalog)
- [Capability-Based Security Model](#capability-based-security-model)
- [Security Hardening](#security-hardening)
- [Channel System](#channel-system)
- [Skill System](#skill-system)
- [MCP and A2A Protocols](#mcp-and-a2a-protocols)
- [Wire Protocol (OFP)](#wire-protocol-ofp)
- [Operator Surfaces](#operator-surfaces)
- [Subsystem Diagram](#subsystem-diagram)

---

## Crate Structure

Captain is organized as a Cargo workspace. Dependencies flow downward (lower crates depend on nothing above them).

```
operator surfaces
  captain-cli          CLI/TUI, daemon client, MCP server
  captain-api          authenticated Control web, REST/WS/SSE, agent API
  captain-desktop      retained Tauri wrapper (frozen release surface)
        |
  captain-kernel       lifecycle, orchestration, recovery, budgets, automation
        |
  +-- captain-runtime      agent loop, LLM drivers, tools, checkpoints, MCP/A2A
  +-- captain-memory       SQLite state, sessions, detached runs, semantic memory
  +-- captain-channels     active channels plus frozen compatibility adapters
  +-- captain-skills       governed skills plus frozen format compatibility
  +-- captain-wire         OFP compatibility transport
  +-- captain-graph        graph projections and relationships
  +-- captain-hands        retained/frozen curated packages
  +-- captain-extensions   retained/frozen integration registry
        |
  captain-types        shared contracts and configuration types
```

### Crate Responsibilities

| Crate | Description |
|-------|-------------|
| **captain-types** | Shared contracts for agents, capabilities, events, tools, configuration, taint/signing, model catalog, MCP/A2A compatibility, web, and agent-as-service. Config structs use `#[serde(default)]` for forward-compatible TOML parsing. |
| **captain-memory** | SQLite-backed durable substrate for KV memory, semantic search, graph entities/relations, sessions, tasks, usage, canonical cross-channel context, projects, and detached tool-run history. Ordered migrations upgrade existing databases to the current schema at boot. |
| **captain-runtime** | Agent execution engine for streaming/non-streaming LLM turns, fail-closed native tool parallelism, durable tool-use boundaries, loop guards, session repair, compaction, detached tool supervision, built-in tools, WASM, MCP/A2A compatibility, web access, audit, and embeddings. Defines `KernelHandle` to avoid circular dependencies. |
| **captain-kernel** | Central coordinator for registry, scheduler, capabilities, event bus, supervisor, workflows/triggers/crons, metering/budgets, model routing, authentication, background work, project/goals, agent API provisioning, checkpoint recovery, and graceful shutdown. Boot reconciles interrupted detached work and restores persistent agents/sessions. |
| **captain-api** | Axum REST/WS/SSE server, agent-as-service ingress/egress, OpenAI-compatible API, authenticated six-hub Control web app, health/status, and frozen compatibility routes. Middleware covers authentication, request ids/logging, rate limiting, security headers, and health redaction. |
| **captain-channels** | Channel bridge layer. The active Hermes-level user path is Telegram, Discord, Signal, and Email; long-tail adapters remain compatibility/frozen unless explicitly reopened. Each adapter implements the `ChannelAdapter` trait. Features: `AgentRouter` for message routing, `BridgeManager` for lifecycle coordination, `ChannelRateLimiter` (per-user DashMap tracking), `formatter.rs` (Markdown to TelegramHTML/SlackMrkdwn/PlainText), `ChannelOverrides` (model/system_prompt/dm_policy/group_policy/rate_limit/threading/output_format), DM/group policy enforcement. |
| **captain-wire** | Captain Protocol (OFP) for peer-to-peer agent communication. JSON-framed messages over TCP with HMAC-SHA256 mutual authentication (nonce + constant-time verify via `subtle`). `PeerNode` listens for connections and manages peers. `PeerRegistry` tracks known remote peers and their agents. |
| **captain-cli** | Clap CLI and six-hub Ratatui desktop experience. It discovers the daemon through `~/.captain/daemon.json`, uses HTTP when available, and supports an in-process fallback plus MCP server mode. |
| **captain-desktop** | Retained Tauri wrapper. Its implementation remains buildable for compatibility, but Tauri packaging and promotion are frozen; it is not the active release path. |
| **captain-hands** | Retained curated capability packages. Hands are frozen/hidden from the primary product navigation. |
| **captain-extensions** | Retained integration registry and credential compatibility. Experimental integrations remain frozen. |
| **captain-graph** | Graph projection and relationship support for runtime knowledge paths. |
| **captain-skills** | Skill system for pluggable tool bundles. Bundled skills are compiled via `include_str!()`, while installed and generated skills live on disk. Skills are `skill.toml` + Python/WASM/Node.js/PromptOnly code. `SkillManifest` defines metadata, runtime config, provided tools, and requirements. `SkillRegistry` manages installed and bundled skills. Marketplace/ClawHub code paths are compatibility/frozen outside the active core release path. `SKILL.md` parser supports OpenClaw compatibility (YAML frontmatter + Markdown body). `SkillVerifier` provides SHA256 verification. Prompt injection scanner (`scan_prompt_content()`) detects override attempts, data exfiltration, and shell references. |
| **xtask** | Build automation tasks (cargo-xtask pattern). |

Daemon shutdown closes the API listener and gives long-lived HTTP connections
up to 15 seconds to drain before forcing the server future closed. It then takes
the channel bridge out of its shared mutex and gives adapters a separate 15-second
drain period before abandoning residual pollers. The kernel finally
persists suspended agent state and returns to the service manager, so a stuck
browser, SSE stream, or external channel cannot block restart.

---

## Kernel Boot Sequence

When `CaptainKernel::boot_with_config()` is called (either by the daemon or in-process by the CLI/desktop app), the following sequence executes:

```
1. Load configuration
   - Read ~/.captain/config.toml (or specified path)
   - Apply #[serde(default)] defaults for missing fields
   - Validate config and log warnings (missing API keys, etc.)

2. Create data directory
   - Ensure ~/.captain/data/ exists

3. Initialize memory substrate
   - Open SQLite database (captain.db)
   - Run schema migrations (up to v5)
   - Set memory decay rate

4. Initialize LLM driver
   - Read API key from environment variable
   - Create driver for the configured provider
   - Validate driver config

5. Initialize model catalog
   - Build `ModelCatalog` from the runtime provider/model/alias definitions
   - Run detect_auth() to check env var presence (never reads secrets)
   - Store as kernel.model_catalog

6. Initialize metering engine
   - Create MeteringEngine with cost catalog (20+ model families)
   - Wire to model catalog for pricing source

7. Initialize model router
   - Create ModelRouter with TaskComplexity scoring
   - Validate configured models and resolve aliases

8. Initialize core subsystems
   - AgentRegistry (DashMap-based concurrent agent store)
   - CapabilityManager (DashMap-based capability grants)
   - EventBus (async broadcast channel)
   - AgentScheduler (quota tracking per agent, hourly window reset)
   - Supervisor (health monitoring, panic/restart counters)
   - WorkflowEngine (workflow registration and execution, run eviction cap 200)
   - TriggerEngine (event pattern matching)
   - BackgroundExecutor (continuous/periodic agent loops)
   - WasmSandbox (Wasmtime engine, dual fuel+epoch metering)

9. Initialize RBAC auth manager
   - Create AuthManager with UserRole hierarchy
   - Set up channel identity resolution

10. Initialize skill registry
    - Load bundled skills via parse_bundled()
    - Load user-installed skills from disk
    - Wire skill tools into tool_runner fallback chain
    - Inject PromptOnly skill context into system prompts

11. Initialize web tools context
    - Create WebSearchEngine (4-provider cascading: Tavily->Brave->Perplexity->DDG)
    - Create WebFetchEngine (SSRF-protected)
    - Bundle as WebToolsContext

12. Restore persisted agents
    - Load all agents from SQLite
    - Re-register in memory (registry, capabilities, scheduler)
    - Set state to Running

13. Publish KernelStarted event

14. Return kernel instance
```

When the daemon wraps the kernel in `Arc`, additional steps occur:

```
15. Set self-handle (weak Arc reference for trigger dispatch)

16. Connect to MCP servers
    - Background connect to configured MCP servers (stdio/SSE)
    - Namespace tools as mcp_{server}_{tool}
    - Store connections in kernel.mcp_connections

17. Start heartbeat monitor
    - Background tokio task for agent health checks
    - Publishes HealthCheckFailed events on anomalies

18. Start background agent loops (continuous, periodic, proactive)
```

---

## Agent Lifecycle

### States

```
    spawn                    message/tick              kill
      |                         |                       |
      v                         v                       v
  [Running] <------------> [Running] ---------> [Terminated]
      |                                              ^
      |          shutdown                            |
      +----------> [Suspended] ---------------------+
                      |          reboot/restore
                      +------> [Running]
```

- **Running**: Agent is active and can receive messages.
- **Suspended**: Agent is paused (e.g., during daemon shutdown). Persisted to SQLite for restore on next boot.
- **Terminated**: Agent has been killed. Removed from registry and persistent storage.

### Spawn Flow

1. Generate new `AgentId` (UUID v4) and `SessionId`.
2. Create a session in the memory substrate.
3. Parse the manifest and extract capabilities.
4. Validate capability inheritance (`validate_capability_inheritance()` prevents privilege escalation).
5. Grant capabilities via `CapabilityManager`.
6. Register with the `AgentScheduler` (quota tracking).
7. Create `AgentEntry` and register in `AgentRegistry`.
8. Persist to SQLite via `memory.save_agent()`.
9. If agent has a parent, update parent's children list.
10. Register proactive triggers (if schedule mode is `Proactive`).
11. Publish `Lifecycle::Spawned` event and evaluate triggers.

### Message Flow

1. **RBAC check**: `AuthManager` resolves channel identity and checks user role permissions.
2. **Channel policy check**: `ChannelBridgeHandle.authorize_channel_user()` enforces DM/group policy.
3. **Quota check**: `AgentScheduler` verifies the agent has not exceeded its token-per-hour limit.
4. **Entry lookup**: Fetch `AgentEntry` from the registry.
5. **Module dispatch**: Based on `manifest.module`:
   - `builtin:chat` or unrecognized: LLM agent loop
   - `wasm:path/to/module.wasm`: WASM sandbox execution
   - `python:path/to/script.py`: Python subprocess execution (env_clear() + selective vars)
6. **LLM agent loop** (for `builtin:chat`):
   a. Load or create session from memory.
   b. Load canonical context summary (cross-channel memory) into system prompt.
   c. Append stability guidelines to system prompt.
   d. Resolve LLM driver (per-agent override or kernel default).
   e. Gather available tools (filtered by capabilities + skill tools + MCP tools).
   f. Initialize loop guard (tool loop detection).
   g. Run session repair (validate and fix message history).
   h. Run iterative loop: send messages to LLM, execute tool calls, accumulate results.
   i. Auto-compact session if threshold exceeded (block-aware compaction).
   j. Save updated session and canonical session back to memory.
7. **Cost estimation**: `MeteringEngine.estimate_cost_with_catalog()` computes cost in USD.
8. **Record usage**: Update quota tracking with token counts; persist usage event.
9. **Return result**: `AgentLoopResult` with response text, token usage, iteration count, and `cost_usd`.

### Kill Flow

1. Check caller has `AgentKill(target_name)` capability.
2. Remove from `AgentRegistry`.
3. Stop background loops via `BackgroundExecutor`.
4. Unregister from `AgentScheduler`.
5. Revoke all capabilities.
6. Unsubscribe from `EventBus`.
7. Remove triggers.
8. Remove from persistent storage (SQLite).

---

## Agent Loop Stability

The agent loop includes multiple hardening layers to prevent runaway behavior:

### Loop Guard

`LoopGuard` detects when an agent is stuck calling the same tool with the same parameters. Uses SHA256 hashing of `(tool_name, params)` to identify repetition.

- **Warn threshold** (default 3): Logs a warning and injects a hint to the LLM.
- **Block threshold** (default 5): Refuses the tool call and returns an error to the LLM.
- **Circuit breaker** (default 30): Terminates the agent loop entirely.

Configured via `LoopGuardConfig`.

### Session Repair

`validate_and_repair()` runs before each agent loop iteration to ensure message history consistency:

- Drops orphaned `ToolResult` messages (no matching `ToolUse`).
- Removes empty messages.
- Merges consecutive same-role messages.

### Tool Result Truncation

`truncate_tool_result()` enforces a 50,000 character hard cap on tool output. Truncated results include a marker showing the original size.

### Tool Execution and Recovery

Tool limits are tool-specific. One-shot shell/SSH/code paths use bounded review
windows and hard caps; intentional servers/watchers use supervised
`process_start`; eligible long diagnostics use detached `tool_run_start` and
can be revisited after later turns. Before PRE hooks or execution, the runtime
persists the assistant `ToolUse` turn and a session-scoped checkpoint. Recovery
never assumes that an unknown side effect failed and never injects one
session's checkpoint into another session.

Native parallel execution is fail-closed. Only explicitly classified
independent read-only calls can overlap during their EXEC phase; PRE and POST
remain ordered. Unknown, MCP, skill, custom, dependent, overlapping-path, and
side-effecting calls remain sequential.

### Max Continuations

`MAX_CONTINUATIONS = 3` prevents infinite "Please continue" loops. After 3 continuation attempts, the agent returns its partial response rather than requesting another round.

### Inter-Agent Depth Limit

`MAX_AGENT_CALL_DEPTH = 5` enforced via `tokio::task_local!` in the tool runner. Prevents unbounded recursive agent-to-agent calls.

### Stability Guidelines

`STABILITY_GUIDELINES` are appended to every agent's system prompt. These contain anti-loop and anti-retry behavioral rules that the LLM follows to avoid degenerate patterns.

### Block-Aware Compaction

The session compactor handles all content block types (Text, ToolUse, ToolResult, Image) rather than assuming text-only messages. Auto-compaction triggers when the session exceeds the configured threshold (default 80% of context window), keeping the most recent messages (default 20).

---

## Memory Substrate

The memory substrate (`captain-memory`) provides six layers of storage:

### 1. Structured KV Store

Per-agent key-value storage backed by SQLite. Keys are strings, values are JSON. Used by the `memory_store` and `memory_recall` tools.

A shared memory namespace (fixed agent ID `00000000-...01`) enables cross-agent data sharing.

```
agent_id | key         | value
---------|-------------|------------------
uuid-a   | preferences | {"theme": "dark"}
uuid-b   | state       | {"step": 3}
shared   | project     | {"name": "foo"}
```

### 2. Semantic Search

Vector embeddings for similarity-based memory retrieval. Documents are embedded using the configured embedding driver and stored with their vectors. Queries are embedded at search time and matched by cosine similarity.

### 3. Knowledge Graph

Entity-relation storage for structured knowledge. Agents can store entities (with types and properties) and relations between them. Supports graph traversal queries.

### 4. Session Manager

Conversation history storage. Each agent has a session containing its message history (user, assistant, tool use, tool result, image). Sessions track context window token counts. Sessions are persisted to SQLite and restored on kernel reboot.

### 5. Task Board

A shared task queue for multi-agent collaboration:
- `task_post`: Create a task with title, description, and optional assignee.
- `task_claim`: Claim the next available task.
- `task_complete`: Mark a task as done with a result.
- `task_list`: List tasks filtered by status (pending, claimed, completed).

### 6. Usage and Canonical Sessions

- **Usage tracking**: `usage_events` table persists token counts, cost estimates, and model usage per agent. `UsageStore` provides query and aggregation APIs.
- **Persisted conversation sessions**: Each row has a stable UUID, owning
  agent, transcript, context budget, timestamps, and optional label. A client
  can target that UUID for a single turn without mutating the agent registry's
  default session. Session reset creates a new default but preserves the prior
  row; only explicit history deletion removes reopenable conversations.
- **Automatic labels**: Unlabelled sessions derive a bounded title from the
  first meaningful user message. Explicit labels are preserved, including when
  an older in-memory session snapshot is saved later.
- **One cross-surface catalog**: Web Control, full and standalone TUI,
  line-based CLI and Desktop list and load the same SQLite rows. A restored row
  carries both its session UUID and owning agent, so continuation never depends
  on which surface originally created it. The boot migration imports legacy
  TUI JSON mirrors with deterministic UUID v5 identifiers and never overwrites
  a row already continued elsewhere. A sidecar `.json.imported` marker makes
  every successful migration one-shot while leaving failed imports retryable.
- **Canonical sessions**: Cross-channel memory. `CanonicalSession` tracks a user's conversation context across multiple channels. Compaction produces summaries that are injected into system prompts and stored in `canonical_sessions`.

### SQLite Architecture

All memory operations go through `Arc<Mutex<Connection>>` with Tokio's
`spawn_blocking` for async bridging. Ordered schema migrations run
automatically at boot. They cover current durable contracts such as canonical
sessions, projects, checkpoints, and detached tool runs; operators should not
couple integrations to a hard-coded schema version.

---

## LLM Driver Abstraction

The `LlmDriver` trait (`captain-runtime`) provides a unified interface for all LLM providers:

```rust
#[async_trait]
pub trait LlmDriver: Send + Sync {
    async fn send_message(
        &self,
        model: &str,
        system_prompt: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse, CaptainError>;

    async fn send_message_streaming(
        &self,
        model: &str,
        system_prompt: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<LlmResponse, CaptainError>;

    fn key_required(&self) -> bool;
}
```

### Provider Architecture

Three driver families cover the runtime model catalog:

1. **AnthropicDriver**: Native Anthropic Messages API. Handles Claude-specific features (content blocks including images, tool use blocks, streaming deltas). Supports `ContentBlock::Image` with media type validation and 5MB cap.

2. **GeminiDriver**: Native Google Gemini API (v1beta). Uses `x-goog-api-key` auth, `systemInstruction`, `functionDeclarations`, `streamGenerateContent?alt=sse`. Maps Gemini function call responses to the unified `ToolUse` stop reason.

3. **OpenAiCompatDriver**: OpenAI-compatible Chat Completions API. Works with any provider that implements the OpenAI API format. Configured with different base URLs per provider. Covers OpenAI-compatible hosted providers and local runners.

### Provider Configuration

| Provider | Driver | Base URL | Key Required |
|----------|--------|----------|--------------|
| `anthropic` | Anthropic | `https://api.anthropic.com` | Yes |
| `gemini` | Gemini | `https://generativelanguage.googleapis.com` | Yes |
| `openai` | OpenAI-compat | `https://api.openai.com` | Yes |
| `deepseek` | OpenAI-compat | `https://api.deepseek.com` | Yes |
| `groq` | OpenAI-compat | `https://api.groq.com/openai` | Yes |
| `openrouter` | OpenAI-compat | `https://openrouter.ai/api` | Yes |
| `mistral` | OpenAI-compat | `https://api.mistral.ai` | Yes |
| `together` | OpenAI-compat | `https://api.together.xyz` | Yes |
| `fireworks` | OpenAI-compat | `https://api.fireworks.ai/inference` | Yes |
| `perplexity` | OpenAI-compat | `https://api.perplexity.ai` | Yes |
| `cohere` | OpenAI-compat | `https://api.cohere.ai` | Yes |
| `ai21` | OpenAI-compat | `https://api.ai21.com` | Yes |
| `cerebras` | OpenAI-compat | `https://api.cerebras.ai` | Yes |
| `sambanova` | OpenAI-compat | `https://api.sambanova.ai` | Yes |
| `huggingface` | OpenAI-compat | `https://api-inference.huggingface.co` | Yes |
| `xai` | OpenAI-compat | `https://api.x.ai` | Yes |
| `replicate` | OpenAI-compat | `https://api.replicate.com` | Yes |
| `ollama` | OpenAI-compat | `http://localhost:11434` | No |
| `vllm` | OpenAI-compat | `http://localhost:8000` | No |
| `lmstudio` | OpenAI-compat | `http://localhost:1234` | No |

### Per-Agent Driver Resolution

Each agent can override the kernel's default provider:

```toml
[model]
provider = "openai"                   # Different from kernel default
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"        # Custom key env var
base_url = "https://custom.api.com"   # Optional custom endpoint
```

When resolving the driver for an agent:
1. If the agent uses the same provider as the kernel default (and no custom key/URL), reuse the kernel's shared driver instance.
2. Otherwise, create a dedicated driver for that agent.

### Retry and Rate Limiting

LLM calls use exponential backoff for rate-limited (429) and overloaded (529) responses. The retry logic is built into the driver layer. All API key fields use `Zeroizing<String>` for automatic memory wipe on drop.

---

## Model Catalog

The `ModelCatalog` (`captain-runtime/src/model_catalog.rs`) provides a registry of all known models, providers, and aliases.

### Registry Contents

- Built-in and runtime-discovered models across hosted and local providers.
- Aliases for convenience, exposed by `captain models aliases` and `/api/models/aliases`.
- Providers with authentication status detection, exposed by `captain models providers` and `/api/providers`.

### Types

- `ModelCatalogEntry`: Model ID, display name, provider, tier, context window, cost rates.
- `ProviderInfo`: Provider name, driver type, base URL, key env var, auth status.
- `ModelTier`: Frontier, Smart, Balanced, Fast (maps to cost and capability tiers).
- `AuthStatus`: Detected, NotDetected (based on env var presence without reading secrets).

### Integration Points

- **Metering**: `estimate_cost_with_catalog()` uses catalog entries as the pricing source.
- **Router**: `ModelRouter.validate_models()` and `resolve_aliases()` reference the catalog.
- **API**: 4 endpoints (`/api/models`, `/api/models/{id}`, `/api/models/aliases`, `/api/providers`).
- **Channels**: `/models` and `/providers` chat commands via `ChannelBridgeHandle`.

---

## Capability-Based Security Model

Every agent operation is subject to capability checks. Capabilities are declared in the agent manifest and enforced at runtime.

### Capability Types

```rust
pub enum Capability {
    // Tool access
    ToolInvoke(String),       // Access to a specific tool (e.g., "file_read")
    ToolAll,                  // Access to all tools

    // Memory access
    MemoryRead(String),       // Read scope (e.g., "*", "self.*")
    MemoryWrite(String),      // Write scope

    // Network access
    NetConnect(String),       // Connect to host (e.g., "api.example.com", "*")

    // Agent operations
    AgentSpawn,               // Can spawn new agents
    AgentMessage(String),     // Can message agents matching pattern
    AgentKill(String),        // Can kill agents matching pattern

    // Shell access
    ShellExec(String),        // Can execute shell commands matching pattern

    // OFP networking
    OfpDiscover,              // Can discover remote peers
    OfpConnect(String),       // Can connect to specific peers
    OfpAdvertise,             // Can advertise to peers
}
```

### Capability Inheritance Validation

`validate_capability_inheritance()` prevents privilege escalation when agents spawn child agents. A child agent can never receive capabilities that its parent does not hold. This is enforced at spawn time before any capabilities are granted.

### Manifest Declaration

```toml
[capabilities]
tools = ["file_read", "file_list", "web_fetch"]
memory_read = ["*"]
memory_write = ["self.*"]
network = ["api.anthropic.com"]
shell = []
agent_spawn = false
agent_message = ["coder", "researcher"]
agent_kill = []
ofp_discover = false
ofp_connect = []
```

### Enforcement Flow

```
Tool invocation request
    |
    v
CapabilityManager.check(agent_id, ToolInvoke("file_read"))
    |
    +-- Granted --> Validate path (traversal check) --> Execute tool
    |
    +-- Denied --> Return "Permission denied" error to LLM
```

The `CapabilityManager` uses a `DashMap<AgentId, Vec<Capability>>` for lock-free concurrent access. Capabilities are granted at spawn time (after inheritance validation) and revoked at kill time.

The tool runner also enforces capabilities by filtering the tool list before passing it to the LLM. If the LLM hallucinates a tool name outside the agent's granted list, the tool runner rejects it with a permission error.

---

## Security Hardening

Captain implements defense-in-depth security systems organized around critical runtime protections:

### Path Traversal Prevention

`safe_resolve_path()` and `safe_resolve_parent()` in WASM host functions prevent directory traversal attacks. Path validation in `tool_runner.rs` (`validate_path`) protects file tools. Capability check runs BEFORE path resolution (deny first, then validate).

### Subprocess Isolation

`subprocess_sandbox.rs` provides a secure execution environment for Python/Node skill runtimes. All subprocess invocations use `cmd.env_clear()` followed by selective environment variable injection, preventing secret leakage.

### SSRF Protection

`is_ssrf_target()` and `is_private_ip()` block requests to private IPs and cloud metadata endpoints (169.254.169.254, etc.). DNS resolution is checked to prevent DNS rebinding attacks. Applied in `host_net_fetch` and `web_fetch.rs`.

### WASM Dual Metering

WASM sandbox uses both Wasmtime fuel metering (instruction count) and epoch interruption (wall-clock timeout via watchdog thread). This prevents both CPU-bound and time-bound runaway modules.

### Merkle Audit Trail

`audit.rs` implements a Merkle hash chain where each audit entry includes a hash of the previous entry. This provides tamper-evident logging of all agent actions.

### Information Flow Taint Tracking

`taint.rs` in `captain-types` implements taint labels and taint sets. Data from external sources carries taint labels that propagate through operations, enabling information flow analysis.

### Ed25519 Manifest Signing

`manifest_signing.rs` provides Ed25519 digital signatures for agent manifests. Ensures manifest integrity and authenticity.

### OFP HMAC-SHA256 Mutual Auth

Wire protocol authentication uses `hmac_sign(secret, nonce + node_id)` on both handshake sides. Nonce prevents replay attacks. Constant-time verification via the `subtle` crate prevents timing attacks.

### Security Headers Middleware

CSP, X-Frame-Options, X-Content-Type-Options, X-XSS-Protection, Referrer-Policy, and Permissions-Policy headers on all API responses.

### GCRA Rate Limiter

Generic Cell Rate Algorithm with cost-aware token buckets. Per-IP tracking with stale entry cleanup. Configurable burst and sustained rates.

### Health Endpoint Redaction

Public health endpoint (`/api/health`) returns minimal status. Detailed health (`/api/health/detail`) requires authentication and shows database stats, agent counts, and subsystem status.

### Prompt Injection Scanner

`scan_prompt_content()` in the skills crate detects override attempts, data exfiltration patterns, and shell references in skill content. Applied to all bundled and installed skills and to SKILL.md auto-conversion.

### Secret Zeroization

All LLM driver API key fields use `Zeroizing<String>` from the `zeroize` crate. Keys are automatically wiped from memory when the driver is dropped. `Debug` impls on config structs redact secret fields.

### Localhost-Only Fallback

When no API key is configured, the system falls back to localhost-only mode, preventing accidental exposure of unauthenticated endpoints.

### Loop Guard and Session Repair

See [Agent Loop Stability](#agent-loop-stability) above.

### Security Dependencies

`sha2`, `hmac`, `hex`, `subtle`, `ed25519-dalek`, `rand`, `zeroize`, `governor`

---

## Channel System

The channel system (`captain-channels`) provides the messaging bridge for active
channels and frozen compatibility adapters.

### Adapter Status

| Status | Channels |
|--------|----------|
| Active Hermes-level UX | Telegram, Discord, Signal, Email |
| Frozen compatibility | Slack, WhatsApp, Matrix, Webhook, Teams, Mattermost, IRC, Google Chat, Twitch, Rocket.Chat, Zulip, XMPP, LINE, Viber, Messenger, Reddit, Mastodon, Bluesky, Feishu, Revolt, Nextcloud, Guilded, Keybase, Threema, Nostr, Webex, Pumble, Flock, Twist, Mumble, DingTalk, Discourse, Gitter, Ntfy, Gotify, LinkedIn, and any other non-core adapters |

Use `captain channel list`, `captain status`, and `/api/channels` to inspect
the runtime active/frozen state.

### Channel Features

- **Channel Overrides**: Per-channel configuration of model, system prompt, DM policy, group policy, rate limit, threading, and output format.
- **DM/Group Policy**: `DmPolicy` and `GroupPolicy` enums enforce who can interact with agents in direct messages vs. group chats.
- **Formatter**: `formatter.rs` converts Markdown to platform-specific formats (TelegramHTML, SlackMrkdwn, PlainText).
- **Rate Limiter**: `ChannelRateLimiter` with per-user DashMap tracking prevents message flooding.
- **Threading**: `send_in_thread()` trait method for platforms that support threaded conversations.
- **Chat Commands**: `/models`, `/providers`, `/new`, `/compact`, `/model`, `/stop`, `/usage`, `/think` handled by `ChannelBridgeHandle`.

---

## Skill System

The skill system (`captain-skills`) provides bundled, installed, and generated
skills and supports local/OpenClaw-compatible installation.

### Skill Types

- **Python**: Python scripts executed in subprocess sandbox.
- **Node.js**: Node.js scripts (OpenClaw compatibility).
- **WASM**: WebAssembly modules executed in the WASM sandbox.
- **PromptOnly**: Skills that inject context into the LLM system prompt without code execution.

### Bundled Skills

Compiled into the binary via `include_str!()` in `bundled.rs`. Bundled skill
inventory is code-defined and should be checked with `captain skill list`:

- **Tier 1 (8)**: github, docker, web-search, code-reviewer, sql-analyst, git-expert, sysadmin, writing-coach
- **Tier 2 (6)**: kubernetes, terraform, aws, jira, data-analyst, api-tester
- **Tier 3 (6)**: pdf-reader, slack-tools, notion, sentry, mongodb, regex-expert
- Additional bundled skills may be present depending on the runtime build.

### Security Pipeline

All skills pass through a security pipeline before activation:

1. **SHA256 verification** (`SkillVerifier`): Ensures skill content matches its declared hash.
2. **Prompt injection scan** (`scan_prompt_content()`): Detects malicious patterns in skill prompts and descriptions.
3. **Trust boundary markers**: Skill-injected context in system prompts is wrapped with trust boundary markers.
4. **Subprocess env_clear()**: Skill code execution uses environment isolation.

### Ecosystem Bridges

- **Marketplace/ClawHub compatibility**: frozen outside the active core release
  path. Do not present these as Hermes-level active product flows.
- **SKILL.md Parser**: Auto-converts OpenClaw SKILL.md format (YAML frontmatter + Markdown body) to `skill.toml`.
- **Tool Compat**: OpenClaw-to-Captain tool name mappings in `tool_compat.rs`.

---

## MCP and A2A Protocols

### Model Context Protocol (MCP)

Captain implements both MCP client and server:

- **MCP Client** (`mcp.rs`): JSON-RPC 2.0 over stdio or SSE transports. Connects to external MCP servers. Tools are namespaced as `mcp_{server}_{tool}` to prevent collisions. Background connection in `start_background_agents()`.
- **MCP Server** (`mcp_server.rs`): Exposes Captain's built-in tools via the MCP protocol. Enables external tools to use Captain as a tool provider.
- **Configuration**: `KernelConfig.mcp_servers` (Vec of `McpServerConfigEntry` with name, command, args, env, transport).
- **API**: `/api/mcp/servers` returns configured and connected servers with their tool lists.

### Agent-to-Agent Protocol (A2A)

Google's A2A protocol for inter-system agent communication:

- **A2A Server** (`a2a.rs`): Publishes `AgentCard` at `/.well-known/agent.json`. Handles task lifecycle (send, get, cancel).
- **A2A Client** (`a2a.rs`): Discovers and communicates with remote A2A-compatible agents.
- **Endpoints**: `/.well-known/agent.json`, `/a2a/agents`, `/a2a/tasks/send`, `/a2a/tasks/{id}`, `/a2a/tasks/{id}/cancel`.
- **Configuration**: `KernelConfig.a2a` (optional `A2aConfig`).

---

## Wire Protocol (OFP)

The Captain Protocol (OFP) enables peer-to-peer agent communication across machines.

### Architecture

```
Machine A                          Machine B
+-----------+                      +-----------+
| PeerNode  | ---TCP (JSON)------> | PeerNode  |
| port 4200 | <---TCP (JSON)------ | port 4200 |
+-----------+                      +-----------+
| PeerRegistry |                   | PeerRegistry |
| - Known peers |                  | - Known peers |
| - Remote agents |                | - Remote agents |
+---------------+                  +---------------+
```

### HMAC-SHA256 Mutual Authentication

Before any protocol messages are exchanged, both peers authenticate:

1. Initiator sends `{nonce, node_id, hmac_sign(shared_secret, nonce + node_id)}`.
2. Responder verifies HMAC using constant-time comparison (`subtle` crate).
3. Responder sends its own `{nonce, node_id, hmac}` challenge.
4. Initiator verifies.
5. On mutual success, the connection is established.

Configured via `PeerConfig.shared_secret` (required) and `NetworkConfig.shared_secret` in `config.toml`.

### Protocol Messages

All messages are JSON-framed (newline-delimited JSON over TCP):

```
WireMessage {
    id: UUID,
    sender: PeerId,
    payload: WireRequest | WireResponse
}
```

**Request types:**
- `Discover` -- Request peer information and agent list
- `Advertise` -- Announce local agents to a peer
- `RouteMessage` -- Send a message to a remote agent
- `Ping` -- Keepalive

**Response types:**
- `DiscoverResponse` -- Peer info and agent list
- `RouteResponse` -- Agent's response to a routed message
- `Pong` -- Keepalive response

### PeerRegistry

Tracks all known peers and their advertised agents:

```rust
pub struct PeerEntry {
    pub id: PeerId,
    pub addr: SocketAddr,
    pub agents: Vec<RemoteAgent>,
    pub last_seen: Instant,
}

pub struct RemoteAgent {
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
}
```

### Capability Gating

OFP operations require capabilities:
- `OfpDiscover` -- Required to send discover requests
- `OfpConnect(addr)` -- Required to connect to a specific peer
- `OfpAdvertise` -- Required to advertise agents to peers

---

## Operator Surfaces

The active desktop path is a persistent daemon with two equivalent operational
surfaces:

- the Ratatui TUI, launched with `captain`, with Chat, Projects, Automation,
  Learning, Capabilities, and Status;
- the authenticated Control web app served by `captain-api`, with the same six
  hubs and the same `/api/status` operational contract.

CLI and external channels call the same kernel. Agent-as-service clients use
the per-agent ingress/egress API. The `captain-desktop` Tauri crate is retained
as frozen compatibility code and is not packaged or promoted by the active
release workflow.

---

## Subsystem Diagram

```
CLI/TUI       Control Web       Channels       Agent API clients
   \              |               /                  /
    +-------------+--------------+------------------+
                          |
                    captain-api
                          |
                    captain-kernel
        +-----------------+-------------------+
        |                 |                   |
 captain-runtime   captain-memory      automation/projects
        |                 |                   |
 tools + LLMs      sessions + runs      workflows + goals
        +-----------------+-------------------+
                          |
                    captain-types

Frozen compatibility: captain-desktop, captain-hands,
captain-extensions, long-tail channels, A2A/OFP promotion surfaces.
```
