# VPS Web Terminal

Captain ships a native browser terminal at `/terminal`.
It also ships a focused authenticated config editor at `/config`.

Architecture:

- frontend: self-hosted xterm.js 6.0.0 with addon-fit and
  addon-unicode11 0.9.0 bundled into the Captain binary;
- terminal transport: same-origin WebSocket `/api/sessions/{id}/terminal`;
- conversation binding: explicit persisted `resume_session` UUID, validated
  against the selected agent and forwarded on every API turn;
- config editing: same-origin `/api/config/raw`, `/api/config/validate`,
  `/api/config/template`, and `/api/config/reload`;
- backend: Rust `portable-pty` session actor;
- default command: `captain chat`;
- raw shell: opt-in only;
- auth UX: embedded web-session login or API-key prompt;
- layout: responsive desktop/mobile with dynamic viewport sizing.

The same authenticated top bar opens a global **Live Runs** drawer. It lists
only selective execution metadata, refreshes while visible, and renders the
server-sanitized tail as bounded text. An interruption button appears only for
an active run with a real cancellable runtime handle, and the daemon rechecks
that condition before auditing the transition. Raw tool input, result preview,
output filename, managed path, retry, Telegram delivery, and provider delivery
are not exposed by this drawer.

The authenticated Control top bar also exposes one global **Fichiers
produits** drawer. It is not a seventh hub: the six operational hubs stay
unchanged. The drawer refreshes the immutable inventory in place, lets the
operator select an exact version, and provides a checksum-verified download.
Passive previews use the authenticated same-origin endpoint inside an empty
`sandbox` iframe with no referrer; SVG and unknown active formats remain
download-only. Managed filesystem paths never reach the browser. On narrow
mobile viewports, including foldable cover screens, the same drawer becomes a
full-width stacked inventory and preview instead of overflowing horizontally.

The embedded terminal runs standalone `captain chat`. Its `/artifacts` command
returns a bounded, payload-free inventory summary and points back to the
Control drawer; it does not write a download onto the VPS filesystem or render
active content inside Ratatui. The browser drawer remains the authoritative
preview and download surface.

Its `/runs` command is also intercepted before the model. It returns at most
twelve selective Live Runs metadata rows and directs the operator to full
Ratatui or the authenticated Control drawer for a redacted tail and confirmed
cancellation. The embedded terminal never gains a one-step stop command, raw
input/result access, or a managed filesystem path.

## Configuration

Fresh installs created through `captain setup --profile vps` or
`captain setup vps` now bootstrap secure access first:

- root `api_key` for CLI/API Bearer auth;
- `[auth]` web terminal username/password session login;
- `~/.captain/initial-credentials.txt` when setup generated initial secrets;
- `/terminal` enabled in Captain chat mode by default.

The browser never asks the operator to paste the daemon API key. It accepts the
administrator username and password, then creates an HttpOnly browser session.
At the end of a host install, Captain prints the username and the owner-only
credentials-file path, but not secret values that could leak into SSH,
cloud-init, or automation logs. The API key remains a separate credential for
CLI and external API clients.

```toml
[web_terminal]
enabled = true
default_mode = "captain"
allow_raw_shell = false
max_sessions = 4

[deployment]
profile = "vps"
public_url = "https://captain.example.com"
https = true
reverse_proxy = "caddy"

[auth]
enabled = true
allow_unauthenticated_loopback = false
username = "admin"
session_ttl_hours = 72
session_cookie_secure = "always"
```

Run `captain setup` or the native `web_credentials_update` tool to provision
the password. `password_hash`, `session_secret`, and `session_epoch` are
Captain-managed and intentionally omitted from the deployment snippet. The
session secret is unique per installation; password rotation increments the
epoch and invalidates every older browser session.

To expose raw shell mode for technical clients, set:

```toml
[web_terminal]
allow_raw_shell = true
```

For a VPS, keep `session_ttl_hours` between 24 and 72. New installs default to
72 hours; lower it to 24 hours for highly exposed hosts.

The interactive browser page requires web-session auth so it uses the HttpOnly
`captain_session` cookie. Set `session_cookie_secure = "always"` on an HTTPS
VPS. `"auto"` also enables `Secure` when the declared public URL is HTTPS or a
trusted loopback reverse proxy reports `X-Forwarded-Proto: https`; `"never"`
is only for explicit local HTTP development.

