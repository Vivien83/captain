use super::*;

/// B.1 — Captain's daemon holds API keys (OPENAI_API_KEY, GROQ_API_KEY,
/// SLACK_TOKEN, …) in its env; an LLM-supplied execute_code snippet must
/// NOT inherit them. We plant a fake secret in the parent env, run a
/// bash one-liner that prints it, and assert the subprocess sees nothing.
#[tokio::test]
async fn execute_code_strips_inherited_secret() {
    std::env::set_var("CAPTAIN_TEST_SECRET_B1", "topsecret_42");
    let input = serde_json::json!({
        "language": "bash",
        "code": "echo MARKER=$CAPTAIN_TEST_SECRET_B1"
    });
    let res = tool_execute_code(&input, None)
        .await
        .expect("execute_code ok");
    let body: serde_json::Value = serde_json::from_str(&res).unwrap();
    let stdout = body["stdout"].as_str().unwrap_or("");
    assert!(
        !stdout.contains("topsecret_42"),
        "execute_code must not leak ambient secrets — got stdout: {stdout}"
    );
    std::env::remove_var("CAPTAIN_TEST_SECRET_B1");
}

/// B.1 — env_clear must not break tools that need PATH to locate the
/// interpreter. We assert PATH survives the whitelist.
#[tokio::test]
async fn execute_code_preserves_path() {
    let input = serde_json::json!({
        "language": "bash",
        "code": "if [ -n \"$PATH\" ]; then echo PATH_OK; else echo PATH_MISSING; fi"
    });
    let res = tool_execute_code(&input, None)
        .await
        .expect("execute_code ok");
    let body: serde_json::Value = serde_json::from_str(&res).unwrap();
    let stdout = body["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("PATH_OK"),
        "PATH must survive env_clear whitelist — got stdout: {stdout}"
    );
}

const TEST_GOOGLE_API_KEY: &str = "AIzaSyC3_abcdefghij1234567890ABCDEFGhij";

fn assert_secret_feedback(err: &str) {
    assert!(err.contains("Security blocked"), "got: {err}");
    assert!(err.contains("secret_write"), "got: {err}");
    assert!(err.contains("secret_read"), "got: {err}");
    assert!(err.contains("env_inject"), "got: {err}");
    assert!(
        !err.contains(TEST_GOOGLE_API_KEY),
        "must not echo key: {err}"
    );
}

#[test]
fn secret_literal_guard_gives_actionable_vault_feedback() {
    let err = ensure_no_secret_literal("file_write", "content", TEST_GOOGLE_API_KEY)
        .expect_err("literal API key must be blocked");
    assert_secret_feedback(&err);

    ensure_no_secret_literal(
        "file_write",
        "content",
        r#"api_key = os.getenv("GEMINI_API_KEY")"#,
    )
    .expect("env-var references are safe; only raw values are blocked");
}

#[tokio::test]
async fn file_write_rejects_literal_secret_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let input = serde_json::json!({
        "path": "gemini_probe.py",
        "content": format!("API_KEY = \"{TEST_GOOGLE_API_KEY}\"\n")
    });
    let err = tool_file_write(&input, Some(dir.path()), None, None)
        .await
        .expect_err("file_write must reject raw secrets");
    assert_secret_feedback(&err);
    assert!(
        !dir.path().join("gemini_probe.py").exists(),
        "blocked write must not create the script"
    );
}

#[tokio::test]
async fn edit_file_rejects_secret_replacement_and_preserves_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    tokio::fs::write(&path, "API_KEY = None\n").await.unwrap();

    let input = serde_json::json!({
        "path": "client.py",
        "old_string": "API_KEY = None",
        "new_string": format!("API_KEY = \"{TEST_GOOGLE_API_KEY}\"")
    });
    let err = tool_edit_file(&input, Some(dir.path()), None, None)
        .await
        .expect_err("edit_file must reject raw secrets");
    assert_secret_feedback(&err);
    assert_eq!(
        tokio::fs::read_to_string(&path).await.unwrap(),
        "API_KEY = None\n"
    );
}

#[tokio::test]
async fn multi_edit_rejects_secret_replacement_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    let original = "name = 'demo'\napi_key = None\n";
    tokio::fs::write(&path, original).await.unwrap();

    let input = serde_json::json!({
        "path": "client.py",
        "edits": [
            { "old_string": "name = 'demo'", "new_string": "name = 'safe'" },
            { "old_string": "api_key = None", "new_string": format!("api_key = \"{TEST_GOOGLE_API_KEY}\"") }
        ]
    });
    let err = tool_multi_edit(&input, Some(dir.path()), None, None)
        .await
        .expect_err("multi_edit must reject raw secrets");
    assert_secret_feedback(&err);
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), original);
}

