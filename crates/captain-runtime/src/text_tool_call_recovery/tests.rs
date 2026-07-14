use super::recover_text_tool_calls;
use captain_types::tool::ToolDefinition;

fn tool(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({}),
    }
}

#[test]
fn recover_text_tool_calls_basic() {
    let tools = vec![tool("web_search", "Search the web")];
    let text = r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "rust async");
    assert!(calls[0].id.starts_with("recovered_"));
}

#[test]
fn recover_text_tool_calls_unknown_tool() {
    let tools = vec![tool("web_search", "Search the web")];
    let text = r#"<function=hack_system>{"cmd":"rm -rf /"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty(), "Unknown tools should be rejected");
}

#[test]
fn recover_text_tool_calls_invalid_json() {
    let tools = vec![tool("web_search", "Search the web")];
    let text = r#"<function=web_search>not valid json</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty(), "Invalid JSON should be skipped");
}

#[test]
fn recover_text_tool_calls_multiple() {
    let tools = vec![
        tool("web_search", "Search"),
        tool("read_file", "Read a file"),
    ];
    let text = r#"<function=web_search>{"query":"hello"}</function> then <function=read_file>{"path":"a.txt"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[1].name, "read_file");
}

#[test]
fn recover_text_tool_calls_no_pattern() {
    let tools = vec![tool("web_search", "Search")];
    let text = "Just a normal response with no tool calls.";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_text_tool_calls_empty_tools() {
    let text = r#"<function=web_search>{"query":"hello"}</function>"#;
    let calls = recover_text_tool_calls(text, &[]);
    assert!(calls.is_empty(), "No tools = no recovery");
}

#[test]
fn recover_text_tool_calls_nested_json() {
    let tools = vec![tool("web_search", "Search")];
    let text =
        r#"<function=web_search>{"query":"rust","filters":{"lang":"en","year":2024}}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["filters"]["lang"], "en");
}

#[test]
fn recover_text_tool_calls_with_surrounding_text() {
    let tools = vec![tool("web_search", "Search")];
    let text = "Sure, let me search that for you.\n\n<function=web_search>{\"query\":\"rust async programming\"}</function>\n\nI'll get back to you with results.";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["query"], "rust async programming");
}

#[test]
fn recover_text_tool_calls_whitespace_in_json() {
    let tools = vec![tool("web_search", "Search")];
    let text = "<function=web_search>\n  {\"query\": \"hello world\"}\n</function>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["query"], "hello world");
}

#[test]
fn recover_text_tool_calls_unclosed_tag() {
    let tools = vec![tool("web_search", "Search")];
    let text = r#"<function=web_search>{"query":"test"}"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty(), "Unclosed tag should be skipped");
}

#[test]
fn recover_text_tool_calls_missing_closing_bracket() {
    let tools = vec![tool("web_search", "Search")];
    let text = r#"<function=web_search{"query":"test"}</function>"#;
    let _ = recover_text_tool_calls(text, &tools);
}

#[test]
fn recover_text_tool_calls_empty_json_object() {
    let tools = vec![tool("list_files", "List")];
    let text = r#"<function=list_files>{}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "list_files");
    assert_eq!(calls[0].input, serde_json::json!({}));
}

#[test]
fn recover_text_tool_calls_mixed_valid_invalid() {
    let tools = vec![tool("web_search", "Search"), tool("read_file", "Read")];
    let text = r#"<function=web_search>{"q":"a"}</function> <function=unknown>{"x":1}</function> <function=read_file>{"path":"b"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2, "Should recover 2 valid, skip 1 unknown");
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[1].name, "read_file");
}

#[test]
fn recover_variant2_basic() {
    let tools = vec![tool("web_fetch", "Fetch")];
    let text = r#"<function>web_fetch{"url":"https://example.com"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_fetch");
    assert_eq!(calls[0].input["url"], "https://example.com");
}

#[test]
fn recover_variant2_unknown_tool() {
    let tools = vec![tool("web_search", "Search")];
    let text = r#"<function>unknown_tool{"q":"test"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 0);
}

#[test]
fn recover_variant2_with_surrounding_text() {
    let tools = vec![tool("web_search", "Search")];
    let text = r#"Let me search for that. <function>web_search{"query":"rust lang"}</function> I'll find the answer."#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
}

#[test]
fn recover_both_variants_mixed() {
    let tools = vec![tool("web_search", "Search"), tool("web_fetch", "Fetch")];
    let text = r#"<function=web_search>{"q":"a"}</function> <function>web_fetch{"url":"https://x.com"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[1].name, "web_fetch");
}

