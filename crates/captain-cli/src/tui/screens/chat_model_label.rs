//! Model label normalization for the chat screen and chat runners.

#[cfg(test)]
mod tests;

/// Strip the `?/?` placeholder that older Captain builds wrote when the
/// agent metadata fetch had not returned yet. Returning an empty label lets
/// the next live metadata fetch repopulate it naturally.
pub(crate) fn sanitize_model_label(label: &str) -> String {
    if label == "?/?" {
        String::new()
    } else {
        label.to_string()
    }
}

/// Build a model label from a provider/model pair returned by daemon metadata.
///
/// Returns `None` when both halves are the unknown sentinel so callers can keep
/// the previous good label instead of overwriting it with `?/?`.
pub(crate) fn compose_model_label(provider: &str, model: &str) -> Option<String> {
    if provider == "?" && model == "?" {
        return None;
    }
    Some(format!("{provider}/{model}"))
}

pub(crate) fn model_label_from_agent_metadata(body: &serde_json::Value) -> Option<String> {
    let provider = body["model_provider"]
        .as_str()
        .or_else(|| body["model"]["provider"].as_str())
        .unwrap_or("?");
    let model = body["model_name"]
        .as_str()
        .or_else(|| body["model"]["model"].as_str())
        .unwrap_or("?");
    compose_model_label(provider, model)
}
