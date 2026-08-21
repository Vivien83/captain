#![allow(dead_code)]

//! Workspace-scoped runtime adapter for durable Hub/Node tool runs.
//!
//! This is deliberately not a second tool engine. It reviews a narrow local
//! surface, then calls the same guarded runtime dispatcher as Captain with the
//! process-wide result cache disabled. Critical shell commands remain locally
//! denied: a remote approval is never converted into a critical shell permit.

use std::{fmt, fs, path::Path};

use captain_types::{
    approval::{approval_action_digest, RiskLevel},
    config::{ExecPolicy, ExecutionProfile},
    tool::ToolResult,
    tool_compat::normalize_tool_name,
};

use crate::{
    guarded_exec::{ExecSurface, ReviewDecision},
    tool_runner::{execute_tool_with_cache_mode, ToolCacheMode},
    work_verification::{classify_distributed_tool_effect, WorkEffect},
};

const LOCAL_NODE_FILE_TOOLS: &[&str] = &[
    "file_inspect_batch",
    "file_read",
    "file_write",
    "file_list",
    "glob",
    "grep",
    "edit_file",
    "multi_edit",
    "apply_patch",
];
const MAX_LOCAL_NODE_TOOL_INPUT_BYTES: usize = 1_048_576;
const MAX_LOCAL_NODE_RESULT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalNodeToolEffect {
    ReadOnly,
    LocalMutation,
    ExternalEffect,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalNodeToolReview {
    tool_name: String,
    family: &'static str,
    effect: LocalNodeToolEffect,
    action_digest: String,
    approval_required: bool,
    risk_level: RiskLevel,
}

impl LocalNodeToolReview {
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub const fn family(&self) -> &'static str {
        self.family
    }

    pub const fn effect(&self) -> LocalNodeToolEffect {
        self.effect
    }

    pub fn action_digest(&self) -> &str {
        &self.action_digest
    }

    pub const fn approval_required(&self) -> bool {
        self.approval_required
    }

    pub const fn risk_level(&self) -> RiskLevel {
        self.risk_level
    }

    /// Sanitized text suitable for a Hub approval card. Tool input and local
    /// paths are intentionally absent.
    pub fn approval_summary(&self) -> String {
        format!(
            "Authorize one {} run of `{}` on the selected Node workspace.",
            match self.effect {
                LocalNodeToolEffect::ReadOnly => "read-only",
                LocalNodeToolEffect::LocalMutation => "local-mutation",
                LocalNodeToolEffect::ExternalEffect => "external-effect",
            },
            self.tool_name
        )
    }
}

impl fmt::Debug for LocalNodeToolReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalNodeToolReview")
            .field("tool_name", &self.tool_name)
            .field("family", &self.family)
            .field("effect", &self.effect)
            .field("action_digest", &self.action_digest)
            .field("approval_required", &self.approval_required)
            .field("risk_level", &self.risk_level)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalNodeToolOutput {
    succeeded: bool,
    content: String,
    total_output_bytes: u64,
    capped: bool,
    redacted: bool,
}

impl LocalNodeToolOutput {
    pub const fn succeeded(&self) -> bool {
        self.succeeded
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub const fn total_output_bytes(&self) -> u64 {
        self.total_output_bytes
    }

    pub const fn capped(&self) -> bool {
        self.capped
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }

    pub fn into_parts(self) -> (bool, String, u64, bool, bool) {
        (
            self.succeeded,
            self.content,
            self.total_output_bytes,
            self.capped,
            self.redacted,
        )
    }
}

impl fmt::Debug for LocalNodeToolOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalNodeToolOutput")
            .field("succeeded", &self.succeeded)
            .field("content", &"[REDACTED]")
            .field("total_output_bytes", &self.total_output_bytes)
            .field("stored_output_bytes", &self.content.len())
            .field("capped", &self.capped)
            .field("redacted", &self.redacted)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LocalNodeToolRejection {
    code: &'static str,
    message: &'static str,
    retryable: bool,
}

impl LocalNodeToolRejection {
    const fn permanent(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
        }
    }

    const fn retryable(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: true,
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub const fn message(&self) -> &'static str {
        self.message
    }

    pub const fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Debug for LocalNodeToolRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalNodeToolRejection")
            .field("code", &self.code)
            .field("message", &"[REDACTED]")
            .field("retryable", &self.retryable)
            .finish()
    }
}

impl fmt::Display for LocalNodeToolRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for LocalNodeToolRejection {}

