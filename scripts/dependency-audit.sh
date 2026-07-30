#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT_DIR"

if ! command -v cargo-audit >/dev/null 2>&1; then
  echo "cargo-audit is required: cargo install cargo-audit --locked" >&2
  exit 1
fi

for command in jq; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "$command is required for the dependency warning baseline" >&2
    exit 1
  fi
done

report="$(mktemp "${TMPDIR:-/tmp}/captain-audit-report.XXXXXX")"
full_report="$(mktemp "${TMPDIR:-/tmp}/captain-audit-full-report.XXXXXX")"
metadata="$(mktemp "${TMPDIR:-/tmp}/captain-audit-metadata.XXXXXX")"
cleanup() {
  rm -f "$report" "$full_report" "$metadata"
}
trap cleanup EXIT

cargo audit

cargo audit --json --no-fetch >"$report"
jq -e '
  .vulnerabilities.found == false
  and .vulnerabilities.count == 0
  and (
    [
      .warnings
      | to_entries[]
      | .value[]
      | {
          kind,
          package: .package.name,
          version: .package.version,
          advisory: (.advisory.id // "")
        }
    ]
    | sort_by(.kind, .package, .version, .advisory)
  ) == [
    {
      "kind": "unmaintained",
      "package": "number_prefix",
      "version": "0.4.0",
      "advisory": "RUSTSEC-2025-0119"
    },
    {
      "kind": "yanked",
      "package": "spin",
      "version": "0.9.8",
      "advisory": ""
    }
  ]
' "$report" >/dev/null || {
  echo "cargo-audit informational warning baseline changed" >&2
  jq '.vulnerabilities, .warnings' "$report" >&2
  exit 1
}

set +e
(
  cd "${TMPDIR:-/tmp}"
  cargo audit --json --no-fetch --file "$ROOT_DIR/Cargo.lock"
) >"$full_report"
full_audit_status=$?
set -e
if [[ "$full_audit_status" -ne 1 ]]; then
  echo "unfiltered cargo-audit returned unexpected status $full_audit_status" >&2
  exit 1
fi

jq -e '
  (
    [
      .vulnerabilities.list[]
      | {
          advisory: .advisory.id,
          package: .package.name,
          version: .package.version
        }
    ]
    | sort_by(.advisory, .package, .version)
  ) == [
    {
      "advisory": "RUSTSEC-2026-0194",
      "package": "quick-xml",
      "version": "0.37.5"
    },
    {
      "advisory": "RUSTSEC-2026-0195",
      "package": "quick-xml",
      "version": "0.37.5"
    }
  ]
  and (
    [
      .warnings
      | to_entries[]
      | select(.key != "yanked")
      | .value[]
      | {
          kind: .kind,
          advisory: (.advisory.id // ""),
          package: .package.name,
          version: .package.version
        }
    ]
    | sort_by(.kind, .advisory, .package, .version)
  ) == [
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0370","package":"proc-macro-error","version":"1.0.4"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0411","package":"gdkwayland-sys","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0412","package":"gdk","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0413","package":"atk","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0414","package":"gdkx11-sys","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0415","package":"gtk","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0416","package":"atk-sys","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0417","package":"gdkx11","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0418","package":"gdk-sys","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0419","package":"gtk3-macros","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0420","package":"gtk-sys","version":"0.18.2"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2024-0436","package":"paste","version":"1.0.15"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0057","package":"fxhash","version":"0.2.1"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0075","package":"unic-char-range","version":"0.9.0"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0080","package":"unic-common","version":"0.9.0"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0081","package":"unic-char-property","version":"0.9.0"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0098","package":"unic-ucd-version","version":"0.9.0"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0100","package":"unic-ucd-ident","version":"0.9.0"},
    {"kind":"unmaintained","advisory":"RUSTSEC-2025-0119","package":"number_prefix","version":"0.4.0"},
    {"kind":"unsound","advisory":"RUSTSEC-2023-0086","package":"lexical-core","version":"0.7.6"},
    {"kind":"unsound","advisory":"RUSTSEC-2024-0429","package":"glib","version":"0.18.5"},
    {"kind":"unsound","advisory":"RUSTSEC-2026-0002","package":"lru","version":"0.12.5"},
    {"kind":"unsound","advisory":"RUSTSEC-2026-0097","package":"rand","version":"0.7.3"}
  ]
' "$full_report" >/dev/null || {
  echo "reviewed transitive advisory baseline changed" >&2
  jq '.vulnerabilities, .warnings' "$full_report" >&2
  exit 1
}

