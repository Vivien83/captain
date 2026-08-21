# Hub, Console, Clients, and Nodes

Captain can run as one standalone installation or as one authoritative
Captain Full reached from lightweight Clients and optional execution Nodes.
Captain Full owns the model, memory, sessions, projects, goals, providers,
channels, automation, audit, agents, tools, and durable work.

There is no multi-primary synchronization. A Console never creates a second
memory, and a Node never becomes another agent runtime.

## Choose an Edition

| Edition | Use it for | Local tools | Local agent state |
|---|---|---:|---:|
| Captain Full | Standalone use or an authoritative Hub | Yes | Yes |
| Captain Console | TUI and local Web access to one or more Full installations | No | No |
| Captain Node | Optional files, commands, and workspaces on another machine | Approved grants only | No |

Standalone remains the default composition. Install Console alone when the
machine only needs to operate remote Captains. Add Node separately only when a
remote Captain must act on this machine.

## Install the Hub

Install Captain Full on the server and expose it through authenticated HTTPS.
The managed VPS installer can configure one domain; see
[GitHub + VPS Install](deployment/github-vps-install.md).

Open the fail-closed enrollment window for ten minutes:

```bash
captain devices pair
```

The same action is available from authenticated **Status > Devices**.
Enrollment is closed by default, closes automatically, and is closed again
after every Hub restart. Already approved devices can reconnect while new
enrollment is closed.

## Install and Pair Captain Console

macOS or Linux:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install-edition.sh \
  | CAPTAIN_EDITION=console CAPTAIN_VERSION=v0.1.0-alpha.15 bash

captain-console pair \
  --hub https://hub.example.com \
  --label Production
```

Windows PowerShell:

```powershell
$env:CAPTAIN_EDITION = "console"
$env:CAPTAIN_VERSION = "v0.1.0-alpha.15"
irm https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install-edition.ps1 | iex
captain-console pair --hub https://hub.example.com --label Production
```

Captain displays a one-time code and opens the Hub approval page when possible.
Approve the exact code while signed into the Hub, or use:

```bash
captain devices approve <CODE>
```

Operate configured Captains locally:

```bash
captain-console list             # live, bounded inventory
captain-console list --local     # no network request
captain-console use Production   # explicit default for future processes
captain-console rename Production Main
captain-console open             # private loopback Web gateway
captain-console tui              # lightweight shared-session TUI
```

Console can hold several independent profiles. It never merges their data.
Changing authority is explicit, visible, and completed only after the new
profile bootstraps successfully. An unavailable Hub remains unavailable;
Console never starts a local Full fallback.

Each profile is bound to one exact Hub instance UUID and HTTPS origin. The
long-lived credential lives in macOS Keychain, Windows Credential Manager, or
the supported Linux secret service. Local profile files contain only a secret
reference. A legacy profile migrates atomically and a conflicting native secret
blocks without overwrite.

The Client authority covers ordinary chat, sessions, projects, workflows,
memory, approvals, and cancellable Live Runs. It excludes secrets,
configuration, install/update, shutdown, device administration, and durable
approval grants. Revoke a lost Console immediately from **Status > Devices** or
with `captain devices remove <DEVICE_ID>`.

## Install and Pair Captain Node

Install Node only on a machine that owns a workspace:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install-edition.sh \
  | CAPTAIN_EDITION=node CAPTAIN_VERSION=v0.1.0-alpha.15 bash

captain-node pair \
  --hub https://hub.example.com \
  --workspace /path/to/project \
  --workspace-id project-main
```

Pairing requests read-only authority by default. Add `--allow-mutation` only
when the Hub must modify that workspace, then approve the separate mutation
request on the Hub:

```bash
captain devices approve <CODE> --allow-mutation
```

Inspect redacted local state and install the native user service:

```bash
captain-node status --json
captain-node service install
captain-node service status
```

Use `captain-node run` for a foreground diagnostic. The service lifecycle is:

```bash
captain-node service start
captain-node service stop
captain-node service uninstall
```

launchd, systemd user services, and Windows Service Control Manager all invoke
the same `service-runtime`. Ctrl+C, SIGTERM, and Windows SCM stop use the same
cooperative shutdown, which closes the worker and persists the stopped state
before returning. Windows installs explicitly under the current user so the
service can access that user's native credential store; it never falls back to
`LocalSystem`.

## Enterprise Network Path

Console and Node initiate outbound HTTPS 443 only. No inbound port, NAT rule,
VPN, UDP discovery, mDNS, or Telegram tunnel is required. WebSocket is
preferred; streaming HTTP and bounded long polling provide deterministic
fallbacks when an enterprise proxy rejects the upgrade.

Environment or explicit proxies, `NO_PROXY`, authenticated proxy passwords in
the native secret store, and enterprise CA bundles are supported. Credentials,
Hub origins, and raw network errors are excluded from status and logs.

## Routing Work

The execution target is `Auto` by default. Captain selects the Hub or an online
capable Node from the logical workspace and required tool family. A session or
project can pin `Hub` or one Node without prompting for every tool call.

Physical workspace paths stay on the Node. The Hub sees a logical workspace
identifier and sanitized results. Before every call, Node repeats workspace,
traversal, tool-family, mutation, approval, timeout, output, and guarded-shell
checks. An unavailable requested Node fails honestly and is never rewritten as
a Hub execution merely to make the action succeed.

## Failure and Recovery

The rail persists sequence numbers, acknowledgements, leases, heartbeat state,
approval evidence, idempotency keys, terminal evidence, and an outbox before
network delivery. Revocation is applied on the next authenticated request.

After disconnect or process crash, read-only work may resume when exact
evidence permits it. A mutation that may have happened but lacks terminal
evidence becomes `uncertain`; Captain does not replay it blindly. Repeating the
same idempotency key returns the reconciled result without executing the local
effect twice.

## Alpha 15 Scope

Alpha 15 publishes the separate Console and Node binaries. The retained Tauri
Desktop wrapper composes Console and can switch profiles from its native tray,
but no separately signed/notarized Desktop application bundle is included in
the public release.

Alpha 15 does not include a complete mobile application, application-data
connectors, multi-primary memory synchronization, or a machine tunnel through
Telegram. The versioned security and durability details are in
[Hub, Client, and Node Protocol](HUB_CLIENT_NODE_PROTOCOL.md).
