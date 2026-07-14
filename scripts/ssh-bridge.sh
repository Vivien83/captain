#!/usr/bin/env bash
# SSH wrapper installed on the host to validate commands from Captain container.
# Install: copy to ~/bin/captain-shim, then in ~/.ssh/authorized_keys:
#   command="/Users/you/bin/captain-shim",no-port-forwarding ssh-ed25519 AAAA... captain

set -euo pipefail

# Commands allowed to run, by binary name only. The allowed command is
# executed directly via `exec "$BIN" "${ARGS[@]}"` — an argv array, never
# a string handed to a shell — so embedded `;`, backticks, `$()`, `|`,
# etc. in an argument are inert literal characters, not shell syntax.
#
# A previous version matched the WHOLE command against regexes like
# '^echo .+$' and ran the match via `exec bash -c "$CMD"`. Since `.`
# matches any character, `echo hi; rm -rf ~` satisfied that pattern, and
# `bash -c` then executed the `; rm -rf ~` part with full privileges —
# the whitelist offered no real protection at all.
ALLOWED_COMMANDS=(ls cat echo pwd date uname df ps uptime whoami osascript open)

CMD="${SSH_ORIGINAL_COMMAND:-}"
LOG="${HOME}/.captain-ssh.log"

log() {
  echo "[$(date +%FT%T)] $*" >> "$LOG"
}

if [[ -z "$CMD" ]]; then
  log "REJECT empty"
  echo "captain-shim: empty command rejected" >&2
  exit 1
fi

# Plain whitespace word-splitting — no quote handling, no `$()`/backtick/
# variable expansion. This is intentional: any tokenizer that understands
# quoting well enough to let a script pass one argument containing spaces
# necessarily also understands enough shell syntax to reintroduce the
# original injection risk. The one case that needs a multi-word single
# argument (`osascript -e "<script>"`) is handled explicitly below by
# rejoining the trailing words with spaces — plain string concatenation,
# not re-parsing — instead of widening the tokenizer.
read -ra ARGS <<< "$CMD"
BIN="${ARGS[0]:-}"

allowed=0
for candidate in "${ALLOWED_COMMANDS[@]}"; do
  if [[ "$BIN" == "$candidate" ]]; then
    allowed=1
    break
  fi
done

if [[ "$allowed" -eq 0 ]]; then
  log "REJECT $CMD"
  echo "captain-shim: command not in whitelist: $CMD" >&2
  echo "Edit ~/bin/captain-shim to add commands." >&2
  exit 1
fi

log "ALLOW $CMD"

if [[ "$BIN" == "osascript" && "${ARGS[1]:-}" == "-e" && "${#ARGS[@]}" -gt 2 ]]; then
  exec osascript -e "${ARGS[*]:2}"
fi

exec "$BIN" "${ARGS[@]:1}"