pub struct LocalNodeToolExecution<'a> {
    pub tool_use_id: &'a str,
    pub tool_name: &'a str,
    pub input: &'a serde_json::Value,
    pub workspace_id: &'a str,
    pub workspace_root: &'a Path,
    pub exec_policy: &'a ExecPolicy,
    pub approved_action_digest: Option<&'a str>,
}

impl fmt::Debug for LocalNodeToolExecution<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalNodeToolExecution")
            .field("tool_use_id", &self.tool_use_id)
            .field("tool_name", &self.tool_name)
            .field("input", &"[REDACTED]")
            .field("workspace_id", &self.workspace_id)
            .field("workspace_root", &"[REDACTED]")
            .field(
                "approved_action_digest",
                &self.approved_action_digest.map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

pub fn review_local_node_tool(
    tool_name: &str,
    input: &serde_json::Value,
    exec_policy: &ExecPolicy,
) -> Result<LocalNodeToolReview, LocalNodeToolRejection> {
    if !input.is_object() {
        return Err(LocalNodeToolRejection::permanent(
            "invalid_tool_input",
            "The Node tool input must be a JSON object",
        ));
    }
    if normalize_tool_name(tool_name) != tool_name {
        return Err(LocalNodeToolRejection::permanent(
            "non_canonical_tool_name",
            "The Hub must offer the exact canonical runtime tool name",
        ));
    }
    let family = local_node_tool_family(tool_name).ok_or_else(|| {
        LocalNodeToolRejection::permanent(
            "unsupported_local_tool",
            "This runtime tool is not exposed on the local Node execution rail",
        )
    })?;
    let input_bytes = serde_json::to_vec(input).map_err(|_| {
        LocalNodeToolRejection::permanent(
            "invalid_tool_input",
            "The Node tool input could not be bound to an approval digest",
        )
    })?;
    if input_bytes.len() > MAX_LOCAL_NODE_TOOL_INPUT_BYTES {
        return Err(LocalNodeToolRejection::permanent(
            "tool_input_too_large",
            "The Node tool input exceeds the durable rail limit",
        ));
    }
    validate_local_input(tool_name, input)?;
    if tool_name == "shell_exec" {
        // Preserve the local execution policy's stronger classification for
        // critical commands before applying the rail's path disclosure guard.
        review_node_shell(input, exec_policy)?;
    }
    if !local_node_input_uses_workspace_relative_paths(tool_name, input) {
        return Err(LocalNodeToolRejection::permanent(
            "path_policy_violation",
            "Node tool paths must remain relative to the selected logical workspace",
        ));
    }

    let effect = local_node_tool_effect(tool_name, input).ok_or_else(|| {
        LocalNodeToolRejection::permanent(
            "unsupported_local_tool",
            "This runtime tool is not exposed on the local Node execution rail",
        )
    })?;
    let approval_required = effect == LocalNodeToolEffect::ExternalEffect;
    Ok(LocalNodeToolReview {
        tool_name: tool_name.to_string(),
        family,
        effect,
        action_digest: approval_action_digest(tool_name, &input_bytes),
        approval_required,
        risk_level: match effect {
            LocalNodeToolEffect::ReadOnly => RiskLevel::Low,
            LocalNodeToolEffect::LocalMutation => RiskLevel::Medium,
            LocalNodeToolEffect::ExternalEffect => RiskLevel::High,
        },
    })
}

pub async fn execute_local_node_tool(
    request: LocalNodeToolExecution<'_>,
) -> Result<LocalNodeToolOutput, LocalNodeToolRejection> {
    let review = review_local_node_tool(request.tool_name, request.input, request.exec_policy)?;
    if review.approval_required {
        match request.approved_action_digest {
            None => {
                return Err(LocalNodeToolRejection::permanent(
                    "approval_required",
                    "The exact external-effect action requires durable operator approval",
                ))
            }
            Some(digest) if digest != review.action_digest => {
                return Err(LocalNodeToolRejection::permanent(
                    "approval_digest_mismatch",
                    "The durable approval does not authorize this exact tool input",
                ))
            }
            Some(_) => {}
        }
    }

    let workspace_root = fs::canonicalize(request.workspace_root).map_err(|_| {
        LocalNodeToolRejection::retryable(
            "workspace_unavailable",
            "The local Node workspace is not currently available",
        )
    })?;
    if !workspace_root.is_dir() {
        return Err(LocalNodeToolRejection::retryable(
            "workspace_unavailable",
            "The local Node workspace is not currently available",
        ));
    }
    validate_workspace_id(request.workspace_id)?;
    let allowed_tools = [request.tool_name.to_string()];
    let effective_policy = node_exec_policy(request.exec_policy);
    let result = execute_tool_with_cache_mode(
        request.tool_use_id,
        request.tool_name,
        request.input,
        None,
        Some(&allowed_tools),
        Some("captain-node"),
        None,
        None,
        None,
        None,
        Some(&[]),
        Some(&workspace_root),
        None,
        Some(&effective_policy),
        None,
        None,
        None,
        ToolCacheMode::Disabled,
    )
    .await;
    finalize_local_node_output(result, &workspace_root, request.workspace_id)
}

/// Canonical capability family for a tool supported by the local Node rail.
pub fn local_node_tool_family(tool_name: &str) -> Option<&'static str> {
    if LOCAL_NODE_FILE_TOOLS.contains(&tool_name) {
        Some("file")
    } else if tool_name == "shell_exec" {
        Some("shell-process")
    } else {
        None
    }
}

