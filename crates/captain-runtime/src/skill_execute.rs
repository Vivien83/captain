//! Skill .md executor — parses capabilities from markdown and runs bash blocks.
//!
//! Skills follow the Captain .md format with `### capability_name` headers
//! followed by bash code blocks. Each capability can use `$CREDENTIAL_*` markers
//! for auto-injected credentials and `$token_name` for cached tokens.
//!
//! Credentials are auto-injected from the vault via `Credential \`Name\`` markers.
//! Tokens returned by capabilities are cached and injected into subsequent calls.

use dashmap::DashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, info};

/// Cache for tokens returned by skill capabilities (persists for server lifetime).
static TOKEN_CACHE: std::sync::LazyLock<DashMap<String, String>> =
    std::sync::LazyLock::new(DashMap::new);

pub const SKILL_SYNTAX_PREFLIGHT_ERROR_PREFIX: &str = "Skill capability syntax preflight failed";

/// Execute a specific capability from a skill .md file.
///
/// - `skill_path`: path to the .md file
/// - `capability`: the `### heading` name to execute
/// - `credentials`: map of credential names to resolved values
/// - `args`: additional args injected as env vars
///
/// Returns the stdout output (expected to be JSON) or an error.
pub async fn execute_capability(
    skill_path: &Path,
    capability: &str,
    credentials: &[(String, String)],
    args: &serde_json::Value,
) -> Result<String, String> {
    let content = std::fs::read_to_string(skill_path).map_err(|e| format!("Read skill: {e}"))?;

    let script = extract_bash_block(&content, capability)
        .ok_or_else(|| format!("Capability '{capability}' not found in skill"))?;

    preflight_bash_syntax(capability, &script)?;

    // Build env vars: credentials + cached tokens + args
    let mut env_vars: Vec<(String, String)> = credentials.to_vec();

    // Inject cached tokens
    for entry in TOKEN_CACHE.iter() {
        env_vars.push((entry.key().clone(), entry.value().clone()));
    }

    // Inject args as env vars
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            env_vars.push((k.clone(), val));
        }
    }

    info!(skill = %skill_path.display(), capability, "Executing skill capability");

    let output = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .envs(env_vars)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Spawn bash: {e}"))?;

    let result = tokio::time::timeout(Duration::from_secs(30), output.wait_with_output())
        .await
        .map_err(|_| "Skill execution timed out (30s)".to_string())?
        .map_err(|e| format!("Wait output: {e}"))?;

    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    let stderr = String::from_utf8_lossy(&result.stderr).to_string();

    if !result.status.success() {
        return Err(format!(
            "Skill '{}' failed (exit {}): {}",
            capability,
            result.status.code().unwrap_or(-1),
            if stderr.is_empty() { &stdout } else { &stderr }
        ));
    }

    // Cache any tokens from the output (look for JSON keys ending in _token)
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if let Some(obj) = json.as_object() {
            for (k, v) in obj {
                if k.ends_with("_token") || k.ends_with("_jwt") || k.ends_with("_key") {
                    if let Some(s) = v.as_str() {
                        debug!(key = k, "Caching skill token");
                        TOKEN_CACHE.insert(k.clone(), s.to_string());
                    }
                }
            }
        }
    }

    Ok(stdout)
}

/// Check a capability's bash syntax without executing the skill.
pub fn preflight_capability_syntax(skill_path: &Path, capability: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(skill_path).map_err(|e| format!("Read skill: {e}"))?;
    let script = extract_bash_block(&content, capability)
        .ok_or_else(|| format!("Capability '{capability}' not found in skill"))?;
    preflight_bash_syntax(capability, &script)
}

pub fn is_syntax_preflight_error(error: &str) -> bool {
    error.starts_with(SKILL_SYNTAX_PREFLIGHT_ERROR_PREFIX)
}

fn preflight_bash_syntax(capability: &str, script: &str) -> Result<(), String> {
    let mut child = StdCommand::new("bash")
        .arg("-n")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| syntax_preflight_error(capability, format!("bash unavailable: {e}")))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(script.as_bytes()).map_err(|e| {
            syntax_preflight_error(capability, format!("write preflight script: {e}"))
        })?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| syntax_preflight_error(capability, format!("wait for bash -n: {e}")))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(syntax_preflight_error(
        capability,
        if stderr.is_empty() {
            "bash -n failed".to_string()
        } else {
            stderr
        },
    ))
}

fn syntax_preflight_error(capability: &str, detail: String) -> String {
    format!("{SKILL_SYNTAX_PREFLIGHT_ERROR_PREFIX} for '{capability}': {detail}")
}

