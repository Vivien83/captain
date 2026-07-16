# Deploying Captain

Captain ships as a single CLI/daemon bundle and as a public multiarchitecture
container image. The current public release is the prerelease
`v0.1.0-alpha.4`; pin it explicitly because GitHub's `/releases/latest`
endpoint excludes prereleases.

## Host Install

macOS, Linux, or a Linux VPS:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.4/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.4 CAPTAIN_PROFILE=desktop bash
```

Use `CAPTAIN_PROFILE=vps` for a service-oriented server install. The installer
selects the architecture, verifies the archive checksum and manifest, runs
setup, installs a supported launchd/systemd service, starts Captain, and checks
health. See [GitHub + VPS Install](deployment/github-vps-install.md) for Codex
device login and non-interactive options.

The macOS alpha binary is ad-hoc signed but not Apple-notarized. The Windows
CLI zip is not Authenticode-signed. Verify the published SHA-256 sidecar before
approving first launch.

## Public Container Image

The immutable image supports `linux/amd64` and `linux/arm64`:

```bash
docker pull ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.4

docker run -d --name captain --restart unless-stopped \
  -p 50051:50051 \
  -v captain-data:/root/.captain \
  -e CAPTAIN_LISTEN=0.0.0.0:50051 \
  -e MISTRAL_API_KEY \
  ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.4
```

The moving prerelease channel is `ghcr.io/vivien83/captain-agent-os:alpha`.
Production or reproducible deployments should use the immutable version tag.
Public image pulls require no registry login.

The named volume contains configuration, credentials, sessions, memory,
projects, and audit state. The release image includes the checksum-pinned local
embedding model and installs the architecture-specific ONNX Runtime during
image assembly.

## Compose

Clone the public source tree when you need a source build or a host-access
overlay:

```bash
git clone https://github.com/Vivien83/captain.git
cd captain
docker compose up -d --build
```

To consume the published image without rebuilding:

```bash
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.4 docker compose pull
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.4 docker compose up -d --no-build
```

The optional `personal`, `trusted`, and `yolo` overlays progressively grant
host filesystem, SSH, Docker socket, or privileged access. Review the compose
diff before enabling an overlay; the base profile exposes only the state volume
and API port.

## Verify

```bash
captain --version
captain status
captain doctor --full
curl http://127.0.0.1:50051/api/health
```

Control is available at `http://127.0.0.1:50051/`; the expert terminal is at
`http://127.0.0.1:50051/terminal`. Setup writes the initial web credentials to
`~/.captain/initial-credentials.txt` for a host install. Container state uses
the same path inside the named volume.

## Remote Access

Captain binds to loopback by default and refuses an unauthenticated public
binding. Keep Captain authentication enabled and place remote access behind an
HTTPS reverse proxy. Forward WebSocket/SSE upgrades and use long read timeouts
for terminal and streaming sessions. Follow
[VPS Web Terminal](deployment/vps-web-terminal.md) for the complete contract.

Do not expose port `50051` directly to the Internet without Captain auth and
TLS termination.

## Update

Host installs can rerun the pinned installer or use `captain update` after
reviewing the target version. Container installs should pull the desired
immutable tag and recreate the container:

```bash
docker pull ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.4
docker rm -f captain
# Re-run the same docker run command; captain-data preserves state.
```

## Backup and Reset

For a host install, use Captain's native snapshot rail:

```bash
captain snapshot create --reason before-upgrade
captain snapshot list
```

`captain reset --factory` stops the daemon and creates a recovery snapshot by
default before resetting state. Do not replace it with an unreviewed recursive
delete.

For Docker, back up the named volume while Captain is stopped:

```bash
docker stop captain
docker run --rm \
  -v captain-data:/data:ro \
  -v "$PWD":/backup \
  alpine tar czf /backup/captain-data.tar.gz -C /data .
docker start captain
```

## Diagnose

```bash
captain service status
captain logs daemon
docker logs captain
curl http://127.0.0.1:50051/api/health
```

Continue with [Troubleshooting](troubleshooting.md) for provider, channel,
session, authentication, and Docker checks.