/// Deterministic side-effect class for a tool supported by the local Node rail.
pub fn local_node_tool_effect(
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<LocalNodeToolEffect> {
    local_node_tool_family(tool_name)?;
    Some(match classify_distributed_tool_effect(tool_name, input) {
        WorkEffect::Observation | WorkEffect::Verification => LocalNodeToolEffect::ReadOnly,
        WorkEffect::LocalMutation => LocalNodeToolEffect::LocalMutation,
        WorkEffect::DurableMutation | WorkEffect::ExternalEffect | WorkEffect::HumanInput => {
            LocalNodeToolEffect::ExternalEffect
        }
    })
}

/// Verify that every path-bearing argument can cross the Hub rail without
/// disclosing a physical Node path. This is called both before Hub persistence
/// and again by the Node runtime before execution.
pub fn local_node_input_uses_workspace_relative_paths(
    tool_name: &str,
    input: &serde_json::Value,
) -> bool {
    match tool_name {
        "file_read" | "file_write" | "file_list" | "edit_file" | "multi_edit" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(is_workspace_relative_path),
        "glob" => {
            input
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .is_some_and(is_workspace_relative_path)
                && optional_relative_path(input, "path")
        }
        "grep" => optional_relative_path(input, "path") && optional_relative_path(input, "glob"),
        "file_inspect_batch" => input
            .get("operations")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|operations| {
                operations.iter().all(|operation| {
                    let Some(action) = operation.get("action").and_then(serde_json::Value::as_str)
                    else {
                        return false;
                    };
                    match action {
                        "read" | "file_read" => {
                            local_node_input_uses_workspace_relative_paths("file_read", operation)
                        }
                        "list" | "file_list" => {
                            local_node_input_uses_workspace_relative_paths("file_list", operation)
                        }
                        "glob" => local_node_input_uses_workspace_relative_paths("glob", operation),
                        "grep" => local_node_input_uses_workspace_relative_paths("grep", operation),
                        _ => false,
                    }
                })
            }),
        "apply_patch" => input
            .get("patch")
            .and_then(serde_json::Value::as_str)
            .is_some_and(patch_paths_are_workspace_relative),
        "shell_exec" => input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .is_some_and(shell_paths_are_workspace_relative),
        _ => false,
    }
}

fn optional_relative_path(input: &serde_json::Value, field: &str) -> bool {
    input
        .get(field)
        .filter(|value| !value.is_null())
        .and_then(serde_json::Value::as_str)
        .is_none_or(is_workspace_relative_path)
}

fn is_workspace_relative_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.starts_with("~/")
        || value.starts_with("~\\")
        || value.starts_with("workspace://")
        || looks_like_windows_absolute_path(value)
    {
        return false;
    }
    !std::path::Path::new(value).components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    })
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn patch_paths_are_workspace_relative(patch: &str) -> bool {
    let Ok(operations) = crate::apply_patch::parse_patch(patch) else {
        return false;
    };
    operations.iter().all(|operation| match operation {
        crate::apply_patch::PatchOp::AddFile { path, .. }
        | crate::apply_patch::PatchOp::DeleteFile { path } => is_workspace_relative_path(path),
        crate::apply_patch::PatchOp::UpdateFile { path, move_to, .. } => {
            is_workspace_relative_path(path)
                && move_to.as_deref().is_none_or(is_workspace_relative_path)
        }
    })
}