Before opening WebSocket or SSE, the browser exchanges its authenticated
cookie for a path/IP/session-epoch-bound ticket that expires after 30 seconds
and works once. Technical clients may still send API-key auth in headers.
Captain never accepts an API key or session token from a URL query string.
Do not enable `allow_unauthenticated_loopback` on a VPS. That escape hatch is
limited to the actual client IP and a declared local reverse proxy does not
make remote traffic eligible.

`/config` follows the same web-session rule. It edits the full `config.toml`
instead of a partial form, creates timestamped backups, validates before save,
and reloads hot settings after a successful write.

Unattended installer variables:

```bash
CAPTAIN_PROFILE=vps
CAPTAIN_SETUP=1
CAPTAIN_YES=1
CAPTAIN_DOMAIN=captain.example.com
CAPTAIN_ADMIN_USERNAME=admin
# CAPTAIN_ADMIN_PASSWORD=...       # generated if omitted
# CAPTAIN_DAEMON_API_KEY=...       # generated if omitted
# CAPTAIN_WEB_TERMINAL_SHELL=1     # explicit raw shell opt-in
```

## HTTPS Domain

Recommended VPS shape:

```text
Internet -> Caddy/Nginx TLS reverse proxy -> Captain on 127.0.0.1:50051
```

Captain's built-in login limiter is bounded and process-local. It preserves
active blocks under capacity pressure and briefly fails closed when every slot
is active, but it resets on daemon restart. Add an upstream login request limit
at the reverse proxy, firewall, WAF, or equivalent edge; TLS and Captain
authentication alone are not a distributed brute-force control.
The proxy must preserve Captain's `Content-Security-Policy` header instead of
replacing it: every browser script is an embedded same-origin asset and inline
or evaluated JavaScript is intentionally denied.

Caddy example:

```caddyfile
captain.example.com {
  encode zstd gzip
  reverse_proxy 127.0.0.1:50051
}
```

`captain setup --profile vps` with `CAPTAIN_DOMAIN` or `CAPTAIN_PUBLIC_URL`
writes a validated handoff Caddyfile to `~/.captain/deploy/Caddyfile`. The
The current `install.sh` goes further: it installs Caddy when needed, adds one
idempotent managed import to `/etc/caddy/Caddyfile`, validates before reload,
rolls back failed activation, and verifies the public `/api/health` plus Control
root. An existing non-Caddy listener on `80/443` is never replaced; use
`CAPTAIN_INSTALL_PROXY=0` and integrate the generated handoff manually.

Keep the original `Host` header. The terminal WebSocket checks that the browser
`Origin` host matches the request `Host`; changing the host header at the proxy
will intentionally break the connection.

## Security Policy

- Do not load terminal JavaScript from a CDN.
- Keep the vendored Unicode 11 addon active. Native TUI and browser width
  calculations must agree so emoji redraw, copy, and screen-reader output use
  the same terminal cells.
- Do not expose Captain on `0.0.0.0` without `api_key`.
- Prefer web-session auth for browser use.
- Keep raw shell disabled unless the VPS administrator explicitly needs it.
- Use HTTPS (`wss://`) for every non-local deployment.

## Durable Session Behavior

The terminal tab ID and the persisted conversation UUID are separate values.
Captain never assumes that a UUID-shaped terminal ID is a stored conversation.
Only an explicit `resume_session`, or a UUID validated against the persisted
history list, can reopen history.

The **New session** action first calls `POST /api/agents/{id}/sessions` with
`{"activate":false}`. The returned UUID is kept as `resume_session`, while the
PTY gets a separate `web-*` ID. Every streamed turn carries the persisted UUID.
Selecting a history row creates a fresh PTY and reloads the latest canonical
transcript, so work continued from another surface is not hidden behind stale
terminal process state.

The browser keeps only 18 local PTY IDs as a convenience cache, but the session
drawer queries global `/api/sessions` and does not truncate persisted history.
It therefore shows conversations created by Web, TUI, CLI, Desktop or API,
including sessions owned by specialized agents. Selecting one forwards both its
UUID and owner contract to `captain chat`, which resolves the real owner from
the persisted transcript; `/new` preserves the previous session and explicit
session/history deletion remains the destructive operation.
