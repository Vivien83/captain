#!/usr/bin/env bash

# Lifecycle, scope, Telegram, portability, and crash phases for the real
# CapSpec certification. Sourced by capspec-real-certification.sh.

certify_hot_reload() {
  local source_v1="$WORKDIR/sources/cert-hot-v1.captain"
  local source_v2="$WORKDIR/sources/cert-hot-v2.captain"
  local live_source="$HOME_DIR/capabilities/cert-hot.captain"
  cat >"$source_v1" <<'EOF'
format = 1
name = "cert-hot"
description = "Hot reload certification revision one."
version = "1.0.0"
output = "{{steps.read.output}}"
[permissions]
tools = ["file_read"]
read_paths = ["cert-repo/**"]
[[steps]]
id = "read"
tool = "file_read"
with = { path = "cert-repo/README.md" }
EOF
  sed 's/revision one/revision two/; s/version = "1.0.0"/version = "2.0.0"/' \
    "$source_v1" >"$source_v2"

  cp "$source_v1" "$live_source"
  wait_for_api_jq "/api/capabilities/native?scope=global" \
    'any(.capabilities[]; .name == "cert-hot" and .version == "1.0.0" and .ready)' \
    "$WORKDIR/hot-v1-active.json" || fail "hot reload did not activate v1"
  pass "hot reload activates a dropped read-only source"
  local hash_v1
  hash_v1="$(jq -r '.capabilities[] | select(.name == "cert-hot") | .active_hash' "$WORKDIR/hot-v1-active.json")"

  printf 'format = 1\nname = [broken\n' >"$live_source"
  wait_for_api_jq "/api/capabilities/native?scope=global" \
    "any(.capabilities[]; .name == \"cert-hot\" and .active_hash == \"$hash_v1\" and .last_error != null)" \
    "$WORKDIR/hot-invalid.json" || fail "invalid edit did not preserve active v1"
  pass "invalid hot edit preserves the last valid revision"

  cp "$source_v2" "$live_source"
  wait_for_api_jq "/api/capabilities/native?scope=global" \
    'any(.capabilities[]; .name == "cert-hot" and .version == "2.0.0" and .ready)' \
    "$WORKDIR/hot-v2-active.json" || fail "valid v2 hot edit did not activate"
  pass "valid contained hot edit activates without restart"
  local hash_v2
  hash_v2="$(jq -r '.capabilities[] | select(.name == "cert-hot") | .active_hash' "$WORKDIR/hot-v2-active.json")"
  [[ "$hash_v1" != "$hash_v2" ]] || fail "hot reload hashes did not change"

  rm -f "$live_source"
  wait_for_api_jq "/api/capabilities/native?scope=global" \
    'any(.capabilities[]; .name == "cert-hot" and (.ready | not))' \
    "$WORKDIR/hot-deleted.json" || fail "delete did not disable new runs"
  pass "delete disables without erasing history"

  install_capability "$source_v1" global "" hot-reinstall-v1
  assert_jq "$WORKDIR/hot-reinstall-v1-install-response.json" \
    '.ready == true and .active_hash != null' "exact-source reinstall restores v1"

  jq -n --arg target_hash "$hash_v2" \
    '{target_hash:$target_hash,scope:"global"}' >"$WORKDIR/hot-rollback-request.json"
  http_request POST "/api/capabilities/native/cert-hot/rollback" \
    "$WORKDIR/hot-rollback-request.json" "$WORKDIR/hot-rollback-response.json"
  assert_status 200 "rollback restores known v2 hash" "$WORKDIR/hot-rollback-response.json"
  assert_jq "$WORKDIR/hot-rollback-response.json" \
    ".ready == true and .active_hash == \"$hash_v2\" and .version == \"2.0.0\"" \
    "rollback result is the exact historical revision"
}

