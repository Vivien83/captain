use super::status_health::print_verbose_runtime_health;
use crate::{truncate_display, ui};

pub(super) fn print_verbose_runtime(body: &serde_json::Value) {
    ui::blank();
    ui::section("Runtime");
    ui::kv("Version", body["version"].as_str().unwrap_or("?"));
    ui::kv("Timezone", body["timezone"].as_str().unwrap_or("?"));
    ui::kv("Log level", body["log_level"].as_str().unwrap_or("?"));
    ui::kv(
        "Auth",
        if body["auth_enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        },
    );
    ui::kv(
        "Network",
        if body["network_enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        },
    );
    ui::kv(
        "Fallbacks",
        &body["fallback_provider_count"]
            .as_u64()
            .unwrap_or(0)
            .to_string(),
    );
    print_verbose_runtime_health(body);
    print_verbose_channels(body);
    print_verbose_agent_api(body);
    print_verbose_consciousness(body);
    print_verbose_media(body);
    print_verbose_tts(body);
    print_verbose_native_voice(body);
    print_verbose_native_embeddings(body);
}

fn print_verbose_channels(body: &serde_json::Value) {
    ui::blank();
    ui::section("Channels");
    let channels_status = &body["channels"];
    let configured = channels_status["configured_count"]
        .as_u64()
        .unwrap_or_else(|| body["channel_configured_count"].as_u64().unwrap_or(0));
    let total = channels_status["total"]
        .as_u64()
        .unwrap_or_else(|| body["channel_total"].as_u64().unwrap_or(0));
    let ready = channels_status["ready_count"]
        .as_u64()
        .unwrap_or(configured);
    ui::kv("Configured", &format!("{configured}/{total}"));
    ui::kv("Ready", &format!("{ready}/{total}"));
    if let Some(channels) = channels_status["configured"]
        .as_array()
        .or_else(|| body["configured_channels"].as_array())
    {
        let names = channels
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if !names.is_empty() {
            ui::kv("Active", &names);
        }
    }
    if let Some(locked) = channels_status["locked"].as_array() {
        let names = locked
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if !names.is_empty() {
            ui::kv_warn("Locked", &names);
        }
    }
    print_verbose_channel_inbound_queue(channels_status);
}

fn print_verbose_channel_inbound_queue(channels_status: &serde_json::Value) {
    let queue = &channels_status["inbound_queue"];
    let Some(summary) = inbound_queue_summary(queue) else {
        return;
    };

    if inbound_queue_needs_attention(queue) {
        ui::kv_warn("Inbound queue", &summary);
    } else {
        ui::kv("Inbound queue", &summary);
    }
}

fn inbound_queue_summary(queue: &serde_json::Value) -> Option<String> {
    if queue.is_null() {
        return None;
    }

    let bridge = match queue["bridge_running"].as_bool() {
        Some(true) => "running",
        Some(false) => "stopped",
        None => "unknown",
    };
    Some(format!(
        "{bridge}, {} active, {} pending sessions, {} queued, {} retrying, {} dead letters, {} interjected",
        queue["active_sessions"].as_u64().unwrap_or(0),
        queue["pending_sessions"].as_u64().unwrap_or(0),
        queue["pending_messages"].as_u64().unwrap_or(0),
        queue["inflight_messages"].as_u64().unwrap_or(0),
        queue["dead_letter_messages"].as_u64().unwrap_or(0),
        queue["interjected_messages"].as_u64().unwrap_or(0),
    ))
}

fn inbound_queue_needs_attention(queue: &serde_json::Value) -> bool {
    queue["bridge_running"].as_bool() == Some(false)
        || queue["pending_messages"].as_u64().unwrap_or(0) > 0
        || queue["inflight_messages"].as_u64().unwrap_or(0) > 0
        || queue["dead_letter_messages"].as_u64().unwrap_or(0) > 0
}

fn print_verbose_consciousness(body: &serde_json::Value) {
    let consciousness = &body["consciousness"];
    let Some(state) = consciousness["state"].as_str() else {
        return;
    };

    ui::blank();
    ui::section("Consciousness");
    if state == "steady" {
        ui::kv_ok("State", state);
    } else {
        ui::kv_warn("State", state);
    }
    ui::kv(
        "Confidence",
        &format!("{:.2}", consciousness["confidence"].as_f64().unwrap_or(0.0)),
    );
    ui::kv(
        "Error rate",
        &format!("{:.2}", consciousness["error_rate"].as_f64().unwrap_or(0.0)),
    );
    ui::kv(
        "Queue",
        &consciousness["queued_thoughts"]
            .as_u64()
            .unwrap_or(0)
            .to_string(),
    );
    ui::kv(
        "Goals",
        &format!(
            "{} active, {} escalated",
            consciousness["active_goals"].as_u64().unwrap_or(0),
            consciousness["escalated_goals"].as_u64().unwrap_or(0)
        ),
    );
    let projects = &consciousness["projects"];
    let project_attention = projects["attention"].as_u64().unwrap_or(0);
    if project_attention > 0 {
        ui::kv_warn(
            "Projects",
            &format!(
                "{} attention, {} waiting, {} pending tools, {} denied ({} repeated), {} resume-ready, {} stale",
                project_attention,
                projects["waiting_for_user"].as_u64().unwrap_or(0),
                projects["tool_request_pending"].as_u64().unwrap_or(0),
                projects["tool_request_denied"].as_u64().unwrap_or(0),
                projects["repeated_tool_denials"].as_u64().unwrap_or(0),
                projects["resume_ready"].as_u64().unwrap_or(0),
                projects["stale_active"].as_u64().unwrap_or(0)
            ),
        );
    }
    ui::kv(
        "Supervisor",
        &format!(
            "{} failures, {} panics, {} restarts since start",
            consciousness["supervisor"]["failure_count"]
                .as_u64()
                .unwrap_or(0),
            consciousness["supervisor"]["panic_count"]
                .as_u64()
                .unwrap_or(0),
            consciousness["supervisor"]["restart_count"]
                .as_u64()
                .unwrap_or(0)
        ),
    );
    if let Some(signals) = string_list(&consciousness["signals"]).filter(|value| !value.is_empty())
    {
        ui::kv("Signals", &signals);
    }
    if let Some(actions) =
        string_list(&consciousness["operator_actions"]).filter(|value| !value.is_empty())
    {
        ui::kv("Actions", &actions);
    }
}

fn print_verbose_agent_api(body: &serde_json::Value) {
    let queue = &body["agent_api"]["egress_queue"];
    if queue.is_null() {
        return;
    }

    let pending = queue["pending"].as_u64().unwrap_or(0);
    let due = queue["due"].as_u64().unwrap_or(0);
    let dead = queue["dead_letters"].as_u64().unwrap_or(0);
    if pending == 0 && due == 0 && dead == 0 && queue["readable"].as_bool() != Some(false) {
        return;
    }

    ui::blank();
    ui::section("Agent API");
    if queue["readable"].as_bool() == Some(false) {
        ui::kv_warn("Egress queue", "unavailable");
        if let Some(issue) = queue["issue"].as_str() {
            ui::kv("Issue", issue);
        }
        return;
    }

    ui::kv_warn(
        "Egress queue",
        &format!(
            "{pending} pending, {due} due, {dead} dead letters, {} agents",
            queue["agents_with_queue"].as_u64().unwrap_or(0)
        ),
    );
    if let Some(errors) = queue["last_errors"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        for error in errors.iter().take(5) {
            let agent_id = truncate_display(error["agent_id"].as_str().unwrap_or("?"), 12);
            let event = error["event"].as_str().unwrap_or("callback");
            let kind = error["error_kind"].as_str().unwrap_or("callback_failed");
            let preview = truncate_display(error["error_preview"].as_str().unwrap_or(""), 96);
            println!("    {agent_id} -- {event} -- {kind} -- {preview}");
        }
    }
}

fn print_verbose_media(body: &serde_json::Value) {
    ui::blank();
    ui::section("Media");
    ui::kv(
        "Images",
        enabled_disabled(
            body["media"]["image_description"]
                .as_bool()
                .unwrap_or(false),
        ),
    );
    ui::kv(
        "Audio",
        enabled_disabled(
            body["media"]["audio_transcription"]
                .as_bool()
                .unwrap_or(false),
        ),
    );
    ui::kv(
        "Video",
        enabled_disabled(
            body["media"]["video_description"]
                .as_bool()
                .unwrap_or(false),
        ),
    );
    ui::kv(
        "Image prov.",
        body["media"]["image_provider"].as_str().unwrap_or("auto"),
    );
    ui::kv(
        "Audio prov.",
        body["media"]["audio_provider"].as_str().unwrap_or("auto"),
    );
    ui::kv(
        "Audio effective",
        body["media"]["audio_effective_provider"]
            .as_str()
            .unwrap_or("auto"),
    );
}

fn print_verbose_tts(body: &serde_json::Value) {
    ui::blank();
    ui::section("TTS");
    ui::kv(
        "Enabled",
        if body["tts"]["enabled"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        },
    );
    ui::kv("Provider", body["tts"]["provider"].as_str().unwrap_or("?"));
    ui::kv("Voice", body["tts"]["voice"].as_str().unwrap_or("?"));
}

fn print_verbose_native_voice(body: &serde_json::Value) {
    let native_voice = &body["native_voice"];
    ui::blank();
    ui::section("Native Voice");
    ui::kv(
        "STT",
        if native_voice["stt_ready"].as_bool().unwrap_or(false) {
            "ready"
        } else {
            "pending"
        },
    );
    ui::kv(
        "TTS",
        if native_voice["tts_ready"].as_bool().unwrap_or(false) {
            native_voice["tts_engine"].as_str().unwrap_or("ready")
        } else {
            "pending"
        },
    );
    ui::kv(
        "Whisper bin",
        native_voice["whisper_binary"].as_str().unwrap_or("-"),
    );
    ui::kv(
        "Whisper model",
        native_voice["whisper_model"].as_str().unwrap_or("-"),
    );
    ui::kv(
        "Piper",
        if native_voice["piper_ready"].as_bool().unwrap_or(false) {
            "ready"
        } else {
            "pending"
        },
    );
    ui::kv(
        "Kokoro",
        if native_voice["kokoro_ready"].as_bool().unwrap_or(false) {
            "ready"
        } else {
            "pending"
        },
    );
}

fn print_verbose_native_embeddings(body: &serde_json::Value) {
    let native_embeddings = &body["native_embeddings"];
    ui::blank();
    ui::section("Native Embeddings");
    ui::kv(
        "Status",
        if native_embeddings["ready"].as_bool().unwrap_or(false) {
            "ready"
        } else {
            "pending"
        },
    );
    ui::kv(
        "Runtime",
        native_embeddings["runtime"]
            .as_str()
            .unwrap_or("onnxruntime"),
    );
    ui::kv(
        "Library",
        native_embeddings["library"].as_str().unwrap_or("-"),
    );
    ui::kv(
        "ORT_DYLIB_PATH",
        native_embeddings["ort_dylib_path"].as_str().unwrap_or("-"),
    );
}

fn enabled_disabled(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

fn string_list(value: &serde_json::Value) -> Option<String> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_verbose_inbound_queue_summary_includes_operator_counters() {
        let queue = serde_json::json!({
            "bridge_running": true,
            "active_sessions": 1,
            "pending_sessions": 2,
            "pending_messages": 3,
            "inflight_messages": 4,
            "dead_letter_messages": 5,
            "interjected_messages": 6,
        });

        assert_eq!(
            inbound_queue_summary(&queue),
            Some(
                "running, 1 active, 2 pending sessions, 3 queued, 4 retrying, 5 dead letters, 6 interjected"
                    .to_string()
            )
        );
        assert!(inbound_queue_needs_attention(&queue));
    }

    #[test]
    fn status_verbose_inbound_queue_summary_flags_stopped_bridge() {
        let queue = serde_json::json!({
            "bridge_running": false,
            "active_sessions": 0,
            "pending_sessions": 0,
            "pending_messages": 0,
            "inflight_messages": 0,
            "dead_letter_messages": 0,
            "interjected_messages": 0,
        });

        assert_eq!(
            inbound_queue_summary(&queue),
            Some(
                "stopped, 0 active, 0 pending sessions, 0 queued, 0 retrying, 0 dead letters, 0 interjected"
                    .to_string()
            )
        );
        assert!(inbound_queue_needs_attention(&queue));
    }

    #[test]
    fn status_verbose_inbound_queue_summary_ignores_absent_queue() {
        assert_eq!(inbound_queue_summary(&serde_json::Value::Null), None);
        assert!(!inbound_queue_needs_attention(&serde_json::Value::Null));
    }
}
