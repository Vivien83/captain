//! Skill loader — loads and executes skills from various runtimes.

use crate::{SkillError, SkillManifest, SkillRuntime, SkillToolResult};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error};

/// Resolve Captain's home directory for secrets lookup.
fn captain_home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// B.3 — Inject the secrets a skill explicitly declared in its manifest.
///
/// Captain's `~/.captain/secrets.env` holds every credential the user has
/// stored — service logins, API tokens, SSH keys… Without this filter,
/// every skill subprocess would either get all of them (env inheritance)
/// or none (full env_clear). Per-skill `env_inject` lets a skill receive
/// **only** the entries it asked for, mapped to the variable name its
/// script expects:
///
/// ```toml
/// [requirements.env_inject]
/// SERVICE_USERNAME = "SERVICE_USER"
/// SERVICE_PASSWORD = "SERVICE_PASS"
/// ```
///
/// Returns the number of variables actually injected (useful for tests).
pub(crate) fn inject_env_from_manifest_secrets(
    cmd: &mut tokio::process::Command,
    manifest: &SkillManifest,
    home_dir: &Path,
) -> usize {
    if manifest.requirements.env_inject.is_empty() {
        return 0;
    }
    let secrets_path = home_dir.join(".captain/secrets.env");
    let content = match std::fs::read_to_string(&secrets_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let secrets: std::collections::HashMap<&str, &str> = content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            l.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
        })
        .collect();
    let mut injected = 0usize;
    for (secret_key, target_var) in &manifest.requirements.env_inject {
        if let Some(value) = secrets.get(secret_key.as_str()) {
            cmd.env(target_var, value);
            injected += 1;
        }
    }
    injected
}

/// Resolve a skill's entry-point path from its (untrusted) manifest.
///
/// `entry` comes straight from the skill's `SKILL.toml` — a file a
/// third-party author controls, potentially downloaded from ClawHub. Naive
/// `skill_dir.join(entry)` lets a malicious `entry` such as `/etc/passwd`
/// (absolute — `Path::join` discards `skill_dir` entirely) or
/// `../../other_skill/secret.py` (parent traversal) point execution
/// anywhere on disk. This rejects both shapes up front, then — as defense
/// in depth against symlink escapes not covered by the two checks — makes
/// sure the canonicalized result still lives inside the canonicalized
/// `skill_dir` before handing it back to the caller.
fn resolve_skill_entry_path(skill_dir: &Path, entry: &str) -> Result<PathBuf, SkillError> {
    let entry_path = Path::new(entry);

    if entry_path.is_absolute() {
        return Err(SkillError::SecurityBlocked(format!(
            "Skill entry point must be a relative path, got absolute path: {entry}"
        )));
    }

    if entry_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(SkillError::SecurityBlocked(format!(
            "Skill entry point must not contain '..' components: {entry}"
        )));
    }

    let candidate = skill_dir.join(entry_path);

    // If the file doesn't exist yet, there's nothing to canonicalize —
    // the checks above already guarantee the lexical path stays under
    // `skill_dir` (no absolute prefix, no `..`). Callers check
    // existence next and surface a clear "not found" error.
    if !candidate.exists() {
        return Ok(candidate);
    }

    let canonical_skill_dir = skill_dir.canonicalize().map_err(|e| {
        SkillError::ExecutionFailed(format!("Failed to resolve skill directory: {e}"))
    })?;
    let canonical_candidate = candidate.canonicalize().map_err(|e| {
        SkillError::ExecutionFailed(format!("Failed to resolve skill entry point: {e}"))
    })?;

    if !canonical_candidate.starts_with(&canonical_skill_dir) {
        return Err(SkillError::SecurityBlocked(format!(
            "Skill entry point resolves outside the skill directory: {entry}"
        )));
    }

    Ok(canonical_candidate)
}