/// Extract the bash code block under a `### capability_name` heading.
fn extract_bash_block(content: &str, capability: &str) -> Option<String> {
    let heading = format!("### {capability}");
    let lines: Vec<&str> = content.lines().collect();

    let mut in_capability = false;
    let mut in_code_block = false;
    let mut script_lines = Vec::new();

    for line in &lines {
        if line.trim().eq_ignore_ascii_case(&heading) || line.trim() == heading {
            in_capability = true;
            continue;
        }

        if in_capability && !in_code_block {
            // Next heading = end of capability
            if line.starts_with("### ") || line.starts_with("## ") {
                break;
            }
            if line.trim().starts_with("```bash") || line.trim().starts_with("```sh") {
                in_code_block = true;
                continue;
            }
        }

        if in_capability && in_code_block {
            if line.trim() == "```" {
                break;
            }
            script_lines.push(*line);
        }
    }

    if script_lines.is_empty() {
        None
    } else {
        Some(script_lines.join("\n"))
    }
}

/// Extract credential references from skill content.
///
/// Looks for `Credential \`Name\`` patterns and returns the credential names.
pub fn extract_credential_refs(content: &str) -> Vec<String> {
    let mut creds = Vec::new();
    for line in content.lines() {
        let mut start = 0;
        while let Some(pos) = line[start..].find("Credential `") {
            let abs_pos = start + pos + 12; // skip "Credential `"
            if let Some(end) = line[abs_pos..].find('`') {
                creds.push(line[abs_pos..abs_pos + end].to_string());
            }
            start = abs_pos;
        }
    }
    creds.sort();
    creds.dedup();
    creds
}

/// List all capabilities (### headings with bash blocks) in a skill .md file.
pub fn list_capabilities(content: &str) -> Vec<String> {
    let mut caps = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("### ") {
            let name = rest.trim().to_string();
            if extract_bash_block(content, &name).is_some() {
                caps.push(name);
            }
        }
    }
    caps
}

/// Get the default skills directory (~/.captain/skills/).
pub fn captain_skills_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".captain")
        .join("skills")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SKILL: &str = r#"---
name: test-skill
---

### login
```bash
echo '{"sport_token": "abc123"}'
```

### list_slots
```bash
echo "slots for token: $sport_token"
```

### no_bash
Just some text, no code block.
"#;

    #[test]
    fn test_extract_bash_block() {
        let script = extract_bash_block(SAMPLE_SKILL, "login").unwrap();
        assert!(script.contains("sport_token"));
    }

    #[test]
    fn test_extract_missing_capability() {
        assert!(extract_bash_block(SAMPLE_SKILL, "nonexistent").is_none());
    }

    #[test]
    fn test_no_bash_block() {
        assert!(extract_bash_block(SAMPLE_SKILL, "no_bash").is_none());
    }

    #[test]
    fn test_list_capabilities() {
        let caps = list_capabilities(SAMPLE_SKILL);
        assert_eq!(caps, vec!["login", "list_slots"]);
    }

    #[test]
    fn test_extract_credential_refs() {
        let content = "Use Credential `ResaWod` and Credential `Gmail` for auth.";
        let creds = extract_credential_refs(content);
        assert_eq!(creds, vec!["Gmail", "ResaWod"]);
    }

    #[test]
    fn test_preflight_capability_syntax_passes_valid_bash() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("test.md");
        std::fs::write(
            &skill_path,
            "### hello\n```bash\necho '{\"result\": \"ok\"}'\n```\n",
        )
        .unwrap();

        assert!(preflight_capability_syntax(&skill_path, "hello").is_ok());
    }

    #[test]
    fn test_preflight_capability_syntax_fails_invalid_bash() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("test.md");
        std::fs::write(
            &skill_path,
            "### broken\n```bash\nif true; then\n  echo missing-fi\n```\n",
        )
        .unwrap();

        let err = preflight_capability_syntax(&skill_path, "broken").unwrap_err();
        assert!(is_syntax_preflight_error(&err));
        assert!(err.contains("broken"));
    }

    #[tokio::test]
    async fn test_execute_echo_capability() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("test.md");
        std::fs::write(
            &skill_path,
            "### hello\n```bash\necho '{\"result\": \"ok\"}'\n```\n",
        )
        .unwrap();

        let result = execute_capability(&skill_path, "hello", &[], &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.contains("ok"));
    }

    #[tokio::test]
    async fn test_execute_blocks_invalid_bash_before_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let skill_path = dir.path().join("test.md");
        std::fs::write(
            &skill_path,
            format!(
                "### broken\n```bash\ntouch {}\nif true; then\n  echo missing-fi\n```\n",
                marker.display()
            ),
        )
        .unwrap();

        let err = execute_capability(&skill_path, "broken", &[], &serde_json::json!({}))
            .await
            .unwrap_err();

        assert!(is_syntax_preflight_error(&err));
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn test_token_caching() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("test.md");
        std::fs::write(
            &skill_path,
            "### login\n```bash\necho '{\"api_token\": \"secret123\"}'\n```\n### use_token\n```bash\necho \"token=$api_token\"\n```\n",
        )
        .unwrap();

        // Execute login — should cache api_token
        execute_capability(&skill_path, "login", &[], &serde_json::json!({}))
            .await
            .unwrap();

        // Execute use_token — should have api_token injected
        let result = execute_capability(&skill_path, "use_token", &[], &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.contains("secret123"));
    }
}
