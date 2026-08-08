# Repository Governance

Captain keeps hosted GitHub Actions manual-only while still requiring an
automatic pull-request check. The required context is
`captain/local-pr-gate`; it is produced on the maintainer Mac, not on a billed
GitHub runner.

Public `main` protection requires:

- one approval for non-admin contributions;
- dismissal of stale approvals and approval of the last push;
- resolved review conversations and linear history;
- no force-push or branch deletion;
- a successful, strict `captain/local-pr-gate` status on the current head SHA.

The repository belongs to an individual account, so the policy omits GitHub's
organization-only `bypass_pull_request_allowances` field. Administrators remain
outside the PR rule through `enforce_admins: false`; this preserves the audited
local public-export publisher.

## Discoverability Contract

Captain versions the public repository's discovery metadata instead of relying
on an unrecorded GitHub UI setting. `scripts/github-discoverability.sh` keeps a
search-oriented but factual description, the canonical product homepage, and a
bounded topic taxonomy for the Captain Agent OS brand, agent OS/framework
discovery, self-hosting, Rust, persistent memory, orchestration, Codex, MCP,
Telegram, Discord, and workflow automation.

After authenticating `gh` with repository administration permission, apply and
read back the exact public state:

```bash
scripts/github-discoverability.sh --apply
scripts/github-discoverability.sh --verify
```

The offline policy is covered by:

```bash
scripts/github-discoverability.sh --policy-test
```

This contract improves GitHub topic and text search. It does not fabricate
stars, backlinks, or ranking, and it does not claim that Google or Bing has
indexed the repository. External discovery remains asynchronous and is checked
separately through the canonical site and each search engine's webmaster tools.

## Isolation Contract

`scripts/local-pr-portal.sh` polls open pull requests sequentially. For each
missing check, stale `pending`, or retryable infrastructure error, the trusted
controller:

1. resolves and pins the protected base SHA and pull-request head SHA;
2. verifies the controller, worker, bootstrap, portal, exporter, auditors, and
   their policy files against that exact protected-base SHA;
3. clones a sealed base into a disposable plain-mode Lima VM with no host
   mounts, forwarded SSH agent, X11 forwarding, or container runtime;
4. installs a checksum-verified audit bundle and Rust toolchain under
   root-owned, non-writable paths, while keeping only the dependency cache
   writable by the dedicated `captain-pr` user;
5. fetches the exact public source and locked dependencies before any PR code
   executes, then creates both a full source snapshot and a public export;
6. invokes the single allowlisted root helper to seal those two snapshots,
   cut IPv4 and IPv6 egress, remove its own sudo grant, and verifies privilege
   and network loss;
7. runs formatting, Clippy, and workspace tests on the disposable checkout,
   while trusted guarded-exec, secret, and public-export policies inspect only
   the immutable snapshots;
8. destroys the VM, re-reads the PR, and publishes a final status only when the
   head SHA is unchanged.

No GitHub token, Captain secret, host directory, Docker socket, or SSH agent is
copied into the guest. A code failure becomes `failure`; a timeout or local
infrastructure problem becomes `error`. A host crash leaves `pending`; the
portal retries it after six hours and recovers stale local locks. Logs are
private, size-bounded, and retained for 30 days.

`.github/workflows/ci.yml` remains a `workflow_dispatch` fallback. Automatic
`push`, `pull_request`, and scheduled workflows stay disabled, so the required
check does not consume GitHub Actions minutes.

## Install The Portal

The host requires macOS, Lima, `gh`, and `jq`, at least 35 GiB of free internal
disk, and a GitHub credential able to read PRs and write commit statuses:

```bash
gh auth login -h github.com
scripts/local-pr-gate.sh --verify-controller
scripts/local-pr-portal.sh --install-launchd
```

The installer snapshots the complete verified controller and audit chain under
`~/.captain/local-pr-portal/controller`, writes no token into the launchd
property list, and checks every five minutes. Test one real pull request before
requiring the status.

## Apply And Verify

After the portal has produced a real status and `gh` also has repository
administration permission:

```bash
scripts/github-governance.sh --apply
scripts/github-governance.sh --verify
```

Both commands read the exact remote policy. Documentation must not claim that
the stricter remote policy is active until both pass. The offline contracts are:

```bash
scripts/local-pr-gate-test.sh
scripts/github-governance.sh --policy-test
```

The lightweight real-hypervisor boundary can be checked without compiling the
workspace:

```bash
scripts/local-pr-lima-smoke.sh
```

It creates and destroys a small plain VM and verifies mount, SSH-agent, and
container-runtime isolation. The full PR gate keeps its independent 35 GiB
host floor and refuses to create a build VM below it.