/// Execute a skill tool by spawning the appropriate runtime.
pub async fn execute_skill_tool(
    manifest: &SkillManifest,
    skill_dir: &Path,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    // Verify the tool exists in the manifest
    let _tool_def = manifest
        .tools
        .provided
        .iter()
        .find(|t| t.name == tool_name)
        .ok_or_else(|| SkillError::NotFound(format!("Tool {tool_name} not in skill manifest")))?;

    match manifest.runtime.runtime_type {
        SkillRuntime::Python => execute_python(skill_dir, manifest, tool_name, input).await,
        SkillRuntime::Node => execute_node(skill_dir, manifest, tool_name, input).await,
        SkillRuntime::Shell => execute_shell(skill_dir, manifest, tool_name, input).await,
        SkillRuntime::Wasm => Err(SkillError::RuntimeNotAvailable(
            "WASM skill runtime not yet implemented".to_string(),
        )),
        SkillRuntime::Builtin => Err(SkillError::RuntimeNotAvailable(
            "Builtin skills are handled by the kernel directly".to_string(),
        )),
        SkillRuntime::PromptOnly => {
            // Prompt-only skills inject context into the system prompt.
            // When a tool call arrives here, guide the LLM to use built-in tools.
            Ok(SkillToolResult {
                output: serde_json::json!({
                    "note": "Prompt-context skill — instructions are in your system prompt. Use built-in tools directly."
                }),
                is_error: false,
            })
        }
    }
}

