# Release Supply-Chain Baseline — 2026-07-30

## Scope

This evidence covers the local release and repository-governance part of T14.
It does not claim that the next release artifacts already exist or that the
remote branch policy has already been applied.

## Local-Only CI Decision

The operator requires local builds and gates to avoid hosted GitHub Actions
billing. Automatic `push`, `pull_request`, and scheduled workflows remain
disabled. The three-OS CI and release workflows remain manual fallbacks.

This changes execution location, not the mandatory checks:

- `scripts/release-readiness.sh` remains the pre-publication authority;
- dependency, secret, docs, public-export, test, build, and live-smoke gates
  run locally;
- no release can claim completion from a manual workflow definition alone.

## Host Artifact Provenance

`scripts/release-provenance.sh` generates one deterministic in-toto Statement
v1 with a SLSA provenance v1 predicate after the five host targets complete.
The 20 pre-existing assets become subjects. The statement binds them to:

- the canonical public source URI, Git commit, and Git tree;
- the exact `Cargo.lock` SHA-256;
- all five targets and the release profile;
- local Rust/Cargo and builder-platform observations;
- each target's real build-start and package-finish timestamps;
- sequential target execution.

The statement and checksum bring the publication contract to 22 assets. The
isolated test generates a complete synthetic release, verifies it, changes one
platform source revision and proves generation fails, then changes one archive
and proves verification fails.

The statement is not independently signed in this alpha. It is
machine-readable and digest-bound, but does not claim a SLSA certification
level or a transparency-log identity.

## Sequential Docker Publication

The local publisher no longer uses a combined
`--platform linux/amd64,linux/arm64` build. It:

1. checks free internal disk and load;
2. builds and pushes `linux/amd64` by digest with BuildKit provenance
   `mode=max`;
3. verifies the digest and checks the host again;
4. repeats for `linux/arm64`;
5. creates the version/channel index only after both complete;
6. requires both Linux manifests and at least two attestation manifests.

`release-all.sh` also checks host capacity before and after every host target.
The actual release execution must still invoke each target separately and wait
for completion before starting the next.

## Branch Protection

`scripts/github-governance.sh` defines the exact public `main` policy:
one approving review for non-admin contributors, stale-review dismissal,
last-push approval, resolved conversations, linear history, no force-push, and
no branch deletion. Administrators retain the local publisher path and no
automatic hosted status check is required.

The local policy test is green. Remote application and read-back remain a
publication-time T14 exit condition because the workstation's current `gh`
credential is invalid. No document may claim the remote state is active until
`scripts/github-governance.sh --apply` and `--verify` both succeed.

## Reproducible Gate

The tranche gate passed:

- shell syntax for six release/governance scripts;
- guarded execution audit: 9 controlled sinks;
- provenance generation, verification, and tamper rejection;
- local governance policy validation;
- release workflow audit: 125 checks;
- DOC2 global audit: 630 checks;
- docs release audit: 95 checks;
- staged and unstaged diff checks.

T14 remains open for live branch-policy application, clean-source release
readiness, five real sequential bundles, Docker digests/attestations, and
remote publication verification.
