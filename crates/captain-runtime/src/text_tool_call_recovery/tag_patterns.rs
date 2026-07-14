use super::parsers::parse_parameter_body;
use super::{push_call, push_unique};
use captain_types::tool::ToolCall;
use tracing::{info, warn};

pub(super) fn recover_function_equals_tags(
    text: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<function=") {
        let abs_start = search_from + start;
        let after_prefix = abs_start + "<function=".len();

        let Some(name_end) = text[after_prefix..].find('>') else {
            search_from = after_prefix;
            continue;
        };
        let tool_name = &text[after_prefix..after_prefix + name_end];
        let json_start = after_prefix + name_end + 1;

        let Some(close_offset) = text[json_start..].find("</function>") else {
            search_from = json_start;
            continue;
        };
        let json_body = text[json_start..json_start + close_offset].trim();
        search_from = json_start + close_offset + "</function>".len();

        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call for unknown tool - skipping"
            );
            continue;
        }

        let input = if let Ok(v) = serde_json::from_str(json_body) {
            v
        } else if json_body.contains("<parameter=") {
            let Some(params) = parse_parameter_body(json_body, 0) else {
                continue;
            };
            serde_json::Value::Object(params)
        } else {
            warn!(
                tool = tool_name,
                "Failed to parse text-based tool call body - skipping"
            );
            continue;
        };

        info!(
            tool = tool_name,
            "Recovered text-based tool call -> synthetic ToolUse"
        );
        push_call(calls, tool_name, input);
    }
}

pub(super) fn recover_function_content_tags(
    text: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<function>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<function>".len();

        let Some(close_offset) = text[after_tag..].find("</function>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</function>".len();

        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() {
            continue;
        }
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call (variant 2) for unknown tool - skipping"
            );
            continue;
        }

        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(e) => {
                warn!(tool = tool_name, error = %e, "Failed to parse text-based tool call JSON (variant 2) - skipping");
                continue;
            }
        };

        if push_unique(calls, tool_name, input) {
            info!(
                tool = tool_name,
                "Recovered text-based tool call (variant 2) -> synthetic ToolUse"
            );
        }
    }
}

pub(super) fn recover_tool_tags(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<tool>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool>".len();

        let Some(close_offset) = text[after_tag..].find("</tool>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</tool>".len();

        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() || !tool_names.contains(&tool_name) {
            continue;
        }

        let Ok(input) = serde_json::from_str::<serde_json::Value>(json_body) else {
            continue;
        };

        if push_unique(calls, tool_name, input) {
            info!(
                tool = tool_name,
                "Recovered text-based tool call (<tool> variant) -> synthetic ToolUse"
            );
        }
    }
}

pub(super) fn recover_xml_attribute_functions(
    text: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
) {
    use regex_lite::Regex;

    let re = Regex::new(r#"<function\s+name="([^"]+)"\s+parameters="([^"]*)"[^/]*/?>"#).unwrap();
    for caps in re.captures_iter(text) {
        let tool_name = caps.get(1).unwrap().as_str();
        let raw_params = caps.get(2).unwrap().as_str();

        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "XML-attribute tool call for unknown tool - skipping"
            );
            continue;
        }

        let unescaped = raw_params
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&apos;", "'");

        let input = match serde_json::from_str::<serde_json::Value>(&unescaped) {
            Ok(v) => v,
            Err(e) => {
                warn!(tool = tool_name, error = %e, "Failed to parse XML-attribute tool call params - skipping");
                continue;
            }
        };

        if push_unique(calls, tool_name, input) {
            info!(
                tool = tool_name,
                "Recovered XML-attribute tool call -> synthetic ToolUse"
            );
        }
    }
}
