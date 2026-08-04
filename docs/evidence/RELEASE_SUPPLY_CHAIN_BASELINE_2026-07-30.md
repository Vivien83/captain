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
last-push approval, resolved conversations, linear history, no force-push, no
branch deletion, and strict `captain/local-pr-gate` success on the current SHA.
The status is generated locally in a disposable, mountless Lima clone; no
automatic hosted status check or GitHub Actions minute is required.

The mocked controller/portal contract and local policy test are green. A real
Lima boundary smoke on 2026-08-03 also passed on Ubuntu ARM64 with no host
mount, SSH agent, or container runtime; the disposable VM was destroyed.
Controller policy, worker, exporter, public auditor, and secret policy are
verified at the exact base SHA and installed root-owned in the guest. Source
and public-export snapshots are sealed before any pull-request code executes.

Remote application and read-back remain a post-Alpha 10 exit condition because
the workstation's current `gh` credential is invalid. The full real-PR gate
also retains its 35 GiB host floor; only 22 GiB were free during this proof.
No document may claim the remote state is active before one real portal status.
After that proof, `scripts/github-governance.sh --apply` plus `--verify` must
both succeed.

## Reproducible Gate

The tranche gate passed:

- shell syntax for the release/governance scripts;
- guarded execution audit: 11 controlled sinks;
- provenance generation, verification, and tamper rejection;
- local governance policy validation and mocked exact-SHA portal recovery;
- real mountless Lima isolation smoke;
- release workflow audit: 150 checks;
- DOC2 global audit: 715 maintainer checks and 710 public-export checks;
- docs release audit: 105 checks;
- public source audit and secret scan;
- staged and unstaged diff checks.

The remaining portal exits are a valid GitHub credential, one full real-PR
status, and live application/read-back of the stricter branch policy. Alpha 10
release publication evidence remains recorded separately and is not rewritten
by this post-release gate.
