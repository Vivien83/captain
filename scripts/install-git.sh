#!/usr/bin/env bash
# GitHub installer: forces release download even if a local bundle is present.
# Run from a checkout or via a hosted script endpoint.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd -P)
export CAPTAIN_INSTALL_SOURCE=git

exec bash "$SCRIPT_DIR/install.sh"
