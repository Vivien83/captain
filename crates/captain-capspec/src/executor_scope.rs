use crate::{
    render_template, CompiledCapability, CompiledStep, ExecutorError, PermissionSet,
    TemplateContext,
};
use globset::GlobBuilder;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Component, Path};

pub(crate) fn rendered_permissions(
    capability: &CompiledCapability,
    run_id: &str,
    input: &Map<String, Value>,
) -> Result<PermissionSet, ExecutorError> {
    let context = TemplateContext {
        run_id,
        input,
        step_outputs: &BTreeMap::new(),
    };
    let mut permissions = capability.permissions.clone();
    render_list(&mut permissions.read_paths, &context)?;
    render_list(&mut permissions.write_paths, &context)?;
    render_list(&mut permissions.network_hosts, &context)?;
    render_list(&mut permissions.ssh_hosts, &context)?;
    render_list(&mut permissions.shell_commands, &context)?;
    render_list(&mut permissions.memory_read, &context)?;
    render_list(&mut permissions.memory_write, &context)?;
    permissions.normalize();
    Ok(permissions)
}

fn render_list(values: &mut [String], context: &TemplateContext<'_>) -> Result<(), ExecutorError> {
    for value in values {
        let rendered = render_template(&Value::String(value.clone()), context)?;
        *value = rendered
            .as_str()
            .ok_or_else(|| {
                ExecutorError::InvalidState("permission template must render to text".to_string())
            })?
            .to_string();
    }
    Ok(())
}

pub(crate) fn validate_step_scope(
    step: &CompiledStep,
    input: &Value,
    permissions: &PermissionSet,
    workspace: Option<&Path>,
) -> Result<(), ExecutorError> {
    let denied = |reason: String| ExecutorError::ScopeDenied {
        step_id: step.id.clone(),
        tool: step.tool.clone(),
        reason,
    };
    let result = match step.tool.as_str() {
        "file_read" | "file_list" | "grep" | "glob" => {
            validate_path_field(input, "path", ".", &permissions.read_paths, workspace)
        }
        "file_inspect_batch" => validate_inspect_batch(input, &permissions.read_paths, workspace),
        "file_write" | "edit_file" | "multi_edit" => {
            validate_path_field(input, "path", "", &permissions.write_paths, workspace)
        }
        "apply_patch" => validate_patch_paths(input, &permissions.write_paths, workspace),
        "web_fetch" | "browser_navigate" => validate_network_url(input, &permissions.network_hosts),
        "web_download" => validate_network_url(input, &permissions.network_hosts).and_then(|_| {
            validate_path_field(input, "path", "", &permissions.write_paths, workspace)
        }),
        "web_search" => validate_named_scope("search", &permissions.network_hosts, "network"),
        tool if tool.starts_with("ssh_") => validate_text_field(
            input,
            &["host", "alias"],
            &permissions.ssh_hosts,
            "SSH host",
        ),
        "shell_exec" | "execute_code" | "cargo" | "npm" | "pip" => {
            validate_shell(step, input, &permissions.shell_commands)
        }
        "memory_recall" | "memory_context_batch" | "session_recall" => {
            validate_memory(input, &permissions.memory_read, false)
        }
        "memory_save" | "memory_store" | "memory_forget" => {
            validate_memory(input, &permissions.memory_write, true)
        }
        "secret_read" => {
            validate_text_field(input, &["key"], &permissions.secrets, "secret identifier")
        }
        _ => Ok(()),
    };
    result.map_err(denied)
}

fn validate_path_field(
    input: &Value,
    field: &str,
    default: &str,
    scopes: &[String],
    workspace: Option<&Path>,
) -> Result<(), String> {
    let path = input.get(field).and_then(Value::as_str).unwrap_or(default);
    if path.is_empty() {
        return Err(format!("missing path field '{field}'"));
    }
    validate_path(path, scopes, workspace)
}

