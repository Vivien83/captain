//! Skill .md executor — parses capabilities from markdown and runs bash blocks.
//!
//! Skills follow the Captain .md format with `### capability_name` headers
//! followed by bash code blocks. Each capability can use `$CREDENTIAL_*` markers
//! for auto-injected credentials and `$token_name` for cached tokens.
//!
//! Credentials are auto-injected from the vault via `Credential \`Name\`` markers.
//! Tokens returned by capabilities are cached and injected into subsequent calls.

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{debug, info};
use zeroize::Zeroizing;

const SKILL_TOKEN_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SkillTokenScope {
    canonical_path: PathBuf,
    source_sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SkillTokenCacheKey {
    scope: SkillTokenScope,
    token_name: String,
}

struct CachedSkillToken {
    value: Zeroizing<String>,
    expires_at: Instant,
}

/// Tokens are shared only by capabilities from the exact same skill source.
static TOKEN_CACHE: std::sync::LazyLock<DashMap<SkillTokenCacheKey, CachedSkillToken>> =
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
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<String, String> {
    let content = std::fs::read_to_string(skill_path).map_err(|e| format!("Read skill: {e}"))?;
    let token_scope = skill_token_scope(skill_path, &content);

    let script = extract_bash_block(&content, capability)
        .ok_or_else(|| format!("Capability '{capability}' not found in skill"))?;

    preflight_bash_syntax(capability, &script, exec_policy)?;

    // Build env vars: credentials + cached tokens + args
    let mut env_vars: Vec<(String, String)> = credentials.to_vec();

    env_vars.extend(cached_tokens_for_scope(&token_scope, Instant::now()));

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

    let outcome =
        crate::guarded_exec::run_unattended_shell(crate::guarded_exec::ShellExecRequest {
            surface: crate::guarded_exec::ExecSurface::SkillCapability,
            command: &script,
            policy: exec_policy,
            workspace: skill_path.parent(),
            allowed_env_vars: &[],
            explicit_env: &env_vars,
            timeout_secs: 30,
            no_output_timeout_secs: Some(30),
            bash_required: true,
        })
        .await?;

    if !outcome.success() {
        return Err(format!(
            "Skill '{}' failed (exit {}): {}",
            capability,
            outcome.exit_code,
            if outcome.stderr.is_empty() {
                &outcome.stdout
            } else {
                &outcome.stderr
            }
        ));
    }

    cache_tokens_from_output(&token_scope, &outcome.stdout, Instant::now());

    Ok(outcome.stdout)
}

fn skill_token_scope(skill_path: &Path, content: &str) -> SkillTokenScope {
    SkillTokenScope {
        canonical_path: std::fs::canonicalize(skill_path)
            .unwrap_or_else(|_| skill_path.to_path_buf()),
        source_sha256: Sha256::digest(content.as_bytes()).into(),
    }
}

fn cached_tokens_for_scope(scope: &SkillTokenScope, now: Instant) -> Vec<(String, String)> {
    TOKEN_CACHE.retain(|_, token| token.expires_at > now);
    TOKEN_CACHE
        .iter()
        .filter(|entry| entry.key().scope == *scope)
        .map(|entry| {
            (
                entry.key().token_name.clone(),
                entry.value().value.as_str().to_string(),
            )
        })
        .collect()
}

fn cache_tokens_from_output(scope: &SkillTokenScope, stdout: &str, now: Instant) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return;
    };
    let Some(object) = json.as_object() else {
        return;
    };

    for (key, value) in object {
        if !(key.ends_with("_token") || key.ends_with("_jwt") || key.ends_with("_key")) {
            continue;
        }
        let Some(value) = value.as_str() else {
            continue;
        };
        debug!(key, "Caching scoped skill token");
        TOKEN_CACHE.insert(
            SkillTokenCacheKey {
                scope: scope.clone(),
                token_name: key.clone(),
            },
            CachedSkillToken {
                value: Zeroizing::new(value.to_string()),
                expires_at: now + SKILL_TOKEN_TTL,
            },
        );
    }
}

/// Check a capability's bash syntax without executing the skill.
pub fn preflight_capability_syntax(skill_path: &Path, capability: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(skill_path).map_err(|e| format!("Read skill: {e}"))?;
    let script = extract_bash_block(&content, capability)
        .ok_or_else(|| format!("Capability '{capability}' not found in skill"))?;
    preflight_bash_syntax(capability, &script, None)
}

