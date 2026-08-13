# Hub, Clients, and Nodes

Captain can run as one standalone installation or as one central Hub reached
from lightweight Clients and optional execution Nodes. The Hub remains a full
Captain installation. It is the only authority for memory, sessions, projects,
goals, providers, channels, automation, audit, skills, agents, and durable work.

There is no multi-primary database synchronization. A Client never creates a
second memory, and a Node never becomes another agent runtime.

## Choose a Role

| Role | Use it for | Runs tools locally |
|---|---|---|
| Standalone | One machine running the complete product | Yes |
| Hub | Central Captain, commonly on a VPS | Yes |
| Client | TUI, terminal, or Desktop access to the Hub | No |
| Node | Files, commands, and workspaces on another machine | Yes, within approved grants |

Standalone remains the default and composes the same Hub, Client, and local
execution responsibilities on one machine.

## Prepare the Hub

Install Captain on the server and expose it through authenticated HTTPS. The
managed VPS installer can configure one domain; see
[GitHub + VPS Install](deployment/github-vps-install.md).

On the Hub, open the fail-closed enrollment window for ten minutes:

```bash
captain devices pair
```

The Hub device rail is available on a standard installation, but enrollment
is closed by default. `[pairing].hub_enabled = false` is the explicit hard-off
switch; the separate legacy mobile pairing routes remain disabled unless
`[pairing].enabled = true` is set.

The same action is available from the authenticated **Status > Devices**
surface. Enrollment closes automatically and is closed after every Hub
restart. Existing approved devices can reconnect while enrollment is closed.

## Pair a Lightweight Client

On the Mac, Linux, or Windows machine that will display the remote Captain:

```bash
captain client pair --hub https://hub.example.com
```

Captain displays a one-time code and opens the Hub approval page when possible.
Approve the exact code while signed into the Hub, or use:

```bash
captain devices approve <CODE>
```

Then use the normal surfaces:

```bash
captain client status
captain chat
captain tui
```

The Client reuses the Hub's sessions, projects, memory, model, tools, and
approvals. It has no local provider loop and cannot silently fall back to a
local daemon if the Hub is unavailable. Its authority includes ordinary chat,
session, project, workflow, memory, approval, and cancellable Live Run actions.
It excludes secrets, configuration, install/update, shutdown, device
administration, and persistent or session-wide approval grants.

To remove only the local Client identity:

```bash
captain client reset --yes
```

Revoke a lost Client immediately from Status > Devices or with
`captain devices remove <DEVICE_ID>` on the Hub.

## Pair an Execution Node

On a machine that owns the workspace:

```bash
captain node pair \
  --hub https://hub.example.com \
  --workspace /path/to/project \
  --workspace-id project-main
```

Pairing requests read-only access by default. Add `--allow-mutation` only when
the Hub must modify that workspace, then approve the mutation request
explicitly on the Hub:

```bash
captain devices approve <CODE> --allow-mutation
```

Inspect the local, redacted state and run the worker:

```bash
captain node status --json
captain node run
```

`captain node run` is the Alpha 14 foreground service surface. Keep it under an
operator-controlled supervisor if it must start automatically. A future native
service wrapper must reuse this identity and durable rail rather than create a
second executor.

The Node opens only outbound HTTPS port 443. It prefers authenticated
WebSocket, then falls back to HTTPS streaming and bounded long polling. No
inbound port, NAT rule, UDP discovery, mDNS, or Telegram tunnel is required.
`HTTPS_PROXY`, `NO_PROXY`, an explicit proxy, authenticated proxy credentials
stored as a Captain secret, and an enterprise CA bundle are supported.

Physical workspace paths remain on the Node. The Hub sees logical workspace
identifiers and sanitized results only. Every run is checked again against the
local workspace binding, tool family, mutation grant, approval digest, path
policy, and runtime guard before an effect starts.

## Routing Work

The execution target is `Auto` by default. Captain selects the Hub or an online
capable Node from the logical workspace and requested tool family. A session or
project can pin `Hub` or one Node; users are not prompted for every tool call.

Offline or incompatible devices remain visible with a reason, but are not
selectable. The Hub never rewrites a requested Node execution as a local Hub
execution merely to make the request succeed.

## Failure and Recovery

Credentials are per-device, short-lived access tokens are kept out of durable
configuration, and revocation is checked by the Hub. The local rail persists
sequence numbers, acknowledgements, leases, approval evidence, terminal
results, and an outbox before network delivery.

An idle Node follows the heartbeat interval negotiated in the Hub `Welcome`
(15 seconds with the current 60-second lease). The refresh keeps WebSocket,
HTTPS streaming, and long-poll Nodes selectable even when no run starts or
finishes. A long-poll is bounded by the negotiated interval and completes before
the refresh, so a proxy never retains a poll that Captain abandoned only for a
timer. An unacknowledged heartbeat is reused rather than duplicated.

After a disconnect or process crash, read-only work may resume when the exact
evidence permits it. A mutation that may have happened but lacks terminal
evidence becomes `uncertain`; Captain does not replay it blindly. Repeating the
same idempotency key returns the reconciled durable result without executing
the local effect twice.

## Alpha 14 Scope

Alpha 14 does not include a complete mobile application, application-data
connectors, or a machine tunnel through Telegram. The complete wire and
security contract is documented in
[Hub, Client, and Node Protocol](HUB_CLIENT_NODE_PROTOCOL.md).
