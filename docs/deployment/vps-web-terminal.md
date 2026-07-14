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

## Configuration

Fresh installs created through `captain setup --profile vps` or
`captain setup vps` now bootstrap secure access first:

- root `api_key` for CLI/API Bearer auth;
- `[auth]` web terminal username/password session login;
- `~/.captain/initial-credentials.txt` when setup generated initial secrets;
- `/terminal` enabled in Captain chat mode by default.

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
username = "admin"
password_hash = "<sha256>"
session_ttl_hours = 72
```

To expose raw shell mode for technical clients, set:

```toml
[web_terminal]
allow_raw_shell = true
```

For a VPS, keep `session_ttl_hours` between 24 and 72. New installs default to
72 hours; lower it to 24 hours for highly exposed hosts.

The interactive browser page requires web-session auth so it uses the HttpOnly
`captain_session` cookie. The terminal WebSocket still accepts API-key auth for
technical clients and automation, but the browser UI does not ask users to paste
API keys.

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

Caddy example:

```caddyfile
captain.example.com {
  encode zstd gzip
  reverse_proxy 127.0.0.1:50051
}
```

When `CAPTAIN_DOMAIN` or `CAPTAIN_PUBLIC_URL` is provided during VPS setup,
Captain also writes a ready-to-review Caddyfile to
`~/.captain/deploy/Caddyfile`.

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
