use crate::truncate_display;

pub(super) const PROJECT_ATTENTION_VISIBLE_LIMIT: usize = 8;

pub(super) fn project_attention_action_hint(item: &serde_json::Value) -> Option<String> {
    let action = item["actions"].as_array()?.first()?;
    let method = action["method"].as_str()?.trim();
    let path = action["path"].as_str()?.trim();
    if method.is_empty() || path.is_empty() {
        return None;
    }
    let label = action["label"].as_str().unwrap_or("action").trim();
    Some(format!("{label} {method} {path}"))
}

pub(super) fn project_attention_action_body_hint(item: &serde_json::Value) -> Option<String> {
    let action = item["actions"].as_array()?.first()?;
    let body = action.get("body_hint")?;
    if body.is_null() {
        return None;
    }
    let body = serde_json::to_string(body).ok()?;
    Some(truncate_display(&body, 180))
}

pub(super) fn project_attention_action_reason_hint(item: &serde_json::Value) -> Option<String> {
    let action = item["actions"].as_array()?.first()?;
    let reason = action["reason"]
        .as_str()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())?;
    Some(truncate_display(reason, 160))
}

pub(super) fn project_attention_hidden_count(
    item_count: usize,
    attention_count: Option<u64>,
) -> usize {
    let visible = item_count.min(PROJECT_ATTENTION_VISIBLE_LIMIT);
    let total = attention_count
        .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        .unwrap_or(item_count);
    total.saturating_sub(visible)
}

pub(super) fn project_attention_question_hint(item: &serde_json::Value) -> Option<String> {
    let question = item.get("first_pending_question")?;
    let text = question["question"]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    let mut hint = if let Some(ask_id) = question["ask_id"]
        .as_str()
        .map(str::trim)
        .filter(|ask_id| !ask_id.is_empty())
    {
        format!(
            "[{}] {}",
            truncate_display(ask_id, 8),
            truncate_display(text, 140)
        )
    } else {
        truncate_display(text, 150)
    };
    let options = question["options"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|option| !option.is_empty())
                .take(4)
                .enumerate()
                .map(|(idx, option)| format!("{}. {}", idx + 1, truncate_display(option, 42)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !options.is_empty() {
        hint.push_str(" | options: ");
        hint.push_str(&options.join(" / "));
    }
    Some(truncate_display(&hint, 220))
}

pub(super) fn project_attention_tool_request_hint(item: &serde_json::Value) -> Option<String> {
    let (fallback_status, request) = item
        .get("pending_tool_request")
        .filter(|value| !value.is_null())
        .map(|request| ("pending", request))
        .or_else(|| {
            item.get("denied_tool_request")
                .filter(|value| !value.is_null())
                .map(|request| ("denied", request))
        })?;
    let status = request["status"]
        .as_str()
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .unwrap_or(fallback_status);
    let phase = request["phase"]
        .as_str()
        .map(str::trim)
        .filter(|phase| !phase.is_empty())
        .unwrap_or("unknown");
    let tools = tool_request_tools_label(request).unwrap_or_else(|| "unknown tools".to_string());
    let mut parts = vec![status.to_string(), format!("phase {phase}"), tools];
    if request["repeat_of_denied_tool_request"]
        .as_bool()
        .unwrap_or(false)
    {
        parts.push("repeated denied request".to_string());
    }
    if let Some(reason) = tool_request_reason_label(request) {
        parts.push(reason);
    }
    Some(truncate_display(&parts.join(" -- "), 240))
}

pub(super) fn project_attention_worker_hint(item: &serde_json::Value) -> Option<String> {
    let workers = item.get("workers")?;
    let total = workers["total"].as_u64().unwrap_or(0);
    let progress = item["progress"].as_u64().unwrap_or(0);
    if total == 0 && progress == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if progress > 0 {
        parts.push(format!("progress {progress}%"));
    }
    if total > 0 {
        let by_status = workers["by_status"]
            .as_object()
            .map(|statuses| {
                statuses
                    .iter()
                    .filter_map(|(status, count)| {
                        count.as_u64().map(|count| format!("{status} {count}"))
                    })
                    .take(5)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if by_status.is_empty() {
            parts.push(format!("workers {total}"));
        } else {
            parts.push(format!("workers {total} ({})", by_status.join(", ")));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(truncate_display(&parts.join(" -- "), 180))
    }
}

pub(super) fn project_attention_last_event_hint(item: &serde_json::Value) -> Option<String> {
    let event = item.get("last_event")?;
    let title = event["title"]
        .as_str()
        .or_else(|| event["kind"].as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())?;
    let title = truncate_display(title, 72);
    let mut parts = Vec::new();
    let phase = event["phase"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(phase) = phase {
        parts.push(format!("phase {phase}"));
    }
    let status = event["status"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(status) = status {
        parts.push(status.to_string());
    }
    if parts.is_empty() {
        Some(title)
    } else {
        Some(format!("{title} -- {}", parts.join(" -- ")))
    }
}

fn tool_request_tools_label(request: &serde_json::Value) -> Option<String> {
    let tools = request["tools"].as_array()?;
    let names = tools
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .take(5)
        .map(|tool| truncate_display(tool, 40))
        .collect::<Vec<_>>();
    if names.is_empty() {
        None
    } else {
        Some(names.join(", "))
    }
}

fn tool_request_reason_label(request: &serde_json::Value) -> Option<String> {
    let reason = request["decision_reason"]
        .as_str()
        .or_else(|| request["reason"].as_str())
        .or_else(|| request["previous_decision_reason"].as_str())?
        .trim();
    if reason.is_empty() {
        None
    } else {
        Some(truncate_display(reason, 110))
    }
}
