#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

node --test crates/captain-api/static/js/app/provider_quota_model.test.mjs
node --test crates/captain-api/static/js/app/verification_phase_model.test.mjs
node scripts/control-web-contract-test.mjs
node --check crates/captain-api/static/js/app/api.js
node --check crates/captain-api/static/js/app/main.js
node --check crates/captain-api/static/js/app/components/Shell.js
node --check crates/captain-api/static/js/app/views/Approvals.js
node --check crates/captain-api/static/js/app/views/Automation.js
node --check crates/captain-api/static/js/app/views/Chat.js
node --check crates/captain-api/static/js/app/views/Status.js
