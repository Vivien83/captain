//! WebRTC relay for live Captain voice calls.

use crate::routes::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use captain_types::config::{KernelConfig, VoiceCallConfig};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const OPENAI_REALTIME_CALLS_URL: &str = "https://api.openai.com/v1/realtime/calls";
const MAX_SDP_BYTES: usize = 256 * 1024;

/// GET /api/realtime/calls/config — expose non-secret browser call settings.
pub async fn get_call_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = &state.kernel.config.voice_call;
    Json(json!({
        "enabled": cfg.enabled,
        "provider": cfg.provider,
        "model": cfg.model,
        "voice": cfg.voice,
        "enable_agent_tool": cfg.enable_agent_tool,
        "auto_end_silence_secs": cfg.auto_end_silence_secs,
        "auto_end_inactive_secs": cfg.auto_end_inactive_secs
    }))
}

/// POST /api/realtime/calls — exchange browser SDP for an OpenAI Realtime SDP answer.
pub async fn create_call(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> axum::response::Response {
    if body.is_empty() {
        return text_error(StatusCode::BAD_REQUEST, "missing SDP offer");
    }
    if body.len() > MAX_SDP_BYTES {
        return text_error(StatusCode::PAYLOAD_TOO_LARGE, "SDP offer too large");
    }

    let offer_sdp = match std::str::from_utf8(&body) {
        Ok(s) if s.contains("v=0") => s,
        Ok(_) => return text_error(StatusCode::BAD_REQUEST, "invalid SDP offer"),
        Err(_) => return text_error(StatusCode::BAD_REQUEST, "SDP offer must be UTF-8 text"),
    };

    let cfg = &state.kernel.config.voice_call;
    if !cfg.enabled {
        return text_error(StatusCode::FORBIDDEN, "live voice calls are disabled");
    }
    if !cfg.provider.eq_ignore_ascii_case("openai") {
        return text_error(
            StatusCode::BAD_REQUEST,
            "voice_call.provider must be 'openai' for live WebRTC calls",
        );
    }

    let api_key = match resolve_openai_api_key(&state.kernel.config) {
        Ok(key) => key,
        Err(e) => return text_error(StatusCode::PRECONDITION_FAILED, &e),
    };

    let session_config = build_session_config(cfg);
    let safety_id = safety_identifier(&state.kernel.instance_id);
    let form = reqwest::multipart::Form::new()
        .text("sdp", offer_sdp.to_string())
        .text("session", session_config.to_string());

    let response = match reqwest::Client::new()
        .post(OPENAI_REALTIME_CALLS_URL)
        .bearer_auth(api_key)
        .header("OpenAI-Safety-Identifier", safety_id)
        .multipart(form)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!(error = %e, "OpenAI Realtime call setup failed");
            return text_error(StatusCode::BAD_GATEWAY, "OpenAI Realtime call setup failed");
        }
    };

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return text_error(status, &sanitize_upstream_error(&body));
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/sdp")],
        body,
    )
        .into_response()
}

fn resolve_openai_api_key(config: &KernelConfig) -> Result<String, String> {
    let env_var = config
        .voice_call
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| config.resolve_api_key_env("openai"));

    std::env::var(&env_var)
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{env_var} is not set for live voice calls"))
}

fn build_session_config(cfg: &VoiceCallConfig) -> serde_json::Value {
    let mut session = json!({
        "type": "realtime",
        "model": cfg.model,
        "instructions": cfg.instructions,
        "audio": {
            "input": {
                "turn_detection": {
                    "type": "server_vad",
                    "threshold": 0.5,
                    "prefix_padding_ms": 300,
                    "silence_duration_ms": 700,
                    "create_response": true,
                    "interrupt_response": true
                }
            },
            "output": {
                "voice": cfg.voice
            }
        }
    });

    if cfg.enable_agent_tool {
        session["tools"] = json!([
            {
                "type": "function",
                "name": "captain_message",
                "description": "Route the caller's substantive voice turn to the real Captain agent. Use this for questions, commands, workflow requests, preferences, follow-ups, and anything requiring Captain's memory, tools, reasoning, or current session context. The realtime model is only Captain's voice interface; Captain itself must answer through this tool.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "A concise, self-contained instruction for Captain."
                        }
                    },
                    "required": ["message"],
                    "additionalProperties": false
                }
            },
            {
                "type": "function",
                "name": "captain_activity_summary",
                "description": "Fetch a concise summary of recent Captain voice-call activity only when the caller asks what happened, what Captain is doing, or asks for details/status. Do not call this continuously.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Maximum recent events to summarize, default 8."
                        }
                    },
                    "additionalProperties": false
                }
            }
        ]);
        session["tool_choice"] = json!("required");
    }

    session
}

fn safety_identifier(instance_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"captain-live-call:");
    hasher.update(instance_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn sanitize_upstream_error(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "OpenAI Realtime call setup failed".to_string();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(message) = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return message.to_string();
        }
    }
    trimmed.chars().take(1000).collect()
}

fn text_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_config_defaults_to_realtime_voice_and_agent_tool() {
        let cfg = VoiceCallConfig::default();
        let value = build_session_config(&cfg);
        assert_eq!(value["type"], "realtime");
        assert_eq!(value["model"], "gpt-realtime-2");
        assert_eq!(value["audio"]["output"]["voice"], "marin");
        assert_eq!(
            value["audio"]["input"]["turn_detection"]["type"],
            "server_vad"
        );
        assert_eq!(value["tools"][0]["name"], "captain_message");
        assert_eq!(value["tools"][1]["name"], "captain_activity_summary");
        assert_eq!(value["tool_choice"], "required");
    }

    #[test]
    fn safety_identifier_is_stable_and_does_not_expose_instance_id() {
        let id = safety_identifier("instance-secret");
        assert_eq!(id, safety_identifier("instance-secret"));
        assert_eq!(id.len(), 64);
        assert!(!id.contains("instance-secret"));
    }

    #[test]
    fn upstream_json_error_returns_message_only() {
        let body = r#"{"error":{"message":"bad realtime model","code":"x"}}"#;
        assert_eq!(sanitize_upstream_error(body), "bad realtime model");
    }
}
