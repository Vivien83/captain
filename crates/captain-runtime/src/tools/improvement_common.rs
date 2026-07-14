use super::security::ensure_no_secret_literal;

fn normalize_choice(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

pub(crate) fn enum_field(
    input: &serde_json::Value,
    field: &str,
    allowed: &[&str],
    required: bool,
    default: Option<&str>,
) -> Result<Option<String>, String> {
    match input.get(field) {
        Some(v) if v.is_null() => Ok(default.map(ToString::to_string)),
        Some(v) => {
            let raw = v
                .as_str()
                .ok_or_else(|| format!("Invalid {field}: expected string"))?;
            let normalized = normalize_choice(raw);
            if allowed.contains(&normalized.as_str()) {
                Ok(Some(normalized))
            } else {
                Err(format!(
                    "Invalid {field}: expected one of {}",
                    allowed.join(", ")
                ))
            }
        }
        None if required => default
            .map(|d| Some(d.to_string()))
            .ok_or_else(|| format!("Missing '{field}'")),
        None => Ok(default.map(ToString::to_string)),
    }
}

pub(crate) fn text_field(
    input: &serde_json::Value,
    field: &str,
    required: bool,
    max_bytes: usize,
    tool_name: &str,
) -> Result<Option<String>, String> {
    match input.get(field) {
        Some(v) if v.is_null() => Ok(None),
        Some(v) => {
            let raw = v
                .as_str()
                .ok_or_else(|| format!("Invalid {field}: expected string"))?;
            let trimmed = raw.trim();
            if required && trimmed.is_empty() {
                return Err(format!("Missing '{field}'"));
            }
            ensure_no_secret_literal(tool_name, field, trimmed)?;
            Ok(Some(
                captain_types::truncate_str(trimmed, max_bytes).to_string(),
            ))
        }
        None if required => Err(format!("Missing '{field}'")),
        None => Ok(None),
    }
}

pub(crate) fn public_safe_text_field(
    input: &serde_json::Value,
    field: &str,
    required: bool,
    max_bytes: usize,
    tool_name: &str,
) -> Result<Option<String>, String> {
    text_field(input, field, required, max_bytes, tool_name)
        .map(|value| value.map(|text| redact_local_paths(&text)))
}

pub(crate) fn block_agent_positive_skill_decision(
    tool_name: &str,
    _caller_agent_id: Option<&str>,
) -> Result<(), String> {
    Err(format!(
        "{tool_name} approve=true requires explicit human/API/channel approval after external validation; tool calls may only use approve=false."
    ))
}

pub(crate) fn review_id_field(
    input: &serde_json::Value,
    field: &str,
    tool_name: &str,
) -> Result<String, String> {
    let id = text_field(input, field, true, 80, tool_name)?.ok_or("Missing 'id' parameter")?;
    if id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        Ok(id)
    } else {
        Err(format!(
            "Invalid {field}: expected a review id or id prefix"
        ))
    }
}

pub(crate) fn resolve_json_id_prefix(
    items: &serde_json::Value,
    id_prefix: &str,
    label: &str,
) -> Result<String, String> {
    let items = items
        .as_array()
        .ok_or_else(|| format!("{label} registry is corrupted: expected JSON array"))?;
    let matches: Vec<&str> = items
        .iter()
        .filter_map(|item| item["id"].as_str())
        .filter(|id| id.starts_with(id_prefix))
        .collect();
    match matches.as_slice() {
        [] => Err(format!("{label} id not found")),
        [id] => Ok((*id).to_string()),
        _ => Err(format!("{label} id prefix is ambiguous; use the full id")),
    }
}

pub(crate) fn public_safe_json_value(
    mut value: serde_json::Value,
    tool_name: &str,
) -> serde_json::Value {
    redact_json_strings(&mut value, tool_name);
    value
}

fn redact_json_strings(value: &mut serde_json::Value, tool_name: &str) {
    match value {
        serde_json::Value::String(text) => {
            *text = public_safe_output_string(text, tool_name);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_strings(item, tool_name);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                redact_json_strings(item, tool_name);
            }
        }
        _ => {}
    }
}

fn public_safe_output_string(text: &str, tool_name: &str) -> String {
    let redacted = redact_local_paths(text);
    if ensure_no_secret_literal(tool_name, "output", &redacted).is_err() {
        "<secret>".to_string()
    } else {
        redacted
    }
}

pub(crate) fn redact_local_paths(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let (body, suffix) = split_trailing_ws(token);
        output.push_str(&redact_token_local_paths(body));
        output.push_str(suffix);
    }
    output
}

fn split_trailing_ws(token: &str) -> (&str, &str) {
    let body_end = token.trim_end_matches(char::is_whitespace).len();
    token.split_at(body_end)
}

fn redact_token_local_paths(token: &str) -> String {
    let mut start = 0;
    let mut end = token.len();
    while let Some(first) = token[start..end]
        .chars()
        .next()
        .filter(|ch| matches!(ch, '"' | '\'' | '`' | '(' | '[' | '{' | '<'))
    {
        let len = first.len_utf8();
        start += len;
    }
    while let Some(last) = token[start..end].chars().next_back().filter(|ch| {
        matches!(
            ch,
            '"' | '\'' | '`' | ')' | ']' | '}' | '>' | ',' | ';' | ':'
        )
    }) {
        let len = last.len_utf8();
        end -= len;
    }
    let prefix = &token[..start];
    let body = &token[start..end];
    let suffix = &token[end..];
    if looks_like_local_path(body) {
        format!("{prefix}<local-path>{suffix}")
    } else {
        token.to_string()
    }
}

fn looks_like_local_path(value: &str) -> bool {
    value.starts_with("~/")
        || value.starts_with("$HOME/")
        || value.starts_with("/Users/")
        || value.starts_with("/private/")
        || value.starts_with("/tmp/")
        || value.starts_with("/var/")
        || value.starts_with("/Volumes/")
        || value.starts_with("/home/")
        || value.starts_with("/root/")
        || value.starts_with("/etc/")
        || value.starts_with("/opt/")
        || value.starts_with("/usr/")
}

pub(crate) fn resolve_registry_index(
    items: &[serde_json::Value],
    id_prefix: &str,
    label: &str,
) -> Result<usize, String> {
    let matches: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            item["id"]
                .as_str()
                .is_some_and(|id| id.starts_with(id_prefix))
                .then_some(idx)
        })
        .collect();
    match matches.as_slice() {
        [] => Err(format!("{label} id not found")),
        [idx] => Ok(*idx),
        _ => Err(format!("{label} id prefix is ambiguous; use the full id")),
    }
}

pub(crate) fn push_note(
    object: &mut serde_json::Map<String, serde_json::Value>,
    now: &str,
    note: String,
) {
    let note_entry = serde_json::json!({ "at": now, "note": note });
    match object.get_mut("notes").and_then(|v| v.as_array_mut()) {
        Some(notes) => notes.push(note_entry),
        None => {
            object.insert("notes".to_string(), serde_json::json!([note_entry]));
        }
    }
}