fn shell_paths_are_workspace_relative(command: &str) -> bool {
    if contains_windows_absolute_path(command) {
        return false;
    }
    let Some(tokens) = shlex::split(command) else {
        return false;
    };
    tokens.iter().all(|token| {
        token
            .split([';', '|', '&', '(', ')'])
            .filter(|part| !part.is_empty())
            .all(|part| {
                let candidate = part
                    .trim_start_matches(['<', '>'])
                    .rsplit_once('=')
                    .map_or(part, |(_, value)| value);
                !candidate.starts_with("$HOME/")
                    && !candidate.starts_with("${HOME}/")
                    && !candidate.starts_with("%USERPROFILE%")
                    && is_workspace_relative_path(candidate)
            })
    })
}

fn contains_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(3).enumerate().any(|(index, window)| {
        window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && matches!(window[2], b'/' | b'\\')
            && (index == 0
                || bytes[index - 1].is_ascii_whitespace()
                || matches!(bytes[index - 1], b'\'' | b'"' | b'=' | b'(' | b';' | b'|'))
    })
}

fn review_node_shell(
    input: &serde_json::Value,
    exec_policy: &ExecPolicy,
) -> Result<(), LocalNodeToolRejection> {
    let command = input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let policy = node_exec_policy(exec_policy);
    match crate::guarded_exec::review_shell(
        ExecSurface::ShellTool,
        command,
        Some(&policy),
        true,
    ) {
        Ok(ReviewDecision::Proceed(_)) => Ok(()),
        Ok(ReviewDecision::ApprovalRequired { .. }) => Err(
            LocalNodeToolRejection::permanent(
                "critical_shell_denied",
                "Critical shell execution requires a local approval surface and is denied on a remote Node",
            ),
        ),
        Err(_) => Err(LocalNodeToolRejection::permanent(
            "execution_policy_denied",
            "The local Node execution policy denied this shell command",
        )),
    }
}

fn node_exec_policy(policy: &ExecPolicy) -> ExecPolicy {
    let mut effective = policy.clone();
    effective.profile = effective.profile.stricter(ExecutionProfile::RemoteOperator);
    effective
}

fn validate_local_input(
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<(), LocalNodeToolRejection> {
    let require_string = |field: &str| {
        input
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(invalid_input)
    };
    match tool_name {
        "file_read" | "file_list" => {
            require_string("path")?;
        }
        "file_write" => {
            require_string("path")?;
            input
                .get("content")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid_input)?;
        }
        "glob" | "grep" => {
            require_string("pattern")?;
        }
        "edit_file" => {
            require_string("path")?;
            input
                .get("old_string")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid_input)?;
            input
                .get("new_string")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid_input)?;
        }
        "multi_edit" => {
            require_string("path")?;
            let edits = input
                .get("edits")
                .and_then(serde_json::Value::as_array)
                .filter(|edits| !edits.is_empty())
                .ok_or_else(invalid_input)?;
            if edits.iter().any(|edit| {
                edit.get("old_string")
                    .and_then(serde_json::Value::as_str)
                    .is_none()
                    || edit
                        .get("new_string")
                        .and_then(serde_json::Value::as_str)
                        .is_none()
            }) {
                return Err(invalid_input());
            }
        }
        "apply_patch" => {
            require_string("patch")?;
        }
        "file_inspect_batch" => {
            let operations = input
                .get("operations")
                .and_then(serde_json::Value::as_array)
                .filter(|operations| !operations.is_empty() && operations.len() <= 30)
                .ok_or_else(invalid_input)?;
            for operation in operations {
                let action = operation
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .ok_or_else(invalid_input)?;
                let canonical = match action {
                    "glob" => "glob",
                    "grep" => "grep",
                    "read" | "file_read" => "file_read",
                    "list" | "file_list" => "file_list",
                    _ => return Err(invalid_input()),
                };
                validate_local_input(canonical, operation)?;
            }
        }
        "shell_exec" => {
            require_string("command")?;
        }
        _ => return Err(invalid_input()),
    }
    Ok(())
}

const fn invalid_input() -> LocalNodeToolRejection {
    LocalNodeToolRejection::permanent(
        "invalid_tool_input",
        "The Node tool input does not satisfy the local runtime contract",
    )
}

fn validate_workspace_id(workspace_id: &str) -> Result<(), LocalNodeToolRejection> {
    if workspace_id.is_empty()
        || workspace_id.len() > 128
        || !workspace_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        Err(LocalNodeToolRejection::permanent(
            "invalid_workspace_id",
            "The logical Node workspace identifier is invalid",
        ))
    } else {
        Ok(())
    }
}