#[tokio::test]
async fn apply_patch_rejects_secret_added_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    tokio::fs::write(&path, "api_key = None\n").await.unwrap();
    let patch = format!(
        "*** Begin Patch\n*** Update File: client.py\n@@\n-api_key = None\n+api_key = \"{TEST_GOOGLE_API_KEY}\"\n*** End Patch"
    );
    let input = serde_json::json!({ "patch": patch });

    let err = tool_apply_patch(&input, Some(dir.path()), None, None)
        .await
        .expect_err("apply_patch must reject raw secrets");
    assert_secret_feedback(&err);
    assert_eq!(
        tokio::fs::read_to_string(&path).await.unwrap(),
        "api_key = None\n"
    );
}

#[tokio::test]
async fn execute_and_shell_reject_literal_secret_before_spawn() {
    let code_input = serde_json::json!({
        "language": "python",
        "code": format!("API_KEY = \"{TEST_GOOGLE_API_KEY}\"")
    });
    let code_err = tool_execute_code(&code_input, None)
        .await
        .expect_err("execute_code must reject raw secrets");
    assert_secret_feedback(&code_err);

    let shell_input = serde_json::json!({
        "command": format!("echo {TEST_GOOGLE_API_KEY}")
    });
    let shell_err = tool_shell_exec(&shell_input, &[], None, None)
        .await
        .expect_err("shell_exec must reject raw secrets");
    assert_secret_feedback(&shell_err);
}

#[tokio::test]
async fn shell_exec_rejects_raw_secrets_env_sourcing() {
    let shell_input = serde_json::json!({
        "command": "set -a; . /root/.captain/secrets.env; set +a; curl -s https://example.com"
    });
    let shell_err = tool_shell_exec(&shell_input, &[], None, None)
        .await
        .expect_err("shell_exec must reject raw secrets.env sourcing");
    assert!(shell_err.contains("secret_read"));
    assert!(shell_err.contains("env_inject"));
}

/// v3.8f — execute_code is registered and accepts language param.
#[test]
fn test_execute_code_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"execute_code"));
    let def = tools
        .iter()
        .find(|t| t.name == "execute_code")
        .expect("execute_code must exist");
    let enum_values = def.input_schema["properties"]["language"]["enum"]
        .as_array()
        .expect("language.enum must be an array");
    let langs: Vec<&str> = enum_values.iter().filter_map(|v| v.as_str()).collect();
    assert!(langs.contains(&"python"));
    assert!(langs.contains(&"node"));
    assert!(langs.contains(&"bash"));
}

/// v3.8f — pip_install allowlist rejects unknown packages.
#[test]
fn test_pip_install_allowlist_rejects_unknown() {
    let unknown = vec!["curl-hijack".to_string()];
    assert!(validate_pip_allowlist(&unknown).is_err());

    let known = vec!["requests".to_string()];
    assert!(validate_pip_allowlist(&known).is_ok());

    let with_version = vec!["requests>=2.30".to_string()];
    assert!(validate_pip_allowlist(&with_version).is_ok());
}

/// A model (observed: gpt-5.5/codex) sends `pip_install: []` as a boilerplate
/// default on execute_code calls regardless of language. An empty array
/// must not be treated as "requesting packages on a non-Python language" —
/// only a genuinely non-empty pip_install list should be rejected there.
#[tokio::test]
async fn execute_code_bash_tolerates_empty_pip_install() {
    let input = serde_json::json!({
        "language": "bash",
        "code": "echo EMPTY_PIP_INSTALL_OK",
        "pip_install": []
    });
    let res = tool_execute_code(&input, None)
        .await
        .expect("empty pip_install must not block a non-Python execute_code call");
    let body: serde_json::Value = serde_json::from_str(&res).unwrap();
    let stdout = body["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("EMPTY_PIP_INSTALL_OK"),
        "got stdout: {stdout}"
    );
}

/// Non-regression: a genuinely non-empty pip_install on a non-Python
/// language is still rejected.
#[tokio::test]
async fn execute_code_bash_rejects_nonempty_pip_install() {
    let input = serde_json::json!({
        "language": "bash",
        "code": "echo should_not_run",
        "pip_install": ["requests"]
    });
    let err = tool_execute_code(&input, None)
        .await
        .expect_err("non-empty pip_install must still be rejected for bash");
    assert!(err.contains("pip_install is only valid for language=python"));
}

/// v3.8f — execute_code with inline Python produces stdout (requires python3).
#[tokio::test]
async fn test_execute_code_python_echo() {
    if find_python_interpreter() == "python3"
        && !std::process::Command::new("which")
            .arg("python3")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    {
        eprintln!("skipping: python3 not available");
        return;
    }
    let input = serde_json::json!({
        "code": "print('hello_v3_8f')",
        "language": "python",
    });
    let result = tool_execute_code(&input, None).await;
    let text = result.expect("execute_code should succeed");
    assert!(text.contains("hello_v3_8f"), "stdout missing: {text}");
    assert!(text.contains("\"exit_code\":0"));
}
