# GitHub + VPS Install

Captain's public GitHub Releases provide checksum-verified Linux bundles, so a
VPS does not need Rust, Cargo, or a source build.

## One-Command Install

```bash
curl -fsSL \
  https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.7/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.7 CAPTAIN_PROFILE=vps CAPTAIN_YES=1 bash
```

The public alpha is a GitHub prerelease, so this command pins its immutable tag
instead of using `/releases/latest`, which intentionally excludes prereleases.
The installer selects the host architecture, verifies its SHA-256 checksum and
platform manifest, runs setup, installs the supported system service, starts
Captain, and verifies health.

To pin an immutable release:

```bash
curl -fsSL \
  https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.7/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.7 CAPTAIN_PROFILE=vps CAPTAIN_YES=1 bash
```

## Codex Login Before First Start

For Codex through a ChatGPT subscription, install the service without starting
it, complete device login, then start it:

```bash
curl -fsSL \
  https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.7/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.7 \
    CAPTAIN_PROFILE=vps \
    CAPTAIN_YES=1 \
    CAPTAIN_START=0 \
    bash

captain login codex
systemctl start captain        # use systemctl --user for a non-root install
```

## Domain and HTTPS

```bash
curl -fsSL \
  https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.7/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.7 \
    CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=captain.example.com \
    CAPTAIN_YES=1 \
    bash
```

Captain rejects an unauthenticated non-loopback API binding. Put public access
behind HTTPS and authentication, then follow
[VPS Web Terminal](vps-web-terminal.md) and [Security](../security.md).

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
