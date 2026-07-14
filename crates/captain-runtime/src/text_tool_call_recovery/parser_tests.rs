use super::parsers::{parse_dash_dash_args, parse_json_tool_call_object};

#[test]
fn parse_dash_dash_args_basic() {
    let result = parse_dash_dash_args("{--command \"ls -F /\"}");
    assert_eq!(result["command"], "ls -F /");
}

#[test]
fn parse_dash_dash_args_multiple() {
    let result = parse_dash_dash_args("{--file \"test.txt\", --verbose}");
    assert_eq!(result["file"], "test.txt");
    assert_eq!(result["verbose"], true);
}

#[test]
fn parse_dash_dash_args_unquoted_value() {
    let result = parse_dash_dash_args("{--count 5}");
    assert_eq!(result["count"], "5");
}

#[test]
fn parse_json_tool_call_object_standard() {
    let tool_names = vec!["shell_exec"];
    let result = parse_json_tool_call_object(
        "{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}",
        &tool_names,
    );
    assert!(result.is_some());
    let (name, args) = result.unwrap();
    assert_eq!(name, "shell_exec");
    assert_eq!(args["command"], "ls");
}

#[test]
fn parse_json_tool_call_object_function_field() {
    let tool_names = vec!["web_fetch"];
    let result = parse_json_tool_call_object(
        "{\"function\": \"web_fetch\", \"parameters\": {\"url\": \"https://x.com\"}}",
        &tool_names,
    );
    assert!(result.is_some());
    let (name, args) = result.unwrap();
    assert_eq!(name, "web_fetch");
    assert_eq!(args["url"], "https://x.com");
}

#[test]
fn parse_json_tool_call_object_unknown_tool() {
    let tool_names = vec!["shell_exec"];
    let result =
        parse_json_tool_call_object("{\"name\": \"unknown\", \"arguments\": {}}", &tool_names);
    assert!(result.is_none());
}
