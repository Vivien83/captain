#!/bin/sh
# Ensures a CAPTAIN_DAEMON_API_KEY exists before booting the daemon, and
# persists it into $CAPTAIN_HOME/secrets.env so it survives container
# recreation (the volume, not the container, is what's durable) instead
# of being regenerated on every restart.
#
# Needed because CAPTAIN_LISTEN=0.0.0.0:50051 (set in docker-compose.yml,
# required for the published port to reach the daemon at all) makes the
# kernel refuse to boot without an API key already configured — see
# kernel_boot_foundations.rs's non-loopback bind guard. Writes the exact
# same secrets.env format `captain setup` uses, so a later `captain
# setup` run sees it as already configured instead of overwriting it.
set -eu

CAPTAIN_HOME="${CAPTAIN_HOME:-/root/.captain}"
SECRETS_FILE="$CAPTAIN_HOME/secrets.env"

has_key() {
    [ -n "${CAPTAIN_DAEMON_API_KEY:-}" ] && return 0
    [ -n "${CAPTAIN_API_KEY:-}" ] && return 0
    [ -f "$SECRETS_FILE" ] && grep -qE '^(CAPTAIN_DAEMON_API_KEY|CAPTAIN_API_KEY)=' "$SECRETS_FILE"
}

if ! has_key; then
    mkdir -p "$CAPTAIN_HOME"
    generated_key="captain_api_$(od -An -tx1 -N24 /dev/urandom | tr -d ' \n')"
    printf 'CAPTAIN_DAEMON_API_KEY=%s\n' "$generated_key" >> "$SECRETS_FILE"
    chmod 600 "$SECRETS_FILE"
    echo "Generated CAPTAIN_DAEMON_API_KEY on first boot (persisted in \$CAPTAIN_HOME/secrets.env):"
    echo "  $generated_key"
fi

exec captain "$@"
