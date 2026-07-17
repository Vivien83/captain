use crate::kernel_handle::KernelHandle;
use crate::llm_driver::{
    CacheHints, CompletionRequest, CompletionResponse, LlmDriver, StreamEvent,
};
use crate::tools::require_kernel;
use captain_types::message::Message;
use captain_types::tool::ToolDefinition;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CODEX_PENDING_LOGINS_KEY: &str = "__captain_codex_oauth_pending_v1";

fn codex_auth_path() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .map(|dir| dir.join("auth.json"))
}

fn extract_codex_token_from_auth(parsed: &serde_json::Value) -> Option<&str> {
    parsed
        .get("api_key")
        .or_else(|| parsed.get("token"))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("access_token")))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("id_token")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn pretty_json(value: &Value, context: &str) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("Failed to serialize {context}: {e}"))
}

fn codex_auth_missing_home_status() -> Value {
    serde_json::json!({
        "status": "missing",
        "driver_ready": false,
        "next_action": "Set CODEX_HOME or run codex_login_start from the chat."
    })
}

fn codex_auth_status_base(auth_path: &Path) -> Value {
    serde_json::json!({
        "auth_path": auth_path.display().to_string(),
        "driver_ready": false,
    })
}

fn serialize_codex_auth_status(out: Value) -> Result<String, String> {
    pretty_json(&out, "status")
}

fn render_codex_auth_file_missing(auth_path: &Path) -> Result<String, String> {
    let mut out = codex_auth_status_base(auth_path);
    let obj = out
        .as_object_mut()
        .ok_or_else(|| "internal JSON error".to_string())?;
    obj.insert("status".into(), serde_json::json!("missing"));
    obj.insert(
        "next_action".into(),
        serde_json::json!("Call codex_login_start, show the user verification_url + user_code, then call codex_login_poll."),
    );
    serialize_codex_auth_status(out)
}

fn render_malformed_codex_auth(auth_path: &Path, error: String) -> Result<String, String> {
    let mut out = codex_auth_status_base(auth_path);
    let obj = out
        .as_object_mut()
        .ok_or_else(|| "internal JSON error".to_string())?;
    obj.insert("status".into(), serde_json::json!("malformed"));
    obj.insert("error".into(), serde_json::json!(error));
    serialize_codex_auth_status(out)
}

fn render_codex_auth_from_parsed(auth_path: &Path, parsed: &Value) -> Result<String, String> {
    let mut out = codex_auth_status_base(auth_path);
    let obj = out
        .as_object_mut()
        .ok_or_else(|| "internal JSON error".to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if let Some(exp) = parsed.get("expires_at").and_then(|v| v.as_i64()) {
        obj.insert("expires_at".into(), serde_json::json!(exp));
        if now >= exp {
            obj.insert("status".into(), serde_json::json!("expired"));
            obj.insert(
                "next_action".into(),
                serde_json::json!("Call codex_login_start to renew the ChatGPT/Codex session."),
            );
            return serialize_codex_auth_status(out);
        }
    }

    let Some(token) = extract_codex_token_from_auth(parsed) else {
        obj.insert("status".into(), serde_json::json!("missing_token"));
        obj.insert(
            "next_action".into(),
            serde_json::json!("Call codex_login_start; auth.json exists but has no access token."),
        );
        return serialize_codex_auth_status(out);
    };

    obj.insert("status".into(), serde_json::json!("configured"));
    obj.insert("driver_ready".into(), serde_json::json!(true));
    obj.insert(
        "scopes".into(),
        serde_json::json!(crate::model_catalog::codex_token_scopes(token)),
    );
    obj.insert(
        "available_models".into(),
        serde_json::json!(codex_available_models_json()),
    );
    obj.insert(
        "tool_probe".into(),
        serde_json::json!({
            "status": "not_run",
            "tool": "codex_tool_probe",
            "next_action": "Run codex_tool_probe with the candidate model before promoting it as Captain's agent default."
        }),
    );
    obj.insert(
        "note".into(),
        serde_json::json!("Connector-scoped ChatGPT tokens are valid for the Codex Responses endpoint; do not reject them solely because api.responses.write is absent."),
    );
    serialize_codex_auth_status(out)
}

fn codex_available_models_json() -> Vec<Value> {
    crate::model_catalog::codex_model_choices()
        .into_iter()
        .map(|(id, label)| {
            serde_json::json!({
                "model": id,
                "display_name": label,
            })
        })
        .collect()
}

pub(crate) fn tool_codex_auth_status() -> Result<String, String> {
    let Some(auth_path) = codex_auth_path() else {
        return Ok(codex_auth_missing_home_status().to_string());
    };

    let content = match std::fs::read_to_string(&auth_path) {
        Ok(content) => content,
        Err(_) => return render_codex_auth_file_missing(&auth_path),
    };

    let parsed: Value = match serde_json::from_str(&content) {
        Ok(parsed) => parsed,
        Err(e) => return render_malformed_codex_auth(&auth_path, e.to_string()),
    };

    render_codex_auth_from_parsed(&auth_path, &parsed)
}

fn codex_probe_model_from_input(input: &serde_json::Value) -> String {
    let requested_model = input
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            crate::model_catalog::codex_model_choices()
                .into_iter()
                .next()
                .map(|(model, _)| model)
        })
        .unwrap_or_else(|| "gpt-5.5".to_string());
    normalize_codex_model(&requested_model)
}

