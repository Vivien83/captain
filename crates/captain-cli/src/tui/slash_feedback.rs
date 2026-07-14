pub(crate) struct SlashFeedback<'a> {
    pub value: &'static str,
    pub note: &'a str,
}

pub(crate) fn feedback_for<'a>(command: &str, args: &'a str) -> Option<SlashFeedback<'a>> {
    let value = match command {
        "/like" => "up",
        "/dislike" => "down",
        _ => return None,
    };
    Some(SlashFeedback {
        value,
        note: args.trim(),
    })
}

pub(crate) fn response_preview(text: &str) -> String {
    text.chars().take(120).collect()
}

pub(crate) fn feedback_payload(
    value: &str,
    note: &str,
    preview: &str,
    timestamp_secs: u64,
) -> serde_json::Value {
    serde_json::json!({
        "type": "thumbs",
        "value": value,
        "note": note,
        "preview": preview,
        "ts": timestamp_secs,
    })
}

pub(crate) fn feedback_requires_daemon_message() -> &'static str {
    "Le feedback nécessite le mode daemon avec un agent actif."
}

pub(crate) fn feedback_saved_message(value: &str) -> String {
    format!("{} feedback enregistré.", feedback_icon(value))
}

pub(crate) fn feedback_http_error_message(status: impl std::fmt::Display) -> String {
    format!("Feedback échoué: HTTP {status}")
}

pub(crate) fn feedback_error_message(error: impl std::fmt::Display) -> String {
    format!("Feedback échoué: {error}")
}

fn feedback_icon(value: &str) -> &'static str {
    if value == "up" {
        "👍"
    } else {
        "👎"
    }
}

#[cfg(test)]
#[path = "slash_feedback/tests.rs"]
mod tests;
