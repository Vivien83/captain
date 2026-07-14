use super::parsers::try_parse_bare_json_tool_call;
use super::push_unique;
use captain_types::tool::ToolCall;
use tracing::info;

pub(super) fn recover_markdown_code_blocks(
    text: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
) {
    let mut in_block = false;
    let mut block_content = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_block {
                recover_tool_plus_json(block_content.trim(), tool_names, calls, |tool| {
                    info!(tool = tool, "Recovered tool call from markdown code block")
                });
                block_content.clear();
                in_block = false;
            } else {
                in_block = true;
                block_content.clear();
            }
        } else if in_block {
            if !block_content.is_empty() {
                block_content.push('\n');
            }
            block_content.push_str(trimmed);
        }
    }
}

pub(super) fn recover_backtick_calls(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let parts: Vec<&str> = text.split('`').collect();
    for chunk in parts.iter().skip(1).step_by(2) {
        let trimmed = chunk.trim();
        let Some(brace_pos) = trimmed.find('{') else {
            continue;
        };
        let potential_tool = trimmed[..brace_pos].trim();
        if potential_tool.is_empty()
            || potential_tool.contains(' ')
            || !tool_names.contains(&potential_tool)
        {
            continue;
        }

        let Ok(input) = serde_json::from_str::<serde_json::Value>(trimmed[brace_pos..].trim())
        else {
            continue;
        };

        if push_unique(calls, potential_tool, input) {
            info!(
                tool = potential_tool,
                "Recovered tool call from backtick-wrapped text"
            );
        }
    }
}

pub(super) fn recover_action_input(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        let Some(tool_part) = line
            .strip_prefix("Action:")
            .or_else(|| line.strip_prefix("action:"))
        else {
            i += 1;
            continue;
        };
        let tool_name = tool_part.trim();
        if !tool_names.contains(&tool_name) || i + 1 >= lines.len() {
            i += 1;
            continue;
        }

        let next = lines[i + 1].trim();
        let Some(json_part) = next
            .strip_prefix("Action Input:")
            .or_else(|| next.strip_prefix("action input:"))
            .or_else(|| next.strip_prefix("action_input:"))
        else {
            i += 1;
            continue;
        };

        if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_part.trim()) {
            if push_unique(calls, tool_name, input) {
                info!(
                    tool = tool_name,
                    "Recovered tool call from Action/Action Input pattern"
                );
            }
        }
        i += 2;
    }
}

pub(super) fn recover_name_json_lines(text: &str, tool_names: &[&str], calls: &mut Vec<ToolCall>) {
    let lines: Vec<&str> = text.lines().collect();
    for i in 0..lines.len().saturating_sub(1) {
        let name_line = lines[i].trim();
        if name_line.contains(' ') || name_line.contains('{') || name_line.is_empty() {
            continue;
        }
        if !tool_names.contains(&name_line) {
            continue;
        }

        let json_line = lines[i + 1].trim();
        if !json_line.starts_with('{') {
            continue;
        }
        let Ok(input) = serde_json::from_str::<serde_json::Value>(json_line) else {
            continue;
        };

        if push_unique(calls, name_line, input) {
            info!(
                tool = name_line,
                "Recovered tool call from name+JSON line pair"
            );
        }
    }
}

pub(super) fn recover_bare_json_if_empty(
    text: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
) {
    if !calls.is_empty() {
        return;
    }

    let mut scan_from = 0;
    while let Some(brace_start) = text[scan_from..].find('{') {
        let abs_brace = scan_from + brace_start;
        if let Some((tool_name, input)) =
            try_parse_bare_json_tool_call(&text[abs_brace..], tool_names)
        {
            if push_unique(calls, &tool_name, input) {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from bare JSON object in text"
                );
            }
        }
        scan_from = abs_brace + 1;
    }
}

fn recover_tool_plus_json<F>(
    content: &str,
    tool_names: &[&str],
    calls: &mut Vec<ToolCall>,
    log_recovery: F,
) where
    F: FnOnce(&str),
{
    let Some(brace_pos) = content.find('{') else {
        return;
    };
    let potential_tool = content[..brace_pos].trim();
    if !tool_names.contains(&potential_tool) {
        return;
    }
    let Ok(input) = serde_json::from_str::<serde_json::Value>(content[brace_pos..].trim()) else {
        return;
    };
    if push_unique(calls, potential_tool, input) {
        log_recovery(potential_tool);
    }
}
