use super::CaptainKernel;
use captain_capspec::{
    compile_named, parse, CapabilityScope, CapabilityStatus, CapabilityView, CompiledCapability,
};
use serde_json::{json, Map, Value};

impl CaptainKernel {
    pub(super) fn management_view_value(
        &self,
        view: &CapabilityView,
        include_source: bool,
    ) -> Result<Value, String> {
        let revisions = self
            .capspec_registry
            .revisions(&view.scope, &view.name)
            .map_err(|error| error.to_string())?;
        let selected_hash = view
            .pending_hash
            .as_ref()
            .or(view.active_hash.as_ref())
            .cloned()
            .or_else(|| {
                revisions
                    .first()
                    .map(|revision| revision.source_hash.clone())
            });
        let compiled = selected_hash
            .as_deref()
            .map(|hash| {
                self.capspec_registry
                    .compiled_revision(&view.scope, &view.name, hash)
                    .map_err(|error| error.to_string())
            })
            .transpose()?
            .flatten();
        let mut value = match compiled {
            Some(compiled) => compiled_summary(&compiled, view.status, true),
            None => json!({
                "name": view.name,
                "tool_name": view.tool_name,
                "status": view.status,
                "ready": false,
            }),
        };
        value["scope"] = json!(scope_kind(&view.scope));
        value["source_file"] = json!(view
            .source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown.captain"));
        value["active_hash"] = json!(view.active_hash);
        value["ready"] = json!(view.active_hash.is_some());
        value["pending_hash"] = json!(view.pending_hash);
        value["selected_hash"] = json!(selected_hash);
        value["last_error"] = json!(view.last_error);
        value["updated_at"] = json!(view.updated_at);
        value["human_action_required"] = json!(matches!(
            view.status,
            CapabilityStatus::PendingApproval | CapabilityStatus::UpdatePendingApproval
        ));
        value["next_action"] = json!(next_action(view.status));
        value["revisions"] = json!(revisions
            .iter()
            .map(|revision| json!({
                "source_hash": revision.source_hash,
                "version": revision.version,
                "permission_fingerprint": revision.permission_fingerprint,
                "created_at": revision.created_at,
                "approved_by": revision.approved_by,
                "approved_at": revision.approved_at,
                "rejected_by": revision.rejected_by,
                "rejected_at": revision.rejected_at,
            }))
            .collect::<Vec<_>>());
        if include_source {
            value["source"] = match selected_hash.as_deref() {
                Some(hash) => json!(self
                    .capspec_registry
                    .revision_source(&view.scope, &view.name, hash)
                    .map_err(|error| error.to_string())?),
                None => Value::Null,
            };
        }
        Ok(value)
    }
}

pub(super) fn validate_capspec_source(
    source: &str,
    expected_name: Option<&str>,
) -> Result<CompiledCapability, String> {
    let raw = parse(source).map_err(|error| error.to_string())?;
    if let Some(expected_name) = expected_name {
        if raw.name != expected_name.trim() {
            return Err(format!(
                "CapSpec source name '{}' does not match requested name '{}'",
                raw.name,
                expected_name.trim()
            ));
        }
    }
    let name = raw.name.clone();
    compile_named(source, raw, Some(&name)).map_err(|error| error.to_string())
}

pub(super) fn compiled_summary(
    compiled: &CompiledCapability,
    status: CapabilityStatus,
    include_runtime_status: bool,
) -> Value {
    let mut summary = Map::from_iter([
        ("valid".to_string(), json!(true)),
        ("name".to_string(), json!(compiled.name)),
        ("tool_name".to_string(), json!(compiled.tool_name)),
        ("description".to_string(), json!(compiled.description)),
        ("version".to_string(), json!(compiled.version)),
        ("tags".to_string(), json!(compiled.tags)),
        ("source_hash".to_string(), json!(compiled.source_hash)),
        (
            "permission_fingerprint".to_string(),
            json!(compiled.permission_fingerprint),
        ),
        ("permissions".to_string(), json!(compiled.permissions)),
        ("input_schema".to_string(), compiled.input_schema.clone()),
        (
            "requires_human_approval".to_string(),
            json!(compiled.requires_human_approval()),
        ),
        (
            "steps".to_string(),
            json!(compiled
                .steps
                .iter()
                .map(|step| json!({
                    "id": step.id,
                    "tool": step.tool,
                    "needs": step.needs,
                    "effect": step.effect,
                    "idempotency": step.idempotency,
                    "timeout_secs": step.timeout_secs,
                    "max_attempts": step.retry.max_attempts,
                }))
                .collect::<Vec<_>>()),
        ),
    ]);
    if include_runtime_status {
        summary.insert("status".to_string(), json!(status));
        summary.insert(
            "ready".to_string(),
            json!(status == CapabilityStatus::Operational),
        );
    }
    Value::Object(summary)
}

pub(super) fn required_text<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{field} is required"))
}

pub(super) fn required_source(source: Option<&str>) -> Result<&str, String> {
    let source = source.ok_or_else(|| "source is required".to_string())?;
    if source.trim().is_empty() {
        Err("source is required".to_string())
    } else {
        Ok(source)
    }
}

fn scope_kind(scope: &CapabilityScope) -> &'static str {
    match scope {
        CapabilityScope::Global => "global",
        CapabilityScope::Project(_) => "project",
    }
}

fn next_action(status: CapabilityStatus) -> &'static str {
    match status {
        CapabilityStatus::Operational => "ready",
        CapabilityStatus::PendingApproval | CapabilityStatus::UpdatePendingApproval => {
            "human approval required for the exact pending hash"
        }
        CapabilityStatus::Invalid | CapabilityStatus::InvalidUpdateRetained => {
            "fix the source validation error"
        }
        CapabilityStatus::Disabled => "reinstall or rollback a known revision",
        CapabilityStatus::Rejected | CapabilityStatus::UpdateRejected => {
            "edit the source or submit a new revision"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_source(name: &str) -> String {
        format!(
            r#"format = 1
name = "{name}"
description = "Read one project file."

[permissions]
tools = ["file_read"]
read_paths = ["/tmp/**"]

[[steps]]
id = "read"
tool = "file_read"
with = {{ path = "/tmp/input.txt" }}
"#
        )
    }

    #[test]
    fn validation_summary_never_echoes_step_payloads() {
        let compiled = validate_capspec_source(&read_source("reader"), None).unwrap();
        let value = compiled_summary(&compiled, CapabilityStatus::Operational, false);
        assert_eq!(value["name"], "reader");
        assert!(value["steps"][0].get("input").is_none());
        assert_eq!(value["requires_human_approval"], false);
    }

    #[test]
    fn explicit_name_must_match_the_source() {
        let error = validate_capspec_source(&read_source("reader"), Some("different")).unwrap_err();
        assert!(error.contains("does not match"), "{error}");
    }
}