#[test]
fn recover_tool_tag_variant() {
    let tools = vec![tool("exec", "Execute")];
    let text = r#"I'll run that for you. <tool>exec{"command":"ls -la"}</tool>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn recover_markdown_code_block() {
    let tools = vec![tool("exec", "Execute")];
    let text = "I'll execute that command:\n```\nexec {\"command\": \"ls -la\"}\n```";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn recover_markdown_code_block_with_lang() {
    let tools = vec![tool("web_search", "Search")];
    let text = "```json\nweb_search {\"query\": \"rust\"}\n```";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
}

#[test]
fn recover_backtick_wrapped() {
    let tools = vec![tool("exec", "Execute")];
    let text = r#"Let me run `exec {"command":"pwd"}` for you."#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
    assert_eq!(calls[0].input["command"], "pwd");
}

#[test]
fn recover_backtick_ignores_unknown_tool() {
    let tools = vec![tool("exec", "Execute")];
    let text = r#"Try `unknown_tool {"key":"val"}` instead."#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_no_duplicates_across_patterns() {
    let tools = vec![tool("exec", "Execute")];
    let text = r#"<function=exec>{"command":"ls"}</function> <tool>exec{"command":"ls"}</tool>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
}

#[test]
fn recover_tool_call_block_json() {
    let tools = vec![tool("shell_exec", "Execute shell command")];
    let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn recover_tool_call_block_arrow_syntax() {
    let tools = vec![tool("shell_exec", "Execute shell command")];
    let text =
        "[TOOL_CALL]\n{tool => \"shell_exec\", args => {\n--command \"ls -F /\"\n}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -F /");
}

#[test]
fn recover_tool_call_block_unknown_tool() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "[TOOL_CALL]\n{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_tool_call_block_multiple() {
    let tools = vec![tool("shell_exec", "Execute"), tool("file_read", "Read")];
    let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}\n[/TOOL_CALL]\nSome text.\n[TOOL_CALL]\n{\"name\": \"file_read\", \"arguments\": {\"path\": \"/tmp/test.txt\"}}\n[/TOOL_CALL]";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[1].name, "file_read");
}

#[test]
fn recover_tool_call_block_unclosed() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1, "Bare JSON fallback should recover this");
    assert_eq!(calls[0].name, "shell_exec");
}

#[test]
fn recover_tool_call_xml_basic() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "<tool_call>\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn recover_tool_call_xml_with_surrounding_text() {
    let tools = vec![tool("web_search", "Search")];
    let text = "I'll search for that.\n\n<tool_call>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust async\"}}\n</tool_call>\n\nLet me get results.";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "rust async");
}

#[test]
fn recover_tool_call_xml_function_field() {
    let tools = vec![tool("file_read", "Read")];
    let text =
        "<tool_call>{\"function\": \"file_read\", \"arguments\": {\"path\": \"/etc/hosts\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_read");
}

#[test]
fn recover_tool_call_xml_parameters_field() {
    let tools = vec![tool("web_fetch", "Fetch")];
    let text =
        "<tool_call>{\"name\": \"web_fetch\", \"parameters\": {\"url\": \"https://example.com\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_fetch");
    assert_eq!(calls[0].input["url"], "https://example.com");
}

#[test]
fn recover_tool_call_xml_stringified_args() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": \"{\\\"command\\\": \\\"pwd\\\"}\"}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "pwd");
}

#[test]
fn recover_tool_call_xml_unknown_tool() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "<tool_call>{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_tool_call_xml_multiple() {
    let tools = vec![tool("shell_exec", "Execute"), tool("web_search", "Search")];
    let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}</tool_call>\n<tool_call>{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[1].name, "web_search");
}

#[test]
fn recover_bare_json_tool_call() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text =
        "I'll run that: {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn recover_bare_json_no_false_positive() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "The config looks like {\"debug\": true, \"level\": \"info\"}";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_bare_json_skipped_when_tags_found() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "<function=shell_exec>{\"command\":\"ls\"}</function> {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"pwd\"}}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["command"], "ls");
}

#[test]
fn recover_xml_attribute_basic() {
    let tools = vec![tool("web_search", "Search")];
    let text = r#"<function name="web_search" parameters="{&quot;query&quot;: &quot;best crypto 2024&quot;}" />"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "best crypto 2024");
}

#[test]
fn recover_xml_attribute_unknown_tool() {
    let tools = vec![tool("web_search", "Search")];
    let text = r#"<function name="unknown_tool" parameters="{&quot;x&quot;: 1}" />"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_xml_attribute_non_selfclosing() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = r#"<function name="shell_exec" parameters="{&quot;command&quot;: &quot;ls&quot;}"></function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
}

#[test]
fn recover_plugin_block() {
    let tools = vec![tool("web_search", "Search")];
    let text =
        "<|plugin|>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}\n<|endofblock|>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "rust");
}

#[test]
fn recover_plugin_block_unknown_tool() {
    let tools = vec![tool("web_search", "Search")];
    let text = "<|plugin|>\n{\"name\": \"hack\", \"arguments\": {\"cmd\": \"rm\"}}\n<|endofblock|>";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_action_input() {
    let tools = vec![tool("web_search", "Search")];
    let text = "Action: web_search\nAction Input: {\"query\": \"rust programming\"}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "rust programming");
}

#[test]
fn recover_action_input_unknown_tool() {
    let tools = vec![tool("web_search", "Search")];
    let text = "Action: unknown_tool\nAction Input: {\"key\": \"value\"}";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_name_json_nextline() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "shell_exec\n{\"command\": \"ls -la\"}";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn recover_name_json_nextline_unknown() {
    let tools = vec![tool("shell_exec", "Execute")];
    let text = "unknown_tool\n{\"command\": \"ls\"}";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn recover_tool_use_block() {
    let tools = vec![tool("web_search", "Search")];
    let text =
        "<tool_use>{\"name\": \"web_search\", \"arguments\": {\"query\": \"test\"}}</tool_use>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
}

#[test]
fn recover_tool_use_block_unknown() {
    let tools = vec![tool("web_search", "Search")];
    let text = "<tool_use>{\"name\": \"hack\", \"arguments\": {\"cmd\": \"rm\"}}</tool_use>";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}
