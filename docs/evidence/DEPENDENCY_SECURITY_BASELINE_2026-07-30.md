# Dependency Security Baseline — 2026-07-30

## Scope

This evidence closes the dependency-hardening part of audit tranche T14. The
release gate is `scripts/dependency-audit.sh`; `Cargo.lock` is the authority.

## Removed Findings

| Finding | Resolution |
|---|---|
| `bincode 1.x` unmaintained | Replaced by a versioned JSON cache envelope |
| `time 0.3.45` / RUSTSEC-2026-0009 | `notify-rust 4.18.0`, `mac-notification-sys 0.6.15`, `plist 1.10.0`, and `time 0.3.54` |
| `quick-xml 0.38.4` / RUSTSEC-2026-0194 and 0195 | Main plist/Tauri path upgraded to `quick-xml 0.41.0` |
| `rsa 0.9.10` and `rsa 0.10.0-rc.18` / RUSTSEC-2023-0071 | RSA features removed from `ssh-key` and `russh`; vulnerable crates absent |
| `imap-proto 0.10.2` future Rust rejection | `imap 3.0.0-alpha.15` pinned to the maintained `imap-proto 0.16.7` line |
| `lexical-core 0.7.6` / RUSTSEC-2023-0086 | Removed with the obsolete IMAP parser chain |
| `lru 0.12.5` / RUSTSEC-2026-0002 and RUSTSEC-2026-0253 | Ratatui stack upgraded to `ratatui 0.30.2`, `ratatui-core 0.1.2`, `ratatui-image 11.0.6`, and `ratatui-explorer 0.3.0`; the sole resolved LRU is fixed `lru 0.18.2` |
| `h2 0.4.13` / RUSTSEC-2026-0258 | Lockfile refreshed to patched `h2 0.4.16` before Alpha 15 publication |

Captain now accepts Ed25519 and ECDSA P-256 SSH private keys. RSA-only client
keys and server host keys fail closed with an actionable message.

## Reviewed Exceptions

The unfiltered audit contains no vulnerability record. The lockfile already
contained no Windows-only
`tauri-winrt-notification 0.7.2 -> quick-xml 0.37.5` path, while the previous
gate baseline still expected it. Those stale assertions and exceptions are now
removed. The sole resolved `quick-xml` is patched `0.41.0`, inherited through
`plist 1.10.0`.

## Informational Warnings

- `number_prefix 0.4.0` is inherited only through `indicatif 0.17.11` and the
  FastEmbed/ONNX ABI pinned for release portability.
- `spin 0.9.8` is yanked, not vulnerable, and inherited only through
  `flume 0.12.0`.
- The unfiltered warning set for Tauri GTK3 and its target-specific transitive
  crates is pinned by package, version, and advisory. Parent chains are pinned
  for the remaining quick-xml, FastEmbed, and mDNS warnings.

The IMAP client is deliberately pinned to `imap 3.0.0-alpha.15`: this
prerelease preserves Captain's synchronous login/search/fetch API while moving
the parser to `imap-proto 0.16.7`. Captain explicitly selects implicit TLS for
every configured IMAP port, matching the previous client instead of allowing
the builder to switch a custom port to STARTTLS. Both versions, the only direct
parent of `imap-proto`, the `native-tls` feature, and vendored TLS are enforced
by the gate. The exact pin prevents an ordinary lock refresh from accepting a
later prerelease API change; it should be replaced by the stable 3.0 line once
upstream publishes it.

The native Gmail authorization path pins `oauth2 5.0.0` with only its
`reqwest` and `rustls-tls` features. OAuth browser redirects remain disabled
by Captain's own HTTP client; the exact dependency and feature set are checked
to prevent a lock refresh from changing the token transport silently.

The maintained Ratatui 0.30 line raises the workspace MSRV to Rust 1.88. The
gate pins the complete TUI compatibility set and requires `lru 0.18.2` with
`ratatui-core 0.1.2` as its only direct parent. This prevents a lock refresh
from silently restoring either vulnerable 0.12.x LRU path.

## Fail-Closed Gate

Each gate run uses one fresh temporary RustSec checkout for all filtered and
unfiltered reports, then removes it. This prevents files renamed upstream from
surviving as untracked duplicates in cargo-audit's shared cache while keeping
every report pinned to the same database revision.

The gate fails when:

- the normal audit reports a vulnerability;
- the unfiltered vulnerability set becomes non-empty or its warning set changes;
- an exception is added without updating the reviewed baseline;
- RSA or its supporting crates reappear;
- `imap 3.0.0-alpha.15`, `imap-proto 0.16.7`, their parent chain, or vendored
  TLS features drift;
- the removed `lexical-core` parser reappears;
- an RSA feature is enabled on either resolved `ssh-key` version;
- `oauth2 5.0.0` or its minimal `reqwest`/`rustls-tls` feature set drifts;
- the FastEmbed/ORT ABI pins, mDNS chain, or direct parent
  chains drift.
- Ratatui, its image/explorer adapters, or the fixed LRU parent chain drift.

This baseline means **no unreviewed vulnerability**, not “the dependency graph
contains no advisory records.”
