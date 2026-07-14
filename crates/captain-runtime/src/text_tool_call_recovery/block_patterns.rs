use super::parsers::{
    parse_arrow_syntax_tool_call, parse_json_tool_call_object, parse_xml_parameter_tool_call,
};
use super::push_unique;
use captain_types::tool::ToolCall;
use tracing::info;

pub(super) fn recover_tool_call_blocks(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("[TOOL_CALL]") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "[TOOL_CALL]".len();

        let Some(close_offset) = text[after_tag..].find("[/TOOL_CALL]") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "[/TOOL_CALL]".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, tool_names) {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (JSON)"
                );
            }
            continue;
        }

        if let Some((tool_name, input)) = parse_arrow_syntax_tool_call(inner, tool_names) {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (arrow syntax)"
                );
            }
        }
    }
}

pub(super) fn recover_tool_call_xml_blocks(
    text: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_call>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_call>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_call>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_call>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, tool_names) {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_call> block"
                );
            }
        } else if let Some((tool_name, input)) = parse_xml_parameter_tool_call(inner, tool_names) {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_call> block (XML parameter style)"
                );
            }
        }
    }
}

pub(super) fn recover_plugin_blocks(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<|plugin|>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<|plugin|>".len();

        let close_tag = "<|endofblock|>";
        let Some(close_offset) = text[after_tag..].find(close_tag) else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + close_tag.len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, tool_names) {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <|plugin|> block"
                );
            }
        }
    }
}

pub(super) fn recover_tool_use_blocks(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_use>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_use>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_use>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_use>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, tool_names) {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_use> block"
                );
            }
        }
    }
}