certify_permission_refusal() {
  local source_v1="$WORKDIR/sources/cert-expansion-v1.captain"
  local source_v2="$WORKDIR/sources/cert-expansion-v2.captain"
  cat >"$source_v1" <<'EOF'
format = 1
name = "cert-expansion"
description = "Read-only authority baseline."
version = "1.0.0"
output = "{{steps.read.output}}"
[permissions]
tools = ["file_read"]
read_paths = ["cert-repo/**"]
[[steps]]
id = "read"
tool = "file_read"
with = { path = "cert-repo/README.md" }
EOF
  cat >"$source_v2" <<'EOF'
format = 1
name = "cert-expansion"
description = "Authority-expanding write proposal."
version = "2.0.0"
output = "{{steps.write.output}}"
[permissions]
tools = ["file_write"]
write_paths = ["cert-work/**"]
[[steps]]
id = "write"
tool = "file_write"
with = { path = "cert-work/refused-expansion.txt", content = "must not exist" }
EOF

  install_capability "$source_v1" global "" expansion-v1
  local hash_v1
  hash_v1="$(jq -r '.active_hash' "$WORKDIR/expansion-v1-install-response.json")"
  install_capability "$source_v2" global "" expansion-v2
  assert_jq "$WORKDIR/expansion-v2-install-response.json" \
    '.human_action_required == true and .ready == true and .pending_hash != null' \
    "permission expansion waits for a human"
  decide_pending_capability "$WORKDIR/expansion-v2-install-response.json" reject global "" expansion-v2
  http_request GET "/api/capabilities/native/cert-expansion?scope=global" "" \
    "$WORKDIR/expansion-after-reject.json"
  assert_status 200 "inspect rejected expansion" "$WORKDIR/expansion-after-reject.json"
  assert_jq "$WORKDIR/expansion-after-reject.json" \
    ".active_hash == \"$hash_v1\" and .version == \"1.0.0\" and .pending_hash == null and .ready == true" \
    "refusal preserves the read-only active revision"
  [[ ! -e "$WORKSPACE/cert-work/refused-expansion.txt" ]] || fail "rejected expansion wrote a file"
  pass "rejected authority expansion has no side effect"
}

certify_project_scope() {
  local global_source="$WORKDIR/sources/cert-scope-global.captain"
  local project_source="$WORKDIR/sources/cert-scope-project.captain"
  mkdir -p "$PROJECT_ROOT/.captain/capabilities"
  cat >"$global_source" <<'EOF'
format = 1
name = "cert-scope"
description = "Global scope certification revision."
version = "1.0.0"
output = "{{steps.read.output}}"
[permissions]
tools = ["file_read"]
read_paths = ["cert-repo/**"]
[[steps]]
id = "read"
tool = "file_read"
with = { path = "cert-repo/README.md" }
EOF
  sed 's/Global scope/Project scope/; s/version = "1.0.0"/version = "2.0.0"/' \
    "$global_source" >"$project_source"

  install_capability "$global_source" global "" scope-global
  install_capability "$project_source" project "$PROJECT_ROOT" scope-project
  HTTP_STATUS="$(curl -sS --max-time "$TIMEOUT" -H "X-API-Key: $CERT_API_KEY" \
    -o "$WORKDIR/scope-effective-project.json" \
    -w '%{http_code}' -G "$BASE/api/capabilities/native" \
    --data-urlencode 'scope=effective' --data-urlencode "workspace=$PROJECT_ROOT")"
  assert_status 200 "query effective project scope" "$WORKDIR/scope-effective-project.json"
  assert_jq "$WORKDIR/scope-effective-project.json" \
    'any(.capabilities[]; .name == "cert-scope" and .scope == "project" and .version == "2.0.0")' \
    "project revision overrides global only in its workspace"

  local encoded_workspace
  encoded_workspace="$(jq -nr --arg value "$PROJECT_ROOT" '$value | @uri')"
  http_request DELETE "/api/capabilities/native/cert-scope?scope=project&workspace=$encoded_workspace" "" \
    "$WORKDIR/scope-project-delete.json"
  assert_status 200 "delete project override" "$WORKDIR/scope-project-delete.json"
  HTTP_STATUS="$(curl -sS --max-time "$TIMEOUT" -H "X-API-Key: $CERT_API_KEY" \
    -o "$WORKDIR/scope-effective-global.json" \
    -w '%{http_code}' -G "$BASE/api/capabilities/native" \
    --data-urlencode 'scope=effective' --data-urlencode "workspace=$PROJECT_ROOT")"
  assert_status 200 "query effective fallback scope" "$WORKDIR/scope-effective-global.json"
  assert_jq "$WORKDIR/scope-effective-global.json" \
    'any(.capabilities[]; .name == "cert-scope" and .scope == "global" and .version == "1.0.0")' \
    "deleting project override reveals global revision"
}

