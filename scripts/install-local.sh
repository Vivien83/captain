#!/usr/bin/env bash
# Strict local installer: never downloads from GitHub.
# Put this file next to captain-<platform>.tar.gz and run:
#   bash install-local.sh

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd -P)
export CAPTAIN_INSTALL_SOURCE=local

exec bash "$SCRIPT_DIR/install.sh"