/// Execute a Python skill script.
async fn execute_python(
    skill_dir: &Path,
    manifest: &SkillManifest,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    let entry = manifest.runtime.entry.as_str();
    let script_path = resolve_skill_entry_path(skill_dir, entry)?;
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Python script not found: {}",
            script_path.display()
        )));
    }

    // Build the JSON payload to send via stdin
    let payload = serde_json::json!({
        "tool": tool_name,
        "input": input,
    });

    let python = find_python().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Python not found. Install Python 3.8+ to run Python skills.".to_string(),
        )
    })?;

    debug!(
        "Executing Python skill: {} {}",
        python,
        script_path.display()
    );

    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Isolate environment to prevent secret leakage.
    // Skills are third-party code — they must not inherit API keys,
    // tokens, or credentials from the host environment.
    cmd.env_clear();
    // Preserve PATH for binary resolution and platform essentials
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    {
        if let Ok(sp) = std::env::var("SYSTEMROOT") {
            cmd.env("SYSTEMROOT", sp);
        }
        if let Ok(tmp) = std::env::var("TEMP") {
            cmd.env("TEMP", tmp);
        }
    }
    // Python needs PYTHONIOENCODING for UTF-8 output
    cmd.env("PYTHONIOENCODING", "utf-8");

    // B.3 — only the secrets this skill explicitly declared cross the env
    // boundary; every other key in secrets.env stays invisible.
    inject_env_from_manifest_secrets(&mut cmd, manifest, &captain_home_dir());

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn Python: {e}")))?;

    // Write input to stdin
    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        stdin
            .write_all(&payload_bytes)
            .await
            .map_err(|e| SkillError::ExecutionFailed(format!("Write stdin: {e}")))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("Wait for Python: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Python skill failed: {stderr}");
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    // Parse stdout as JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Execute a Node.js skill script.
async fn execute_node(
    skill_dir: &Path,
    manifest: &SkillManifest,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    let entry = manifest.runtime.entry.as_str();
    let script_path = resolve_skill_entry_path(skill_dir, entry)?;
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Node.js script not found: {}",
            script_path.display()
        )));
    }

    let node = find_node().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Node.js not found. Install Node.js 18+ to run Node skills.".to_string(),
        )
    })?;

    let payload = serde_json::json!({
        "tool": tool_name,
        "input": input,
    });

    debug!(
        "Executing Node.js skill: {} {}",
        node,
        script_path.display()
    );

    let mut cmd = tokio::process::Command::new(&node);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Isolate environment (same as Python — prevent secret leakage)
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    {
        if let Ok(sp) = std::env::var("SYSTEMROOT") {
            cmd.env("SYSTEMROOT", sp);
        }
        if let Ok(tmp) = std::env::var("TEMP") {
            cmd.env("TEMP", tmp);
        }
    }
    // Node needs NODE_PATH sometimes
    cmd.env("NODE_NO_WARNINGS", "1");

    // B.3 — per-skill secret injection.
    inject_env_from_manifest_secrets(&mut cmd, manifest, &captain_home_dir());

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn Node.js: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        stdin
            .write_all(&payload_bytes)
            .await
            .map_err(|e| SkillError::ExecutionFailed(format!("Write stdin: {e}")))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("Wait for Node.js: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Find Python 3 binary.
fn find_python() -> Option<String> {
    for name in &["python3", "python"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Find Node.js binary.
fn find_node() -> Option<String> {
    if std::process::Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some("node".to_string());
    }
    None
}

/// Find Shell/Bash binary.
fn find_shell() -> Option<String> {
    // Try bash first, then sh as fallback
    for name in &["bash", "sh"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Execute a Shell/Bash skill script.
async fn execute_shell(
    skill_dir: &Path,
    manifest: &SkillManifest,
    _tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    let entry = manifest.runtime.entry.as_str();
    let script_path = resolve_skill_entry_path(skill_dir, entry)?;
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Shell script not found: {}",
            script_path.display()
        )));
    }

    let shell = find_shell().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Shell/Bash not found. Install bash or sh to run Shell skills.".to_string(),
        )
    })?;

    debug!("Executing Shell skill: {} {}", shell, script_path.display());

    // Execute the shell script directly, passing action/params as env vars.
    // Input JSON fields are mapped to ACTION, SLOT_ID, START, END env vars.
    // Credentials from ~/.captain/secrets.env are injected as CF_USER / CF_PASS.
    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Isolate environment to prevent accidental secret leakage.
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    {
        if let Ok(sp) = std::env::var("SYSTEMROOT") {
            cmd.env("SYSTEMROOT", sp);
        }
        if let Ok(tmp) = std::env::var("TEMP") {
            cmd.env("TEMP", tmp);
        }
    }

    // Map JSON input fields to env vars for shell dispatch
    if let Some(action) = input.get("action").and_then(|v| v.as_str()) {
        cmd.env("ACTION", action);
    }
    if let Some(slot_id) = input.get("slot_id").and_then(|v| v.as_str()) {
        cmd.env("SLOT_ID", slot_id);
    }
    if let Some(start) = input.get("start").and_then(|v| v.as_str()) {
        cmd.env("START", start);
    }
    if let Some(end) = input.get("end").and_then(|v| v.as_str()) {
        cmd.env("END", end);
    }

    // B.3 — per-skill secret injection. Skills declare in their manifest
    // which secrets cross the env boundary; nothing else from secrets.env
    // is exposed to the subprocess.
    inject_env_from_manifest_secrets(&mut cmd, manifest, &captain_home_dir());

    let child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn shell: {e}")))?;

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("Wait for shell: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Shell skill failed: {stderr}");
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    // Parse stdout as JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_python() {
        // Just ensure it doesn't panic — result depends on environment
        let _ = find_python();
    }

    #[test]
    fn test_find_node() {
        let _ = find_node();
    }

    #[tokio::test]
    async fn test_prompt_only_execution() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-prompt".to_string(),
                version: "0.1.0".to_string(),
                description: "A prompt-only test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::PromptOnly,
                entry: String::new(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "test_tool".to_string(),
                    description: "Test".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: Some("You are a helpful assistant.".to_string()),
            source: None,
        };

        let result = execute_skill_tool(&manifest, dir.path(), "test_tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!result.is_error);
        let note = result.output["note"].as_str().unwrap();
        assert!(note.contains("system prompt"));
    }

    // ── B.3 — per-skill secret injection ──────────────────────────────

    fn write_secrets(home: &Path, body: &str) {
        std::fs::create_dir_all(home.join(".captain")).unwrap();
        std::fs::write(home.join(".captain/secrets.env"), body).unwrap();
    }

    fn manifest_with_inject(pairs: &[(&str, &str)]) -> crate::SkillManifest {
        use crate::{SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillTools};
        let mut env_inject = std::collections::BTreeMap::new();
        for (k, v) in pairs {
            env_inject.insert(k.to_string(), v.to_string());
        }
        SkillManifest {
            skill: SkillMeta {
                name: "test-skill".to_string(),
                version: "0.1.0".to_string(),
                description: String::new(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: crate::SkillRuntime::Shell,
                entry: "run.sh".to_string(),
            },
            tools: SkillTools::default(),
            requirements: SkillRequirements {
                tools: vec![],
                capabilities: vec![],
                env_inject,
            },
            prompt_context: None,
            source: None,
        }
    }

    /// B.3 — Skills with no env_inject must receive zero secrets, even when
    /// secrets.env on disk is full of credentials.
    #[test]
    fn inject_env_returns_zero_when_manifest_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        write_secrets(dir.path(), "FOO=bar\nBAZ=qux\n");
        let manifest = manifest_with_inject(&[]);
        let mut cmd = tokio::process::Command::new("true");
        let n = inject_env_from_manifest_secrets(&mut cmd, &manifest, dir.path());
        assert_eq!(n, 0, "no env_inject = no injection");
    }

    /// B.3 — Only declared secrets cross the env boundary; foreign keys
    /// stay invisible to the skill.
    #[test]
    fn inject_env_only_returns_declared_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        write_secrets(
            dir.path(),
            "SERVICE_USERNAME=alice\nSERVICE_PASSWORD=s3cret\nGITHUB_TOKEN=ghp_xxx\n",
        );
        let manifest = manifest_with_inject(&[
            ("SERVICE_USERNAME", "APP_USER"),
            ("SERVICE_PASSWORD", "APP_PASS"),
        ]);
        let mut cmd = tokio::process::Command::new("true");
        let n = inject_env_from_manifest_secrets(&mut cmd, &manifest, dir.path());
        assert_eq!(n, 2, "exactly the two declared pairs were injected");
    }

    /// B.3 — A declared secret that is missing from secrets.env is silently
    /// skipped (not injected, no error). Skills must handle the absence
    /// themselves rather than have us inject empty strings.
    #[test]
    fn inject_env_skips_missing_secret_silently() {
        let dir = tempfile::TempDir::new().unwrap();
        write_secrets(dir.path(), "ONLY_THIS=x\n");
        let manifest =
            manifest_with_inject(&[("ONLY_THIS", "TARGET_A"), ("MISSING_KEY", "TARGET_B")]);
        let mut cmd = tokio::process::Command::new("true");
        let n = inject_env_from_manifest_secrets(&mut cmd, &manifest, dir.path());
        assert_eq!(n, 1, "only the present secret is injected");
    }

    /// B.3 — When secrets.env doesn't exist at all, injection is a no-op
    /// (zero return) instead of crashing the skill spawn.
    #[test]
    fn inject_env_handles_missing_secrets_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_inject(&[("FOO", "BAR")]);
        let mut cmd = tokio::process::Command::new("true");
        let n = inject_env_from_manifest_secrets(&mut cmd, &manifest, dir.path());
        assert_eq!(n, 0, "missing secrets file must not crash");
    }

    // ── Path traversal via manifest `runtime.entry` ─────────────────────

    fn manifest_with_entry(runtime_type: crate::SkillRuntime, entry: &str) -> crate::SkillManifest {
        use crate::{SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillTools};
        SkillManifest {
            skill: SkillMeta {
                name: "test-skill".to_string(),
                version: "0.1.0".to_string(),
                description: String::new(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type,
                entry: entry.to_string(),
            },
            tools: SkillTools::default(),
            requirements: SkillRequirements::default(),
            prompt_context: None,
            source: None,
        }
    }

    #[test]
    fn resolve_entry_rejects_absolute_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = resolve_skill_entry_path(dir.path(), "/etc/passwd").unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    #[test]
    fn resolve_entry_rejects_parent_dir_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = resolve_skill_entry_path(dir.path(), "../../../etc/passwd").unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    #[test]
    fn resolve_entry_rejects_embedded_parent_dir_component() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let err = resolve_skill_entry_path(dir.path(), "sub/../../escape.py").unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    #[test]
    fn resolve_entry_accepts_normal_relative_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.py"), "print('ok')").unwrap();
        let resolved = resolve_skill_entry_path(dir.path(), "main.py").unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("main.py"));
    }

    #[test]
    fn resolve_entry_accepts_nonexistent_relative_path() {
        // Not-yet-existing entries are allowed through so the caller's
        // existing "script not found" error keeps firing with a clear
        // message, instead of a canonicalize failure here.
        let dir = tempfile::TempDir::new().unwrap();
        let resolved = resolve_skill_entry_path(dir.path(), "missing.py").unwrap();
        assert_eq!(resolved, dir.path().join("missing.py"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_entry_rejects_symlink_escape() {
        let dir = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.py"), "print('leak')").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.py"), dir.path().join("main.py"))
            .unwrap();

        let err = resolve_skill_entry_path(dir.path(), "main.py").unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Python runtime — malicious absolute entry is rejected before any
    /// process is ever spawned.
    #[tokio::test]
    async fn execute_python_rejects_absolute_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Python, "/etc/passwd");
        let err = execute_python(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Python runtime — `..` traversal in entry is rejected before any
    /// process is ever spawned.
    #[tokio::test]
    async fn execute_python_rejects_dotdot_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Python, "../../../etc/passwd");
        let err = execute_python(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Python runtime — non-regression: a normal relative entry still runs
    /// exactly as before the fix.
    #[tokio::test]
    async fn execute_python_normal_entry_still_works() {
        if find_python().is_none() {
            eprintln!("skipping: no python interpreter available");
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.py"),
            "import sys, json\nsys.stdin.read()\nprint(json.dumps({'ok': True}))\n",
        )
        .unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Python, "main.py");
        let result = execute_python(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["ok"], serde_json::json!(true));
    }

    /// Node.js runtime — malicious absolute entry is rejected before any
    /// process is ever spawned.
    #[tokio::test]
    async fn execute_node_rejects_absolute_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Node, "/etc/passwd");
        let err = execute_node(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Node.js runtime — `..` traversal in entry is rejected before any
    /// process is ever spawned.
    #[tokio::test]
    async fn execute_node_rejects_dotdot_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Node, "../../../etc/passwd");
        let err = execute_node(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Node.js runtime — non-regression: a normal relative entry still runs
    /// exactly as before the fix.
    #[tokio::test]
    async fn execute_node_normal_entry_still_works() {
        if find_node().is_none() {
            eprintln!("skipping: no node interpreter available");
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.js"),
            "process.stdin.resume(); process.stdin.on('end', () => console.log(JSON.stringify({ok: true})));",
        )
        .unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Node, "main.js");
        let result = execute_node(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["ok"], serde_json::json!(true));
    }

    /// Shell runtime — malicious absolute entry is rejected before any
    /// process is ever spawned.
    #[tokio::test]
    async fn execute_shell_rejects_absolute_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Shell, "/etc/passwd");
        let err = execute_shell(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Shell runtime — `..` traversal in entry is rejected before any
    /// process is ever spawned.
    #[tokio::test]
    async fn execute_shell_rejects_dotdot_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Shell, "../../../etc/passwd");
        let err = execute_shell(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::SecurityBlocked(_)));
    }

    /// Shell runtime — non-regression: a normal relative entry still runs
    /// exactly as before the fix.
    #[tokio::test]
    async fn execute_shell_normal_entry_still_works() {
        if find_shell().is_none() {
            eprintln!("skipping: no shell interpreter available");
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.sh"),
            "#!/bin/sh\ncat >/dev/null\necho '{\"ok\": true}'\n",
        )
        .unwrap();
        let manifest = manifest_with_entry(crate::SkillRuntime::Shell, "main.sh");
        let result = execute_shell(dir.path(), &manifest, "tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["ok"], serde_json::json!(true));
    }
}
