use captain_types::tool::{ToolCall, ToolDefinition};

mod block_patterns;
mod inline_patterns;
#[cfg(test)]
mod parser_tests;
mod parsers;
mod tag_patterns;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod xml_parameter_tests;

/// Recover tool calls emitted as plain text by providers that missed native
/// tool-call emission.
pub(crate) fn recover_text_tool_calls(
    text: &str,
    available_tools: &[ToolDefinition],
) -> Vec<ToolCall> {
    let tool_names: Vec<&str> = available_tools.iter().map(|t| t.name.as_str()).collect();
    let mut calls = Vec::new();

    tag_patterns::recover_function_equals_tags(text, &tool_names, &mut calls);
    tag_patterns::recover_function_content_tags(text, &tool_names, &mut calls);
    tag_patterns::recover_tool_tags(text, &tool_names, &mut calls);
    inline_patterns::recover_markdown_code_blocks(text, &tool_names, &mut calls);
    inline_patterns::recover_backtick_calls(text, &tool_names, &mut calls);
    block_patterns::recover_tool_call_blocks(text, &tool_names, &mut calls);
    block_patterns::recover_tool_call_xml_blocks(text, &tool_names, &mut calls);
    tag_patterns::recover_xml_attribute_functions(text, &tool_names, &mut calls);
    block_patterns::recover_plugin_blocks(text, &tool_names, &mut calls);
    inline_patterns::recover_action_input(text, &tool_names, &mut calls);
    inline_patterns::recover_name_json_lines(text, &tool_names, &mut calls);
    block_patterns::recover_tool_use_blocks(text, &tool_names, &mut calls);
    inline_patterns::recover_bare_json_if_empty(text, &tool_names, &mut calls);

    calls
}

fn push_unique(calls: &mut Vec<ToolCall>, tool_name: &str, input: serde_json::Value) -> bool {
    if calls
        .iter()
        .any(|c| c.name == tool_name && c.input == input)
    {
        return false;
    }

    calls.push(ToolCall {
        id: format!("recovered_{}", uuid::Uuid::new_v4()),
        name: tool_name.to_string(),
        input,
    });
    true
}

fn push_call(calls: &mut Vec<ToolCall>, tool_name: &str, input: serde_json::Value) {
    calls.push(ToolCall {
        id: format!("recovered_{}", uuid::Uuid::new_v4()),
        name: tool_name.to_string(),
        input,
    });
}
