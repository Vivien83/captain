/// Parse a JSON object that represents a tool call.
///
/// Supports:
/// - `{"name":"tool","arguments":{"key":"value"}}`
/// - `{"name":"tool","parameters":{"key":"value"}}`
/// - `{"function":"tool","arguments":{"key":"value"}}`
/// - `{"tool":"tool_name","args":{"key":"value"}}`
pub(crate) fn parse_json_tool_call_object(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let obj: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = obj.as_object()?;

    let name = obj
        .get("name")
        .or_else(|| obj.get("function"))
        .or_else(|| obj.get("tool"))
        .and_then(|v| v.as_str())?;

    if !tool_names.contains(&name) {
        return None;
    }

    let args = obj
        .get("arguments")
        .or_else(|| obj.get("parameters"))
        .or_else(|| obj.get("args"))
        .or_else(|| obj.get("input"))
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let args = if let Some(s) = args.as_str() {
        serde_json::from_str(s).unwrap_or(serde_json::json!({}))
    } else {
        args
    };

    Some((name.to_string(), args))
}

/// Parse XML parameter-style tool calls emitted by Qwen 3.6 / Xiaomi MiMo:
/// `<function=tool_name><parameter=key>value</parameter>...</function>`.
pub(super) fn parse_xml_parameter_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let func_start = text.find("<function=")?;
    let after_eq = func_start + "<function=".len();
    let name_end = text[after_eq..].find('>')?;
    let tool_name = &text[after_eq..after_eq + name_end];

    if !tool_names.contains(&tool_name) {
        return None;
    }

    parse_parameter_body(text, after_eq + name_end + 1)
        .map(|params| (tool_name.to_string(), serde_json::Value::Object(params)))
}

pub(super) fn parse_parameter_body(
    text: &str,
    mut search: usize,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut params = serde_json::Map::new();
    while let Some(param_start) = text[search..].find("<parameter=") {
        let abs = search + param_start;
        let key_start = abs + "<parameter=".len();
        let Some(key_end) = text[key_start..].find('>') else {
            search = key_start;
            continue;
        };
        let key = &text[key_start..key_start + key_end];
        let value_start = key_start + key_end + 1;
        let Some(close) = text[value_start..].find("</parameter>") else {
            search = value_start;
            continue;
        };
        let value = text[value_start..value_start + close].trim();
        params.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
        search = value_start + close + "</parameter>".len();
    }

    if params.is_empty() {
        None
    } else {
        Some(params)
    }
}

/// Parse the custom arrow syntax used by some Ollama models:
/// `{tool => "name", args => {--key "value"}}` or `{tool => "name", args => {"key":"value"}}`.
pub(super) fn parse_arrow_syntax_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let tool_marker_pos = text.find("tool")?;
    let after_tool = &text[tool_marker_pos + 4..];
    let after_arrow = after_tool.trim_start();
    let after_arrow = after_arrow.strip_prefix("=>")?;
    let after_arrow = after_arrow.trim_start();

    let tool_name = if let Some(stripped) = after_arrow.strip_prefix('"') {
        let end_quote = stripped.find('"')?;
        &stripped[..end_quote]
    } else {
        let end = after_arrow
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .unwrap_or(after_arrow.len());
        &after_arrow[..end]
    };

    if tool_name.is_empty() || !tool_names.contains(&tool_name) {
        return None;
    }

    let args_value = if let Some(args_pos) = text.find("args") {
        let after_args = &text[args_pos + 4..];
        let after_args = after_args.trim_start();
        let after_args = after_args.strip_prefix("=>")?;
        let after_args = after_args.trim_start();

        if after_args.starts_with('{') {
            serde_json::from_str::<serde_json::Value>(after_args)
                .unwrap_or_else(|_| parse_dash_dash_args(after_args))
        } else {
            serde_json::json!({})
        }
    } else {
        serde_json::json!({})
    };

    Some((tool_name.to_string(), args_value))
}

/// Parse `{--key "value", --flag}` or `{--command "ls -F /"}` style arguments
/// into a JSON object.
pub(crate) fn parse_dash_dash_args(text: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    let inner = if text.starts_with('{') {
        let mut depth = 0;
        let mut end = text.len();
        for (i, c) in text.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        text[1..end].trim()
    } else {
        text.trim()
    };

    let mut remaining = inner;
    while let Some(dash_pos) = remaining.find("--") {
        remaining = &remaining[dash_pos + 2..];

        let key_end = remaining
            .find(|c: char| c.is_whitespace() || c == '=' || c == '"')
            .unwrap_or(remaining.len());
        let key = &remaining[..key_end];
        if key.is_empty() {
            continue;
        }
        remaining = remaining[key_end..].trim_start();

        if remaining.starts_with('=') {
            remaining = remaining[1..].trim_start();
        }

        if remaining.starts_with('"') {
            if let Some(end_quote) = remaining[1..].find('"') {
                let value = &remaining[1..1 + end_quote];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                remaining = &remaining[2 + end_quote..];
            } else {
                let value = &remaining[1..];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                break;
            }
        } else {
            let val_end = remaining
                .find([',', '}'])
                .or_else(|| remaining.find("--"))
                .unwrap_or(remaining.len());
            let value = remaining[..val_end].trim();
            if value.is_empty() {
                map.insert(key.to_string(), serde_json::Value::Bool(true));
            } else {
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            }
            remaining = &remaining[val_end..];
        }

        remaining = remaining.trim_start();
        if remaining.starts_with(',') {
            remaining = remaining[1..].trim_start();
        }
    }

    serde_json::Value::Object(map)
}

/// Try to parse a bare JSON object as a tool call.
pub(super) fn try_parse_bare_json_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let mut depth = 0;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return None;
    }

    parse_json_tool_call_object(&text[..end], tool_names)
}