fn normalize_codex_model(requested_model: &str) -> String {
    requested_model
        .strip_prefix("codex/")
        .unwrap_or(requested_model)
        .to_string()
}

fn missing_codex_probe_auth_response(model: &str) -> Result<String, String> {
    pretty_json(
        &serde_json::json!({
            "status": "missing_auth",
            "driver_ready": false,
            "tool_call_ok": false,
            "model": model,
            "next_action": "Run codex_login_start/codex_login_poll or `captain login codex`."
        }),
        "Codex probe",
    )
}

fn codex_probe_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: vec![Message::user(
            "Call the probe_pass tool exactly once with {\"ok\":true}. Do not answer in text.",
        )],
        tools: vec![codex_probe_tool_definition()],
        max_tokens: 512,
        temperature: 0.0,
        system: Some(
            "You are a Captain Codex diagnostics probe. You must call the provided function tool; do not describe the action."
                .to_string(),
        ),
        thinking: None,
        tool_choice: Some(serde_json::json!("required")),
        cache_hints: CacheHints::default(),
    }
}

fn codex_probe_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "probe_pass".to_string(),
        description: "Mark the Codex tool-calling probe as passed.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "ok": {"type": "boolean"}
            },
            "required": ["ok"]
        }),
    }
}

async fn stream_codex_probe(
    driver: crate::drivers::codex::CodexDriver,
    request: CompletionRequest,
) -> Result<CompletionResponse, crate::llm_driver::LlmError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let response = driver.stream(request, tx).await;
    let _ = drain.await;
    response
}

fn codex_probe_response_json(
    model: &str,
    response: Result<CompletionResponse, crate::llm_driver::LlmError>,
) -> Value {
    match response {
        Ok(response) => successful_codex_probe_json(model, response),
        Err(e) => serde_json::json!({
            "status": "error",
            "driver_ready": true,
            "tool_call_ok": false,
            "model": model,
            "error": e.to_string(),
            "next_action": "Do not promote this Codex model to Captain's agent default until the probe passes."
        }),
    }
}

fn successful_codex_probe_json(model: &str, response: CompletionResponse) -> Value {
    let assistant_text = response.text();
    let tool_call_ok = codex_probe_tool_call_ok(&response);
    serde_json::json!({
        "status": if tool_call_ok { "ok" } else { "no_tool_call" },
        "driver_ready": true,
        "tool_call_ok": tool_call_ok,
        "model": model,
        "tool_calls": codex_probe_tool_calls_json(&response),
        "assistant_text_preview": captain_types::truncate_str(&assistant_text, 240),
        "usage": {
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "cached_input_tokens": response.usage.cached_input_tokens,
        }
    })
}

fn codex_probe_tool_call_ok(response: &CompletionResponse) -> bool {
    response.tool_calls.iter().any(|call| {
        call.name == "probe_pass"
            && call
                .input
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
    })
}

fn codex_probe_tool_calls_json(response: &CompletionResponse) -> Vec<Value> {
    response
        .tool_calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "id": &call.id,
                "name": &call.name,
                "input": &call.input,
            })
        })
        .collect()
}

pub(crate) async fn tool_codex_tool_probe(input: &serde_json::Value) -> Result<String, String> {
    let model = codex_probe_model_from_input(input);
    let Some(token) = crate::model_catalog::read_codex_credential_with_refresh() else {
        return missing_codex_probe_auth_response(&model);
    };

    let driver = crate::drivers::codex::CodexDriver::new(
        token,
        captain_types::model_catalog::CODEX_BASE_URL.to_string(),
    );
    let response = stream_codex_probe(driver, codex_probe_request(&model)).await;
    pretty_json(&codex_probe_response_json(&model, response), "Codex probe")
}