fn validate_inspect_batch(
    input: &Value,
    scopes: &[String],
    workspace: Option<&Path>,
) -> Result<(), String> {
    let operations = input
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| "file_inspect_batch requires operations".to_string())?;
    for operation in operations {
        let path = operation.get("path").and_then(Value::as_str).unwrap_or(".");
        validate_path(path, scopes, workspace)?;
    }
    Ok(())
}

fn validate_patch_paths(
    input: &Value,
    scopes: &[String],
    workspace: Option<&Path>,
) -> Result<(), String> {
    let patch = input
        .get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| "apply_patch requires patch text".to_string())?;
    let mut found = 0usize;
    for line in patch.lines() {
        let path = [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix));
        if let Some(path) = path {
            found += 1;
            validate_path(path.trim(), scopes, workspace)?;
        }
    }
    if found == 0 {
        return Err("apply_patch contains no auditable file path".to_string());
    }
    Ok(())
}

fn validate_path(raw: &str, scopes: &[String], workspace: Option<&Path>) -> Result<(), String> {
    if raw.contains('\0') || raw.chars().any(|ch| ch == '\r' || ch == '\n') {
        return Err("path contains control characters".to_string());
    }
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!("path traversal is forbidden: '{raw}'"));
    }
    let normalized = raw.replace('\\', "/");
    let absolute = if path.is_absolute() {
        Some(normalized.clone())
    } else {
        workspace.map(|root| root.join(path).to_string_lossy().replace('\\', "/"))
    };
    if scopes.iter().any(|scope| {
        scope_matches(scope, &normalized)
            || absolute
                .as_deref()
                .is_some_and(|value| scope_matches(scope, value))
    }) {
        Ok(())
    } else {
        Err(format!("path '{raw}' is outside declared scopes"))
    }
}