cargo metadata --format-version 1 --locked >"$metadata"
jq -e '
  . as $root
  | def versions($name):
      [$root.packages[] | select(.name == $name) | .version] | unique | sort;
    (versions("bincode") == [])
    and (versions("rsa") == [])
    and (versions("pkcs1") == [])
    and (versions("num-bigint-dig") == [])
    and (versions("fastembed") == ["5.13.2"])
    and (versions("ort") == ["2.0.0-rc.11"])
    and (versions("ort-sys") == ["2.0.0-rc.11"])
    and (versions("russh") == ["0.62.4"])
    and (versions("ssh-key") == ["0.6.7", "0.7.0-rc.11"])
    and (versions("mdns-sd") == ["0.20.3"])
    and (versions("notify-rust") == ["4.18.0"])
    and (versions("mac-notification-sys") == ["0.6.15"])
    and (versions("plist") == ["1.10.0"])
    and (versions("quick-xml") == ["0.37.5", "0.41.0"])
    and (versions("time") == ["0.3.54"])
    and (versions("number_prefix") == ["0.4.0"])
    and (versions("spin") == ["0.9.8"])
    and (
      ($root.packages[]
        | select(.name == "russh" and .version == "0.62.4")
        | .id) as $target
      | ($root.resolve.nodes[] | select(.id == $target) | .features | sort)
        == ["aws-lc-rs", "flate2"]
    )
    and (
      ($root.packages[]
        | select(.name == "ssh-key" and .version == "0.6.7")
        | .id) as $target
      | ($root.resolve.nodes[] | select(.id == $target) | .features | index("rsa"))
        == null
    )
    and (
      ($root.packages[]
        | select(.name == "ssh-key" and .version == "0.7.0-rc.11")
        | .id) as $target
      | ($root.resolve.nodes[] | select(.id == $target) | .features | index("rsa"))
        == null
    )
    and (
      ($root.packages[]
        | select(.name == "quick-xml" and .version == "0.37.5")
        | .id) as $target
      | [
          $root.resolve.nodes[]
          | select(any(.deps[]?; .pkg == $target))
          | .id as $parent
          | $root.packages[]
          | select(.id == $parent)
          | "\(.name)@\(.version)"
        ]
        | unique
        | sort
    ) == ["tauri-winrt-notification@0.7.2"]
    and (
      ($root.packages[]
        | select(.name == "number_prefix" and .version == "0.4.0")
        | .id) as $target
      | [
          $root.resolve.nodes[]
          | select(any(.deps[]?; .pkg == $target))
          | .id as $parent
          | $root.packages[]
          | select(.id == $parent)
          | "\(.name)@\(.version)"
        ]
        | unique
        | sort
    ) == ["indicatif@0.17.11"]
    and (
      ($root.packages[]
        | select(.name == "spin" and .version == "0.9.8")
        | .id) as $target
      | [
          $root.resolve.nodes[]
          | select(any(.deps[]?; .pkg == $target))
          | .id as $parent
          | $root.packages[]
          | select(.id == $parent)
          | "\(.name)@\(.version)"
        ]
        | unique
        | sort
    ) == ["flume@0.12.0"]
' "$metadata" >/dev/null || {
  echo "dependency package or parent-chain baseline changed" >&2
  exit 1
}

printf 'dependency baseline passed: no unreviewed vulnerabilities; 2 unreachable quick-xml advisories and exact informational warnings reviewed\n'