pub(crate) async fn tool_codex_login_start(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let device = crate::codex_oauth::request_device_code()
        .await
        .map_err(|e| format!("Codex login start failed: {e}"))?;
    let login_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let expires_at = now + 15 * 60;
    let user_code = device.user_code.clone();
    let interval = device.interval;

    let mut pending = kh
        .memory_kv_recall(CODEX_PENDING_LOGINS_KEY)?
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    pending.insert(
        login_id.clone(),
        serde_json::json!({
            "login_id": login_id.clone(),
            "device_auth_id": device.device_auth_id,
            "user_code": user_code.clone(),
            "verification_url": crate::codex_oauth::CODEX_DEVICE_VERIFICATION_URL,
            "interval": interval,
            "created_at": now,
            "expires_at": expires_at
        }),
    );
    kh.memory_kv_store(CODEX_PENDING_LOGINS_KEY, serde_json::Value::Object(pending))?;

    serde_json::to_string_pretty(&serde_json::json!({
        "status": "user_action_required",
        "login_id": login_id,
        "verification_url": crate::codex_oauth::CODEX_DEVICE_VERIFICATION_URL,
        "user_code": user_code,
        "interval_seconds": interval,
        "expires_at": expires_at,
        "next_action": "Ask the user to open verification_url, enter user_code, then call codex_login_poll with login_id."
    }))
    .map_err(|e| format!("Failed to serialize login start: {e}"))
}

