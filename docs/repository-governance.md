# Repository Governance

Captain defines public `main` protection that is compatible with the project's
local-only release policy:

- non-admin contributions go through a pull request with one approval;
- stale approvals are dismissed and the last push needs approval;
- review conversations must be resolved;
- history must remain linear;
- force-pushes and branch deletion are forbidden;
- repository administrators are not bound by the PR rule, so the audited local
  publisher can push the reviewed public export;
- no GitHub status check is required automatically.

The repository is owned by an individual account. The policy therefore omits
GitHub's organization-only `bypass_pull_request_allowances` field; administrator
bypass is controlled solely by `enforce_admins: false`.

The last point is deliberate. `scripts/release-readiness.sh` is the mandatory
release gate and `.github/workflows/ci.yml` remains an explicit
`workflow_dispatch` fallback. Automatic `push`, `pull_request`, and scheduled
workflows are disabled to avoid hosted CI billing. This changes where the
checks run, not which checks are required.

## Apply

After authenticating `gh` with repository administration permission:

```bash
scripts/github-governance.sh --apply
```

The command applies the exact policy and reads it back before succeeding.
Documentation must not claim the remote policy is active until this command
and a separate `--verify` both pass.

## Verify

```bash
scripts/github-governance.sh --verify
```

The local contract can be tested without network access:

```bash
scripts/github-governance.sh --policy-test
```
