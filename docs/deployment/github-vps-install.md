# GitHub + VPS Install

Captain's public GitHub Releases provide checksum-verified Linux bundles, so a
VPS does not need Rust, Cargo, or a source build.

## Managed Domain Install

The `v0.1.0-alpha.15` installer includes Captain's managed single-domain HTTPS
bootstrap. Interactive VPS setup asks for the domain; unattended installs use
the exact `CAPTAIN_DOMAIN` value below.

Before installing, create an `A` and/or `AAAA` record for the chosen hostname
and point it to the VPS. TCP ports `80` and `443` must be allowed by the VPS
provider firewall. Captain can update an active `ufw` or `firewalld` host
firewall, but it cannot modify an external cloud firewall or DNS account.

```bash
CAPTAIN_RELEASE=v0.1.0-alpha.15
curl -fsSL "https://github.com/Vivien83/captain/releases/download/$CAPTAIN_RELEASE/install.sh" \
  | CAPTAIN_VERSION="$CAPTAIN_RELEASE" \
    CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=captain.example.com \
    CAPTAIN_YES=1 \
    bash
```

Captain releases are GitHub prereleases, so this command pins an immutable tag
instead of using `/releases/latest`, which intentionally excludes prereleases.
The installer selects the host architecture, verifies its SHA-256 checksum and
platform manifest, runs setup, installs and enables the Captain systemd
service, and keeps the daemon on `127.0.0.1`. With `CAPTAIN_DOMAIN`, it also:

1. normalizes and validates an HTTPS-only DNS hostname;
2. waits for an `A` or `AAAA` answer and refuses an unresolved domain;
3. refuses to replace an unrelated listener already using port `80` or `443`;
4. installs Caddy, preserving an existing Caddyfile through one idempotent
   Captain-managed import;
5. validates the complete Caddy configuration before reload and restores the previous files if validation or activation fails;
6. opens `80/tcp` and `443/tcp` when `ufw` or `firewalld` is active;
7. waits for the public certificate, compares public and local Captain health,
   and verifies the Control root page.

A successful default install therefore ends with both
`https://captain.example.com/` and `/terminal` ready. Initial browser
authentication uses the generated administrator username and password, not a
Bearer token pasted into the page. The installer prints the username and the
path to `~/.captain/initial-credentials.txt`, which contains the generated
password and, when Captain generated it, the separate CLI/API key. The file is
owner-readable only; secret values are deliberately not copied into terminal,
cloud-init, or provisioning logs. A successful browser login creates the
HttpOnly session token automatically.

## Codex Login Before First Start

For Codex through a ChatGPT subscription, install the service without starting
it, complete device login, then start it:

```bash
CAPTAIN_RELEASE=v0.1.0-alpha.15
curl -fsSL "https://github.com/Vivien83/captain/releases/download/$CAPTAIN_RELEASE/install.sh" \
  | CAPTAIN_VERSION="$CAPTAIN_RELEASE" \
    CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=captain.example.com \
    CAPTAIN_YES=1 \
    CAPTAIN_START=0 \
    bash

captain login codex
systemctl start captain        # use systemctl --user for a non-root install
curl -fsS https://captain.example.com/api/health
```

In this deliberate two-phase path, Caddy is configured during installation but
the final public readiness probe is deferred because `CAPTAIN_START=0` keeps
Captain stopped until Codex login succeeds.

## Continuous Deployment Readiness

The installer's one-time HTTPS proof is followed by a daemon-owned check after
startup and every five minutes. It verifies the local API port and health,
public DNS, TLS, public port, Captain routing through the reverse proxy, public
health, and exact version parity. Independent local and public work runs in
parallel; DNS, pinned connection/TLS and public-health parsing remain ordered
because each depends on the previous result.

```bash
captain doctor --full
captain status --json | jq '.deployment.readiness'
```

Control shows the same state, individual checks, timestamps and deduplicated
actions in Status. These commands read a private, atomically replaced cache and
never trigger fresh outbound traffic. A stale cache is degraded, HTTP-only
public service cannot become ready, and DNS/TLS response details are reduced to
fixed operator-safe categories rather than exposing resolved IPs or raw
transport errors.

## Existing Reverse Proxy

Captain never edits an unrelated Nginx, Apache, Traefik, or manually managed
Caddy deployment. If `80` or `443` is already owned by a proxy other than the
system Caddy service, the managed install fails before changing proxy files.
Keep that proxy and rerun with `CAPTAIN_INSTALL_PROXY=0`, then configure it from
[VPS Web Terminal](vps-web-terminal.md). This is an explicit operator-owned
path; the installer will not claim that the public URL is ready.

Optional controls:

| Variable | Default | Effect |
|---|---:|---|
| `CAPTAIN_DNS_WAIT_SECONDS` | `60` | Maximum DNS propagation wait, capped at 900 seconds |
| `CAPTAIN_HTTPS_WAIT_SECONDS` | `180` | Maximum certificate/public readiness wait, capped at 900 seconds |
| `CAPTAIN_DNS_CHECK=0` | enabled | Skips only the early DNS lookup; the final HTTPS identity check still runs |
| `CAPTAIN_CONFIGURE_FIREWALL=0` | enabled | Leaves `ufw`/`firewalld` unchanged |
| `CAPTAIN_INSTALL_PROXY=0` | enabled with a domain | Disables all managed Caddy work |

Rerunning the same command updates only Captain's managed Caddy fragment and
does not duplicate the import. Captain rejects public URL paths, credentials,
wildcards, IP addresses, custom ports, plain HTTP, and conflicting
`CAPTAIN_PUBLIC_URL`/`CAPTAIN_DOMAIN` hosts.

## Private Fork or Mirror

The official public release needs no token. `CAPTAIN_GITHUB_TOKEN` remains
supported only when `CAPTAIN_GITHUB_REPO` points to a private fork or mirror:

```bash
CAPTAIN_GITHUB_REPO=owner/private-captain \
CAPTAIN_GITHUB_TOKEN=github_pat_xxx \
CAPTAIN_PROFILE=vps \
CAPTAIN_YES=1 \
scripts/install.sh
```

## Bundle Policy

Maintainers produce all host bundles locally and attach them to a GitHub
prerelease; tag pushes do not start a paid build. Linux assets are:

- `captain-x86_64-unknown-linux-gnu.tar.gz` for Intel/AMD VPS hosts.
- `captain-aarch64-unknown-linux-gnu.tar.gz` for ARM64 VPS hosts.

Each archive has a checksum and platform manifest. The installer also
provisions the architecture-specific ONNX Runtime used by Captain's local
embeddings path and verifies readiness before reporting a successful install.

This is an early-access release. Keep a state snapshot and review capabilities
before enabling remote tools. macOS bundles are ad-hoc signed but not
Apple-notarized; the Windows CLI is not Authenticode-signed.