telegram_markup_callback() {
  jq -r '
    def markup:
      if (.body.reply_markup | type) == "string"
      then (.body.reply_markup | fromjson?)
      else .body.reply_markup end;
    [.telegramCalls[]
      | select(
          .method == "sendMessage"
          or .method == "sendMessageDraft"
          or .method == "sendRichMessage"
        )
      | select(
          (.body.text // .body.rich_message.markdown // "")
          | contains("cert-telegram-approval")
        )
      | markup.inline_keyboard[][]?
      | .callback_data
      | select(startswith("capspec:approve:"))][-1] // empty
  ' "$1"
}

certify_telegram_surface() {
  local before_run_id
  http_request GET "/api/capabilities/native/runs?limit=500" "" "$WORKDIR/telegram-runs-before.json"
  assert_status 200 "snapshot runs before Telegram invocation" "$WORKDIR/telegram-runs-before.json"
  before_run_id="$(jq -r '[.runs[] | select(.capability_name == "cert-parallel" and .origin == "telegram")][0].run_id // "none"' "$WORKDIR/telegram-runs-before.json")"
  jq -n '{message:{message_id:701,date:1784390000,chat:{id:4242,type:"private"},from:{id:4242,is_bot:false,first_name:"Certification"},text:"[CAPSPEC-CERT:telegram] Inspect the real certification repository."}}' \
    >"$WORKDIR/telegram-message-update.json"
  curl -sS --max-time "$TIMEOUT" -X POST -H 'Content-Type: application/json' \
    --data-binary @"$WORKDIR/telegram-message-update.json" \
    "$FIXTURE_BASE/cert/telegram/push" >"$WORKDIR/telegram-message-queued.json"
  assert_jq "$WORKDIR/telegram-message-queued.json" '.status == "queued"' "Telegram update enters the real polling adapter"
  wait_for_api_jq "/api/capabilities/native/runs?limit=500" \
    "any(.runs[]; .capability_name == \"cert-parallel\" and .origin == \"telegram\" and .run_id != \"$before_run_id\" and .status == \"succeeded\")" \
    "$WORKDIR/telegram-runs-after.json" 60 || fail "Telegram did not complete a CapSpec run"
  pass "Telegram chat invokes a native capability"
  curl -sS --max-time "$TIMEOUT" "$FIXTURE_BASE/cert/state" >"$WORKDIR/telegram-state-after-message.json"
  assert_jq "$WORKDIR/telegram-state-after-message.json" \
    'any(.telegramCalls[];
      (.method == "sendMessage" or .method == "sendMessageDraft" or .method == "sendRichMessage")
      and ((.body.text // .body.rich_message.markdown // "") | contains("CAPSPEC_CERT_OK telegram")))' \
    "Telegram receives the model response through the adapter"

  local approval_v1="$WORKDIR/sources/cert-telegram-approval-v1.captain"
  local approval_v2="$WORKDIR/sources/cert-telegram-approval-v2.captain"
  cat >"$approval_v1" <<'EOF'
format = 1
name = "cert-telegram-approval"
description = "Telegram approval baseline."
version = "1.0.0"
output = "{{steps.read.output}}"
[permissions]
tools = ["file_read"]
read_paths = ["cert-repo/**"]
[[steps]]
id = "read"
tool = "file_read"
with = { path = "cert-repo/README.md" }
EOF
  cat >"$approval_v2" <<'EOF'
format = 1
name = "cert-telegram-approval"
description = "Telegram exact-hash approval proposal."
version = "2.0.0"
output = "{{steps.write.output}}"
[permissions]
tools = ["file_write"]
write_paths = ["cert-work/**"]
[[steps]]
id = "write"
tool = "file_write"
with = { path = "cert-work/telegram-approved.txt", content = "approved but not invoked" }
EOF
  install_capability "$approval_v1" global "" telegram-approval-v1
  install_capability "$approval_v2" global "" telegram-approval-v2
  assert_jq "$WORKDIR/telegram-approval-v2-install-response.json" \
    '.human_action_required == true and .pending_hash != null' "Telegram proposal waits for a decision"
  local pending_hash
  pending_hash="$(jq -er '.pending_hash | select(type == "string" and length == 64)' \
    "$WORKDIR/telegram-approval-v2-install-response.json")" \
    || fail "Telegram proposal did not expose an exact pending hash"

  local callback=""
  local elapsed=0
  while [[ "$elapsed" -le 30 && -z "$callback" ]]; do
    curl -sS --max-time 3 "$FIXTURE_BASE/cert/state" >"$WORKDIR/telegram-pending-card-state.json"
    callback="$(telegram_markup_callback "$WORKDIR/telegram-pending-card-state.json")"
    [[ -n "$callback" ]] && break
    sleep 1
    elapsed=$((elapsed + 1))
  done
  [[ -n "$callback" ]] || fail "Telegram pending card did not expose an approve callback"
  pass "Telegram scanner publishes a durable pending card"
  local llm_before
  llm_before="$(jq '.openaiRequests | length' "$WORKDIR/telegram-pending-card-state.json")"
  jq -n --arg callback "$callback" \
    '{callback_query:{id:"cert-callback-1",from:{id:4242,is_bot:false,first_name:"Certification"},message:{message_id:1701,chat:{id:4242,type:"private"}},data:$callback}}' \
    >"$WORKDIR/telegram-callback-update.json"
  curl -sS --max-time "$TIMEOUT" -X POST -H 'Content-Type: application/json' \
    --data-binary @"$WORKDIR/telegram-callback-update.json" \
    "$FIXTURE_BASE/cert/telegram/push" >"$WORKDIR/telegram-callback-queued.json"
  wait_for_api_jq "/api/capabilities/native?scope=global" \
    "any(.capabilities[];
      .name == \"cert-telegram-approval\"
      and .active_hash == \"$pending_hash\"
      and .pending_hash == null
      and .human_action_required == false)" \
    "$WORKDIR/telegram-approval-ready.json" 30 || fail "Telegram approval callback was not applied"
  pass "Telegram callback approves the exact pending hash"

  elapsed=0
  while [[ "$elapsed" -le 30 ]]; do
    curl -sS --max-time 3 "$FIXTURE_BASE/cert/state" >"$WORKDIR/telegram-state-after-callback.json"
    if jq -e '
      any(.telegramCalls[]; .method == "answerCallbackQuery")
      and any(.telegramCalls[];
        .method == "editMessageText" or .method == "editMessageReplyMarkup")
    ' "$WORKDIR/telegram-state-after-callback.json" >/dev/null; then
      break
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  assert_jq "$WORKDIR/telegram-state-after-callback.json" \
    ".openaiRequests | length == $llm_before" "Telegram decision bypasses the LLM"
  assert_jq "$WORKDIR/telegram-state-after-callback.json" \
    'any(.telegramCalls[]; .method == "answerCallbackQuery") and any(.telegramCalls[]; .method == "editMessageText" or .method == "editMessageReplyMarkup")' \
    "Telegram acknowledges and closes the operator card"
}

certify_sigkill_recovery() {
  local request="$WORKDIR/crash-message-request.json"
  local response="$WORKDIR/crash-message-response.json"
  jq -n --arg message "[CAPSPEC-CERT:crash] Exercise abrupt-stop recovery." '{message:$message}' >"$request"
  curl -sS --max-time 90 -X POST -H "X-API-Key: $CERT_API_KEY" \
    -H 'Content-Type: application/json' --data-binary @"$request" \
    "$BASE/api/agents/$CAPTAIN_AGENT_ID/message" >"$response" 2>"$WORKDIR/crash-message-curl.stderr" &
  local request_pid=$!
  wait_for_latest_run_status "cert-crash" running "$WORKDIR/crash-running-runs.json" 30 \
    || fail "crash capability never reached running state"
  extract_latest_run "$WORKDIR/crash-running-runs.json" cert-crash "$WORKDIR/crash-running.json"
  pass "crash run is durably visible before SIGKILL"
  local run_id
  run_id="$(jq -r '.run_id' "$WORKDIR/crash-running.json")"

  kill -KILL "$DAEMON_PID"
  wait "$DAEMON_PID" >/dev/null 2>&1 || true
  DAEMON_PID=""
  wait "$request_pid" >/dev/null 2>&1 || true
  pass "daemon stopped by SIGKILL during a real primitive call"
  sleep 14
  start_primary_daemon

  wait_for_api_jq "/api/capabilities/native/runs/$run_id" \
    '.status == "waiting_decision" and any(.nodes[]; .status == "uncertain")' \
    "$WORKDIR/crash-after-restart.json" 45 || fail "manual mutation was not recovered as uncertain"
  pass "restart recovers the exact manual attempt as uncertain"
  local marker="$WORKSPACE/cert-work/crash-marker.txt"
  local observed=0
  if [[ -f "$marker" ]]; then
    observed="$(grep -c '^CAPSPEC_CRASH_ONCE$' "$marker" || true)"
  fi
  [[ "$observed" -le 1 ]] || fail "external crash effect occurred more than once before decision"

  local node_id tool_use_id attempt decision_payload
  node_id="$(jq -r '.nodes[] | select(.status == "uncertain") | .step_id' "$WORKDIR/crash-after-restart.json")"
  tool_use_id="$(jq -r '.nodes[] | select(.status == "uncertain") | .tool_use_id' "$WORKDIR/crash-after-restart.json")"
  attempt="$(jq -r '.nodes[] | select(.status == "uncertain") | .attempts' "$WORKDIR/crash-after-restart.json")"
  decision_payload="$WORKDIR/crash-decision-request.json"
  if [[ "$observed" == "1" ]]; then
    jq -n --arg node_id "$node_id" --arg tool_use_id "$tool_use_id" --argjson attempt "$attempt" \
      '{node_id:$node_id,expected_tool_use_id:$tool_use_id,expected_attempt:$attempt,decision:"confirm_succeeded",output:{observed:"CAPSPEC_CRASH_ONCE"}}' \
      >"$decision_payload"
  else
    jq -n --arg node_id "$node_id" --arg tool_use_id "$tool_use_id" --argjson attempt "$attempt" \
      '{node_id:$node_id,expected_tool_use_id:$tool_use_id,expected_attempt:$attempt,decision:"retry"}' \
      >"$decision_payload"
  fi
  http_request POST "/api/capabilities/native/runs/$run_id/decision" "$decision_payload" \
    "$WORKDIR/crash-decision-response.json"
  assert_status 200 "exact crash decision accepted" "$WORKDIR/crash-decision-response.json"
  wait_for_api_jq "/api/capabilities/native/runs/$run_id" '.status == "succeeded"' \
    "$WORKDIR/crash-succeeded.json" 60 || fail "decided crash run did not finish"
  pass "operator decision resumes and closes the same durable run"
  if [[ ! -f "$marker" ]]; then
    fail "crash marker is absent after successful recovery"
  fi
  observed="$(grep -c '^CAPSPEC_CRASH_ONCE$' "$marker" || true)"
  [[ "$observed" == "1" ]] || fail "crash recovery produced $observed external effects"
  pass "SIGKILL recovery produces exactly one external effect"
}