fn persist_codex_credentials(
    creds: &crate::codex_oauth::CodexCredentials,
) -> Result<String, String> {
    let auth_path = codex_auth_path().ok_or("Cannot resolve CODEX_HOME or home directory")?;
    let dir = auth_path
        .parent()
        .ok_or("Cannot resolve Codex auth directory")?;
    captain_types::durable_fs::create_dir_all(dir)
        .map_err(|e| format!("Create {}: {e}", dir.display()))?;
    let payload = serde_json::json!({
        "tokens": {
            "access_token": creds.access_token,
            "refresh_token": creds.refresh_token,
        },
        "api_key": creds.access_token,
        "expires_at": creds.expires_at,
        "last_refresh": creds.last_refresh,
        "auth_mode": creds.auth_mode,
        "source": creds.source,
    });
    let serialized = serde_json::to_string_pretty(&payload).unwrap_or_default();
    captain_types::durable_fs::atomic_write(&auth_path, serialized.as_bytes())
        .map_err(|e| format!("Persist {}: {e}", auth_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(auth_path.display().to_string())
}

struct PendingCodexLogin {
    entry: Value,
    device_auth_id: String,
    user_code: String,
}

fn load_pending_codex_logins(kh: &dyn KernelHandle) -> Result<Map<String, Value>, String> {
    Ok(kh
        .memory_kv_recall(CODEX_PENDING_LOGINS_KEY)?
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default())
}

fn store_pending_codex_logins(
    kh: &dyn KernelHandle,
    pending: Map<String, Value>,
) -> Result<(), String> {
    kh.memory_kv_store(CODEX_PENDING_LOGINS_KEY, Value::Object(pending))
}

fn codex_login_not_found_response() -> String {
    serde_json::json!({
        "status": "not_found",
        "next_action": "Call codex_login_start again."
    })
    .to_string()
}

fn codex_login_expired(entry: &Value, now: i64) -> bool {
    entry
        .get("expires_at")
        .and_then(|v| v.as_i64())
        .is_some_and(|exp| now >= exp)
}

fn expire_pending_codex_login(
    kh: &dyn KernelHandle,
    pending: &mut Map<String, Value>,
    login_id: &str,
) -> Result<String, String> {
    pending.remove(login_id);
    store_pending_codex_logins(kh, pending.clone())?;
    Ok(serde_json::json!({
        "status": "expired",
        "next_action": "Call codex_login_start again and ask the user to validate the new code."
    })
    .to_string())
}

fn pending_codex_login_from_entry(entry: Value) -> Result<PendingCodexLogin, String> {
    let device_auth_id = entry
        .get("device_auth_id")
        .and_then(|v| v.as_str())
        .ok_or("Pending Codex login is missing device_auth_id")?
        .to_string();
    let user_code = entry
        .get("user_code")
        .and_then(|v| v.as_str())
        .ok_or("Pending Codex login is missing user_code")?
        .to_string();

    Ok(PendingCodexLogin {
        entry,
        device_auth_id,
        user_code,
    })
}

fn render_pending_codex_login_poll(pending: &PendingCodexLogin) -> Result<String, String> {
    pretty_json(
        &serde_json::json!({
            "status": "pending",
            "verification_url": crate::codex_oauth::CODEX_DEVICE_VERIFICATION_URL,
            "user_code": pending.user_code,
            "interval_seconds": pending.entry.get("interval").and_then(|v| v.as_u64()).unwrap_or(5),
            "next_action": "Wait for the user to finish the browser/device-code step, then call codex_login_poll again."
        }),
        "pending status",
    )
}

fn authorized_codex_login_response(auth_path: String) -> Value {
    serde_json::json!({
        "status": "authorized",
        "auth_path": auth_path,
        "driver_ready": true,
        "next_action": "Call model_switch_plan, ask the user for new_session or compact_session if needed, then model_switch_apply."
    })
}

fn maybe_apply_codex_model_switch(
    result: &mut Value,
    input: &serde_json::Value,
    kh: &dyn KernelHandle,
    caller_agent_id: Option<&str>,
) -> Result<(), String> {
    if !input
        .get("apply_model_switch")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(());
    }

    let agent_id = caller_agent_id.ok_or("Cannot apply model switch without an agent context")?;
    let model = input["model"]
        .as_str()
        .ok_or("Missing 'model' when apply_model_switch=true")?;
    let strategy = input["session_strategy"]
        .as_str()
        .ok_or("Missing 'session_strategy' when apply_model_switch=true")?;
    let provider = input
        .get("provider")
        .and_then(|v| v.as_str())
        .or(Some("codex"));
    let switch = kh.model_switch_apply(agent_id, model, provider, strategy)?;
    result["model_switch"] = switch;
    result["next_action"] =
        serde_json::json!("Model switch applied. Continue in the new Captain session.");
    Ok(())
}

pub(crate) async fn tool_codex_login_poll(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let login_id = input["login_id"]
        .as_str()
        .ok_or("Missing 'login_id' parameter")?;
    let mut pending = load_pending_codex_logins(kh.as_ref())?;
    let Some(entry) = pending.get(login_id).cloned() else {
        return Ok(codex_login_not_found_response());
    };

    let now = chrono::Utc::now().timestamp();
    if codex_login_expired(&entry, now) {
        return expire_pending_codex_login(kh.as_ref(), &mut pending, login_id);
    }

    let pending_login = pending_codex_login_from_entry(entry)?;

    match crate::codex_oauth::poll_authorization(
        &pending_login.device_auth_id,
        &pending_login.user_code,
    )
    .await
    .map_err(|e| format!("Codex login poll failed: {e}"))?
    {
        crate::codex_oauth::PollOutcome::Pending => render_pending_codex_login_poll(&pending_login),
        crate::codex_oauth::PollOutcome::Authorized {
            authorization_code,
            code_verifier,
        } => {
            let creds = crate::codex_oauth::exchange_code(&authorization_code, &code_verifier)
                .await
                .map_err(|e| format!("Codex token exchange failed: {e}"))?;
            let auth_path = persist_codex_credentials(&creds)?;
            pending.remove(login_id);
            store_pending_codex_logins(kh.as_ref(), pending)?;

            let mut result = authorized_codex_login_response(auth_path);
            maybe_apply_codex_model_switch(&mut result, input, kh.as_ref(), caller_agent_id)?;
            pretty_json(&result, "authorized status")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{ContentBlock, StopReason, TokenUsage};
    use captain_types::tool::ToolCall;
    use serde_json::json;

    #[test]
    fn codex_probe_model_strips_provider_prefix_and_trims() {
        assert_eq!(
            codex_probe_model_from_input(&json!({"model": "  codex/gpt-5.3-codex  "})),
            "gpt-5.3-codex"
        );
        assert_eq!(normalize_codex_model("gpt-5.5"), "gpt-5.5");
    }

    #[test]
    fn successful_codex_probe_json_marks_ok_tool_call() {
        let response = CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "ignored text".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::ToolUse,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "probe_pass".to_string(),
                input: json!({"ok": true}),
            }],
            usage: TokenUsage {
                input_tokens: 11,
                output_tokens: 22,
                cached_input_tokens: 3,
                ..TokenUsage::default()
            },
        };

        let out = successful_codex_probe_json("gpt-5.5", response);

        assert_eq!(out["status"], "ok");
        assert_eq!(out["tool_call_ok"], true);
        assert_eq!(out["model"], "gpt-5.5");
        assert_eq!(out["usage"]["cached_input_tokens"], 3);
        assert_eq!(out["tool_calls"][0]["name"], "probe_pass");
    }

    #[test]
    fn codex_login_expiration_uses_expires_at() {
        assert!(codex_login_expired(&json!({"expires_at": 10}), 10));
        assert!(!codex_login_expired(&json!({"expires_at": 11}), 10));
        assert!(!codex_login_expired(&json!({}), 10));
    }

    #[test]
    fn pending_codex_login_poll_defaults_interval() {
        let pending = PendingCodexLogin {
            entry: json!({}),
            device_auth_id: "device".to_string(),
            user_code: "ABCD-EFGH".to_string(),
        };

        let out: Value =
            serde_json::from_str(&render_pending_codex_login_poll(&pending).unwrap()).unwrap();

        assert_eq!(out["status"], "pending");
        assert_eq!(out["user_code"], "ABCD-EFGH");
        assert_eq!(out["interval_seconds"], 5);
    }
}