fn finalize_local_node_output(
    result: ToolResult,
    workspace_root: &Path,
    workspace_id: &str,
) -> Result<LocalNodeToolOutput, LocalNodeToolRejection> {
    let ToolResult {
        content,
        is_error,
        transient_content,
        ..
    } = result;
    let marker = format!("workspace://{workspace_id}");
    let (virtualized, path_redacted) = virtualize_local_paths(content, workspace_root, &marker);
    let (sanitized, retention_redacted) =
        crate::tool_run_output::sanitize_for_retention(&virtualized).map_err(|_| {
            LocalNodeToolRejection::permanent(
                "output_sanitization_failed",
                "The local Node output could not be safely retained",
            )
        })?;
    let (content, total_output_bytes, capped) = cap_utf8(sanitized);
    Ok(LocalNodeToolOutput {
        succeeded: !is_error,
        content,
        total_output_bytes,
        capped,
        redacted: path_redacted || retention_redacted || !transient_content.is_empty(),
    })
}

fn virtualize_local_paths(
    mut content: String,
    workspace_root: &Path,
    workspace_marker: &str,
) -> (String, bool) {
    let mut changed = false;
    let native = workspace_root.to_string_lossy();
    if native.len() > 1 && content.contains(native.as_ref()) {
        content = content.replace(native.as_ref(), workspace_marker);
        changed = true;
    }
    let normalized = native.replace('\\', "/");
    if normalized.len() > 1 && normalized != native && content.contains(&normalized) {
        content = content.replace(&normalized, workspace_marker);
        changed = true;
    }
    let (content, absolute_redacted) = redact_remaining_absolute_paths(&content);
    (content, changed || absolute_redacted)
}

fn redact_remaining_absolute_paths(input: &str) -> (String, bool) {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut copied = 0usize;
    let mut cursor = 0usize;
    let mut changed = false;

    while cursor < bytes.len() {
        let start = if is_path_start(bytes, cursor) {
            Some(cursor)
        } else {
            None
        };
        let Some(start) = start else {
            cursor += input[cursor..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            continue;
        };
        let mut end = start;
        while end < bytes.len() && !is_path_terminator(bytes[end]) {
            end += 1;
        }
        output.push_str(&input[copied..start]);
        output.push_str("<local-path>");
        copied = end;
        cursor = end;
        changed = true;
    }
    if !changed {
        return (input.to_string(), false);
    }
    output.push_str(&input[copied..]);
    (output, true)
}

fn is_path_start(bytes: &[u8], index: usize) -> bool {
    let boundary = index == 0 || is_path_boundary(bytes[index - 1]);
    if !boundary {
        return false;
    }
    if bytes[index] == b'/' {
        if index > 0 && bytes[index - 1] == b':' && bytes.get(index + 1) == Some(&b'/') {
            return !preserved_uri_scheme(bytes, index - 1);
        }
        return true;
    }
    if bytes[index] == b'~' {
        return matches!(bytes.get(index + 1), Some(b'/' | b'\\'));
    }
    if bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'\\') {
        return true;
    }
    bytes[index].is_ascii_alphabetic()
        && bytes.get(index + 1) == Some(&b':')
        && matches!(bytes.get(index + 2), Some(b'/' | b'\\'))
}

fn preserved_uri_scheme(bytes: &[u8], colon: usize) -> bool {
    let mut start = colon;
    while start > 0
        && (bytes[start - 1].is_ascii_alphanumeric()
            || matches!(bytes[start - 1], b'+' | b'-' | b'.'))
    {
        start -= 1;
    }
    let scheme = &bytes[start..colon];
    [b"http".as_slice(), b"https", b"ws", b"wss", b"workspace"]
        .into_iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

fn is_path_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'"' | b'\'' | b'`' | b'=' | b'(' | b'[' | b'{' | b',' | b';' | b':'
        )
}

fn is_path_terminator(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'"' | b'\'' | b'`' | b'<' | b'>' | b')' | b']' | b'}' | b',' | b';'
        )
}

fn cap_utf8(mut content: String) -> (String, u64, bool) {
    let total_output_bytes = content.len() as u64;
    if content.len() <= MAX_LOCAL_NODE_RESULT_BYTES {
        return (content, total_output_bytes, false);
    }
    let mut boundary = MAX_LOCAL_NODE_RESULT_BYTES;
    while !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    content.truncate(boundary);
    (content, total_output_bytes, true)
}

#[cfg(test)]
#[path = "node_tool_runtime_tests.rs"]
mod tests;