fn validate_network_url(input: &Value, scopes: &[String]) -> Result<(), String> {
    let raw = input
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing URL field 'url'".to_string())?;
    let url = url::Url::parse(raw).map_err(|error| format!("invalid URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("URL scheme '{}' is not allowed", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("credentials in URLs are forbidden".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_ascii_lowercase();
    if scopes.iter().any(|scope| host_matches(scope, &host)) {
        Ok(())
    } else {
        Err(format!("network host '{host}' is outside declared scopes"))
    }
}

fn host_matches(scope: &str, host: &str) -> bool {
    let scope = scope.trim().to_ascii_lowercase();
    match scope.strip_prefix("*.") {
        Some(suffix) => host != suffix && host.ends_with(&format!(".{suffix}")),
        None => scope == host,
    }
}

fn validate_shell(step: &CompiledStep, input: &Value, scopes: &[String]) -> Result<(), String> {
    let command = if let Some(command) = input.get("command").and_then(Value::as_str) {
        command.to_string()
    } else if step.tool == "execute_code" {
        input
            .get("code")
            .and_then(Value::as_str)
            .ok_or_else(|| "execute_code requires code".to_string())?
            .to_string()
    } else {
        let subcommand = input
            .get("subcommand")
            .or_else(|| input.get("command"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} requires a command", step.tool))?;
        let args = input
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        format!("{} {} {}", step.tool, subcommand, args)
            .trim()
            .to_string()
    };
    if command.contains('\0') || command.contains('\n') || command.contains('\r') {
        return Err("shell command contains forbidden control characters".to_string());
    }
    if scopes.iter().any(|scope| scope_matches(scope, &command)) {
        Ok(())
    } else {
        Err("shell command is outside declared scopes".to_string())
    }
}

fn validate_memory(input: &Value, scopes: &[String], write: bool) -> Result<(), String> {
    let target = if write {
        let room = input.get("room").and_then(Value::as_str).unwrap_or("*");
        let subject = input.get("subject").and_then(Value::as_str).unwrap_or("*");
        let predicate = input
            .get("predicate")
            .and_then(Value::as_str)
            .unwrap_or("*");
        format!("{room}/{subject}/{predicate}")
    } else {
        input
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("*")
            .to_string()
    };
    if scopes.iter().any(|scope| scope_matches(scope, &target)) {
        Ok(())
    } else {
        Err(format!(
            "memory target '{target}' is outside declared scopes"
        ))
    }
}

fn validate_text_field(
    input: &Value,
    fields: &[&str],
    scopes: &[String],
    label: &str,
) -> Result<(), String> {
    let value = fields
        .iter()
        .find_map(|field| input.get(*field).and_then(Value::as_str))
        .ok_or_else(|| format!("missing {label}"))?;
    if scopes.iter().any(|scope| scope_matches(scope, value)) {
        Ok(())
    } else {
        Err(format!("{label} '{value}' is outside declared scopes"))
    }
}

fn validate_named_scope(value: &str, scopes: &[String], label: &str) -> Result<(), String> {
    if scopes.iter().any(|scope| scope_matches(scope, value)) {
        Ok(())
    } else {
        Err(format!("{label} scope '{value}' is not declared"))
    }
}

fn scope_matches(scope: &str, value: &str) -> bool {
    if scope == value {
        return true;
    }
    GlobBuilder::new(&scope.replace('\\', "/"))
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map(|glob| glob.compile_matcher().is_match(value))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Effect, Idempotency, RetryPolicy};
    use serde_json::json;

    fn step(tool: &str) -> CompiledStep {
        CompiledStep {
            id: "test".to_string(),
            tool: tool.to_string(),
            needs: Vec::new(),
            input: json!({}),
            effect: Effect::Read,
            idempotency: Idempotency::Safe,
            idempotency_key: None,
            timeout_secs: 10,
            retry: RetryPolicy::default(),
        }
    }

    #[test]
    fn path_scope_rejects_traversal_before_matching() {
        let permissions = PermissionSet {
            read_paths: vec!["**".to_string()],
            ..PermissionSet::default()
        };
        let error = validate_step_scope(
            &step("file_read"),
            &json!({"path": "../secret"}),
            &permissions,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("traversal"));
    }

    #[test]
    fn wildcard_host_does_not_match_apex_or_suffix_confusion() {
        let scopes = vec!["*.example.com".to_string()];
        assert!(
            validate_network_url(&json!({"url": "https://api.example.com/v1"}), &scopes).is_ok()
        );
        assert!(validate_network_url(&json!({"url": "https://example.com"}), &scopes).is_err());
        assert!(validate_network_url(
            &json!({"url": "https://example.com.attacker.test"}),
            &scopes
        )
        .is_err());
    }

    #[test]
    fn patch_checks_every_target_path() {
        let permissions = PermissionSet {
            write_paths: vec!["src/**".to_string()],
            ..PermissionSet::default()
        };
        let patch =
            "*** Begin Patch\n*** Update File: src/lib.rs\n*** Move to: ../leak\n*** End Patch";
        assert!(validate_step_scope(
            &step("apply_patch"),
            &json!({"patch": patch}),
            &permissions,
            None,
        )
        .is_err());
    }

    #[test]
    fn secret_identifier_requires_an_exact_declared_scope() {
        let permissions = PermissionSet {
            secrets: vec!["CAPTAIN_API_TOKEN".to_string()],
            ..PermissionSet::default()
        };
        assert!(validate_step_scope(
            &step("secret_read"),
            &json!({"key": "CAPTAIN_API_TOKEN"}),
            &permissions,
            None,
        )
        .is_ok());
        assert!(validate_step_scope(
            &step("secret_read"),
            &json!({"key": "OTHER_TOKEN"}),
            &permissions,
            None,
        )
        .is_err());
    }

    #[test]
    fn exact_shell_scope_treats_glob_metacharacters_literally() {
        let code = "const rows=[{amount:2999}];console.log(rows[0].amount);";
        let permissions = PermissionSet {
            shell_commands: vec![code.to_string()],
            ..PermissionSet::default()
        };
        assert!(validate_step_scope(
            &step("execute_code"),
            &json!({"language": "node", "code": code}),
            &permissions,
            None,
        )
        .is_ok());
    }
}
