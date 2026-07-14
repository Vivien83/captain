use super::*;

#[test]
fn test_build_prompt_simple() {
    use captain_types::message::{Message, MessageContent};

    let request = CompletionRequest {
        model: "qwen-code/qwen3-coder".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::text("Hello"),
        }],
        tools: vec![],
        max_tokens: 1024,
        temperature: 0.7,
        system: Some("You are helpful.".to_string()),
        thinking: None,
        tool_choice: None,
        cache_hints: crate::llm_driver::CacheHints::default(),
    };

    let prompt = QwenCodeDriver::build_prompt(&request);
    assert!(prompt.contains("[System]"));
    assert!(prompt.contains("You are helpful."));
    assert!(prompt.contains("[User]"));
    assert!(prompt.contains("Hello"));
}

#[test]
fn test_model_flag_mapping() {
    assert_eq!(
        QwenCodeDriver::model_flag("qwen-code/qwen3-coder"),
        Some("qwen3-coder".to_string())
    );
    assert_eq!(
        QwenCodeDriver::model_flag("qwen-code/qwen-coder-plus"),
        Some("qwen-coder-plus".to_string())
    );
    assert_eq!(
        QwenCodeDriver::model_flag("qwen-code/qwq-32b"),
        Some("qwq-32b".to_string())
    );
    assert_eq!(
        QwenCodeDriver::model_flag("coder"),
        Some("qwen3-coder".to_string())
    );
    assert_eq!(
        QwenCodeDriver::model_flag("custom-model"),
        Some("custom-model".to_string())
    );
}

#[test]
fn test_new_defaults_to_qwen() {
    let driver = QwenCodeDriver::new(None, true);
    assert_eq!(driver.cli_path, "qwen");
    assert!(driver.skip_permissions);
}

#[test]
fn test_new_with_custom_path() {
    let driver = QwenCodeDriver::new(Some("/usr/local/bin/qwen".to_string()), true);
    assert_eq!(driver.cli_path, "/usr/local/bin/qwen");
}

#[test]
fn test_new_with_empty_path() {
    let driver = QwenCodeDriver::new(Some(String::new()), true);
    assert_eq!(driver.cli_path, "qwen");
}

#[test]
fn test_skip_permissions_disabled() {
    let driver = QwenCodeDriver::new(None, false);
    assert!(!driver.skip_permissions);
}

#[test]
fn test_sensitive_env_list_coverage() {
    assert!(SENSITIVE_ENV_EXACT.contains(&"OPENAI_API_KEY"));
    assert!(SENSITIVE_ENV_EXACT.contains(&"ANTHROPIC_API_KEY"));
    assert!(SENSITIVE_ENV_EXACT.contains(&"GEMINI_API_KEY"));
    assert!(SENSITIVE_ENV_EXACT.contains(&"GROQ_API_KEY"));
    assert!(SENSITIVE_ENV_EXACT.contains(&"DEEPSEEK_API_KEY"));
}

#[test]
fn test_build_args_with_yolo() {
    let driver = QwenCodeDriver::new(None, true);
    let args = driver.build_args("test prompt", "qwen-code/qwen3-coder", false);
    assert!(args.contains(&"--yolo".to_string()));
    assert!(args.contains(&"json".to_string()));
    assert!(args.contains(&"--model".to_string()));
}

#[test]
fn test_build_args_without_yolo() {
    let driver = QwenCodeDriver::new(None, false);
    let args = driver.build_args("test prompt", "qwen-code/qwen3-coder", false);
    assert!(!args.contains(&"--yolo".to_string()));
}

#[test]
fn test_build_args_streaming() {
    let driver = QwenCodeDriver::new(None, true);
    let args = driver.build_args("test prompt", "qwen-code/qwen3-coder", true);
    assert!(args.contains(&"stream-json".to_string()));
    assert!(args.contains(&"--verbose".to_string()));
}

#[test]
fn test_json_output_deserialization() {
    let json = r#"{"result":"Hello world","usage":{"input_tokens":10,"output_tokens":5}}"#;
    let parsed: QwenJsonOutput = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.result.unwrap(), "Hello world");
    assert_eq!(parsed.usage.unwrap().input_tokens, 10);
}

#[test]
fn test_json_output_content_field() {
    let json = r#"{"content":"Hello from content field"}"#;
    let parsed: QwenJsonOutput = serde_json::from_str(json).unwrap();
    assert!(parsed.result.is_none());
    assert_eq!(parsed.content.unwrap(), "Hello from content field");
}

#[test]
fn test_response_from_qwen_stdout_preserves_usage() {
    let response = response_from_qwen_stdout(
        r#"{"result":"Hello world","usage":{"input_tokens":10,"output_tokens":5}}"#,
    );

    match &response.content[0] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "Hello world"),
        other => panic!("unexpected content block: {other:?}"),
    }
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 5);
}

#[test]
fn test_response_from_qwen_stdout_accepts_plain_text() {
    let response = response_from_qwen_stdout("plain answer\n");

    match &response.content[0] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "plain answer"),
        other => panic!("unexpected content block: {other:?}"),
    }
    assert_eq!(response.usage.input_tokens, 0);
    assert_eq!(response.usage.output_tokens, 0);
}

#[test]
fn test_stream_event_deserialization() {
    let json = r#"{"type":"content","content":"Hello"}"#;
    let event: QwenStreamEvent = serde_json::from_str(json).unwrap();
    assert_eq!(event.r#type, "content");
    assert_eq!(event.content.unwrap(), "Hello");
}

#[test]
fn test_stream_event_result() {
    let json = r#"{"type":"result","result":"Final answer","usage":{"input_tokens":20,"output_tokens":10}}"#;
    let event: QwenStreamEvent = serde_json::from_str(json).unwrap();
    assert_eq!(event.r#type, "result");
    assert_eq!(event.result.unwrap(), "Final answer");
    assert_eq!(event.usage.unwrap().output_tokens, 10);
}

#[test]
fn test_stream_state_accumulates_content_result_and_usage() {
    let mut state = QwenStreamState::default();

    assert_eq!(
        state.ingest_event(QwenStreamEvent {
            r#type: "content".to_string(),
            content: Some("Hello".to_string()),
            result: None,
            usage: None,
        }),
        Some("Hello".to_string())
    );
    assert_eq!(
        state.ingest_event(QwenStreamEvent {
            r#type: "result".to_string(),
            content: None,
            result: Some("Ignored because text already streamed".to_string()),
            usage: Some(QwenUsage {
                input_tokens: 20,
                output_tokens: 10,
            }),
        }),
        None
    );

    let response = state.into_response();
    match &response.content[0] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "Hello"),
        other => panic!("unexpected content block: {other:?}"),
    }
    assert_eq!(response.usage.input_tokens, 20);
    assert_eq!(response.usage.output_tokens, 10);
}
