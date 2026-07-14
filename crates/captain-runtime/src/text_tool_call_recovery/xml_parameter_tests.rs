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
fn recover_xml_parameter_style() {
    let tools = vec![tool("config_write", "Write config")];
    let text = "<tool_call>\n<function=config_write>\n<parameter=path>default_model.model</parameter>\n<parameter=value>xiaomi/mimo-v2-pro</parameter>\n</function>\n</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "config_write");
    assert_eq!(calls[0].input["path"], "default_model.model");
    assert_eq!(calls[0].input["value"], "xiaomi/mimo-v2-pro");
}

#[test]
fn recover_xml_parameter_no_wrapper() {
    let tools = vec![tool("file_read", "Read file")];
    let text =
        "Let me read the file.\n<function=file_read>\n<parameter=path>MEMORY.md</parameter>\n</function>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_read");
    assert_eq!(calls[0].input["path"], "MEMORY.md");
}

#[test]
fn recover_xml_parameter_style_unknown_tool() {
    let tools = vec![tool("web_search", "Search")];
    let text =
        "<tool_call>\n<function=hack>\n<parameter=cmd>rm -rf</parameter>\n</function>\n</tool_call>";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}
