# Security Profiles — Captain

Captain containers ship with **4 security profiles** to match different needs : from full sandbox to total host access. Choose your fighter.

## Quick selector

```bash
# Default (sandbox)
docker compose up

# Personal Mac/laptop usage
docker compose -f docker-compose.yml -f docker-compose.personal.yml up

# Trusted environment (VPS, dev workstation)
docker compose -f docker-compose.yml -f docker-compose.trusted.yml up

# Full control (you know what you're doing)
docker compose -f docker-compose.yml -f docker-compose.yolo.yml up
```

## Profile matrix

| Capability | 🔒 sandbox | 🏠 personal | 💼 trusted | ⚡ yolo |
|---|---|---|---|---|
| Internet outbound | ✅ | ✅ | ✅ | ✅ |
| Read host files | ❌ | `~/Desktop`, `~/Documents`, `~/Downloads` | `~/`, `/tmp` | `/` |
| Write host files | ❌ | Same as above | Same as above | `/` |
| Run host commands | ❌ | SSH whitelist | SSH whitelist | Direct |
| Spawn containers | ❌ | ❌ | ✅ Docker socket | ✅ |
| Host network | ❌ | ❌ | ❌ | ✅ |
| Privileged | ❌ | ❌ | ❌ | ✅ |
| **Risk if compromised** | None (only container) | Limited dirs | Most user data | Full machine |

## 🔒 Profile: `sandbox` (default)

The agent lives in a perfectly isolated container. It can talk to the internet (LLM APIs) and that's it. If Captain decides to `rm -rf /`, only the container dies.

Use cases:
- Demo, learning, exploration
- Untrusted prompt sources
- Distribution to people who don't trust you

## 🏠 Profile: `personal`

Bind mounts for common user dirs. SSH bridge with whitelist for shell commands. The agent can:
- Read/write files in `~/Desktop`, `~/Documents`, `~/Downloads` (mounted as `/host/Desktop`, etc.)
- Run a curated list of safe commands on the host via SSH (`ls`, `osascript`, `open`, etc.)
- Cannot touch system files, install software, or modify other dirs

### Setup

1. **Generate dedicated SSH key** :
```bash
ssh-keygen -t ed25519 -f ~/.ssh/captain_host -N ""
```

2. **Install the shim** :
```bash
mkdir -p ~/bin
cp scripts/ssh-bridge.sh ~/bin/captain-shim
chmod +x ~/bin/captain-shim
```

3. **Authorize the key** in `~/.ssh/authorized_keys` :
```
command="/Users/yourname/bin/captain-shim",no-port-forwarding,no-X11-forwarding ssh-ed25519 AAAA... captain
```

4. **Enable Remote Login** (Mac) : System Settings → General → Sharing → Remote Login

5. **Test** :
```bash
ssh -i ~/.ssh/captain_host yourname@host.docker.internal "uname -a"
```

6. **Run** :
```bash
docker compose -f docker-compose.yml -f docker-compose.personal.yml up
```

### Customizing the whitelist
Edit `~/bin/captain-shim` and add binary names to `ALLOWED_COMMANDS`. Each allowed command runs via `exec` with its arguments as an argv array — never through a shell — so there's no pattern syntax to get wrong. Each command attempt is logged in `~/.captain-ssh.log`.

## 💼 Profile: `trusted`

Same as `personal` plus:
- Mounts entire `~/` (read-write) — agent can touch everything in your user dir
- Mounts `/var/run/docker.sock` — agent can spawn other containers
- SSH bridge same whitelist (you can extend it)

Use cases:
- VPS where the agent is the manager
- Dev workstation where you fully trust the agent
- DevOps automation

## ⚡ Profile: `yolo`

**No restrictions.** The container shares the host network namespace, runs privileged, mounts `/`, can do anything. Practically same as running the binary natively without container.

Use cases:
- You explicitly want zero isolation
- The host is dedicated to Captain
- You are testing destructive operations safely (because the host is disposable)

⚠️ Don't use this on a host that contains data you care about.

## Combining with .env

All profiles inherit env vars from `.env`. Example :
```env
OPENROUTER_API_KEY=sk-...
TELEGRAM_BOT_TOKEN=...
CAPTAIN_PROFILE=personal
```

## Switching profiles

The volume `captain-data` persists across profile switches — your memory and config follow you. Just stop and start with a different override file.

```bash
docker compose down
docker compose -f docker-compose.yml -f docker-compose.trusted.yml up
```

## Audit

Every action the agent takes through SSH is logged in `~/.captain-ssh.log`. The container internal audit log lives in `/root/.captain/MEMORY/SECURITY/`.