pub fn is_syntax_preflight_error(error: &str) -> bool {
    error.starts_with(SKILL_SYNTAX_PREFLIGHT_ERROR_PREFIX)
}

fn preflight_bash_syntax(
    capability: &str,
    script: &str,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<(), String> {
    crate::guarded_exec::check_bash_syntax(
        crate::guarded_exec::ExecSurface::SkillCapability,
        script,
        exec_policy,
    )
    .map_err(|error| syntax_preflight_error(capability, error))
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

        let result = execute_capability(&skill_path, "hello", &[], &serde_json::json!({}), None)
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

        let err = execute_capability(&skill_path, "broken", &[], &serde_json::json!({}), None)
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
        execute_capability(&skill_path, "login", &[], &serde_json::json!({}), None)
            .await
            .unwrap();

        // Execute use_token — should have api_token injected
        let result =
            execute_capability(&skill_path, "use_token", &[], &serde_json::json!({}), None)
                .await
                .unwrap();
        assert!(result.contains("secret123"));
    }

    #[tokio::test]
    async fn cached_tokens_never_cross_skill_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let source_skill = dir.path().join("source.md");
        let unrelated_skill = dir.path().join("unrelated.md");
        std::fs::write(
            &source_skill,
            "### login\n```bash\necho '{\"shared_token\": \"source-secret\"}'\n```\n",
        )
        .unwrap();
        std::fs::write(
            &unrelated_skill,
            "### inspect\n```bash\necho \"token=${shared_token-unset}\"\n```\n",
        )
        .unwrap();

        execute_capability(&source_skill, "login", &[], &serde_json::json!({}), None)
            .await
            .unwrap();
        let result = execute_capability(
            &unrelated_skill,
            "inspect",
            &[],
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.trim(), "token=unset");
    }

    #[tokio::test]
    async fn skill_capability_blocks_critical_content_before_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("critical.md");
        std::fs::write(&skill_path, "### run\n```bash\nrm -rf /\n```\n").unwrap();

        let error = execute_capability(&skill_path, "run", &[], &serde_json::json!({}), None)
            .await
            .unwrap_err();

        assert!(error.contains("critical pattern"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skill_capability_never_inherits_daemon_secrets() {
        let _guard = crate::guarded_exec::TEST_ASYNC_ENV_LOCK.lock().await;
        let key = "CAPTAIN_SKILL_INHERITED_SECRET";
        std::env::set_var(key, "must-not-leak");
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("isolated.md");
        std::fs::write(
            &skill_path,
            "### inspect\n```bash\nprintf '%s' \"${CAPTAIN_SKILL_INHERITED_SECRET-unset}\"\n```\n",
        )
        .unwrap();

        let output = execute_capability(&skill_path, "inspect", &[], &serde_json::json!({}), None)
            .await
            .unwrap();
        std::env::remove_var(key);

        assert_eq!(output, "unset");
    }

    #[test]
    fn expired_tokens_are_removed_before_injection() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("expired.md");
        std::fs::write(&skill_path, "### noop\n```bash\ntrue\n```\n").unwrap();
        let content = std::fs::read_to_string(&skill_path).unwrap();
        let scope = skill_token_scope(&skill_path, &content);
        let now = Instant::now();

        TOKEN_CACHE.insert(
            SkillTokenCacheKey {
                scope: scope.clone(),
                token_name: "expired_token".to_string(),
            },
            CachedSkillToken {
                value: Zeroizing::new("must-not-leak".to_string()),
                expires_at: now.checked_sub(Duration::from_secs(1)).unwrap(),
            },
        );

        assert!(cached_tokens_for_scope(&scope, now).is_empty());
        assert!(
            !TOKEN_CACHE.iter().any(|entry| entry.key().scope == scope),
            "expired scoped entries must be evicted"
        );
    }

    #[test]
    fn changing_skill_source_invalidates_cached_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("mutable.md");
        let original = "### login\n```bash\necho original\n```\n";
        let modified = "### login\n```bash\necho modified\n```\n";

        let original_scope = skill_token_scope(&skill_path, original);
        let modified_scope = skill_token_scope(&skill_path, modified);

        assert_ne!(original_scope, modified_scope);
    }
}
