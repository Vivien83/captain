# Release Provenance

Captain releases are built and published from a maintainer workstation. The
normal path does not use GitHub Actions.

## Host Artifacts

`scripts/release-provenance.sh` generates
`provenance.intoto.jsonl` after all 15 host bundles have completed. The file
contains one [in-toto Statement v1](https://in-toto.io/Statement/v1) with a
[SLSA provenance v1](https://slsa.dev/provenance/v1) predicate.

The statement binds:

- the 52 host assets that existed before the statement was generated;
- the public Git source URI, commit, and tree;
- the exact `Cargo.lock` SHA-256;
- all three components, all five release targets, and the release Cargo profile;
- the local Rust and Cargo versions;
- the earliest real target-build start and the last platform-package finish;
- the fact that target builds were completed sequentially.

The statement and its SHA-256 sidecar bring the release asset count to 54.
`scripts/publish-release-local.sh` regenerates and verifies both files before
upload. A changed archive, checksum, manifest, installer, source revision, or
lockfile makes verification fail.

Each component/platform manifest records the target-build start, package
finish, source revision, tree, lockfile digest, and dirty-tree state.
Aggregate-manifest generation rejects a missing component/target pair or mixed
source records, so an older bundle cannot be silently combined with a newer
source checkout before attestation.

This alpha provenance is machine-readable and digest-bound, but it is not an
independently signed transparency-log attestation. The macOS binaries remain
ad-hoc signed and the Windows binary remains unsigned. The release does not
claim SLSA build level certification or Apple/Microsoft code-signing identity.

## Container Images

The local publisher builds `linux/amd64` to a registry digest, waits for it to
finish, checks host capacity, then builds `linux/arm64` the same way. Only
after both digests are inspectable does it assemble the version and channel
indexes.

Each architecture uses BuildKit provenance in `mode=max`. Remote verification
requires both Linux image manifests and at least two BuildKit attestation
manifests. A combined local `--platform linux/amd64,linux/arm64` build is
forbidden so release work cannot saturate the maintainer Mac with parallel
architectures.

## Verification

From a release directory:

```bash
CAPTAIN_VERSION=vX.Y.Z scripts/release-provenance.sh --verify
```

The verifier requires a clean checkout at the recorded source revision and
recomputes every subject digest. `scripts/release-provenance-test.sh` also
proves that a modified subject is rejected.
