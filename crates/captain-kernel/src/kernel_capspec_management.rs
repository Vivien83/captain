use super::kernel_capspec_projection::{
    compiled_summary, required_source, required_text, validate_capspec_source,
};
use super::kernel_capspec_resume::KernelCapabilityResumeInvoker;
use super::{CaptainKernel, PRINCIPAL_AGENT_NAME};
use captain_capspec::{
    CapabilityScope, CapabilityStatus, CapabilityView, UncertainNodeExpectation,
    UncertainResolution,
};
use captain_runtime::audit::AuditAction;
use captain_runtime::kernel_handle::{CapSpecForgeAction, CapSpecForgeRequest, CapSpecForgeScope};
use captain_types::agent::AgentId;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

const CONTROL_ACTOR: &str = "control-web";

impl CaptainKernel {
    pub(super) fn handle_capspec_forge(
        &self,
        request: &CapSpecForgeRequest,
        workspace: Option<&Path>,
        caller_agent_id: Option<&str>,
    ) -> Result<Value, String> {
        let actor = caller_agent_id.unwrap_or("unknown-agent");
        match request.action {
            CapSpecForgeAction::List => self.capspec_management_list(
                request.scope.unwrap_or(CapSpecForgeScope::Effective),
                workspace,
            ),
            CapSpecForgeAction::Inspect => self.capspec_management_inspect(
                required_text(request.name.as_deref(), "name")?,
                request.scope.unwrap_or(CapSpecForgeScope::Effective),
                workspace,
                request.include_source,
            ),
            CapSpecForgeAction::Validate => self.capspec_management_validate(
                required_source(request.source.as_deref())?,
                request.name.as_deref(),
                actor,
            ),
            CapSpecForgeAction::Propose => {
                self.ensure_principal_capspec_proposer(caller_agent_id)?;
                self.capspec_management_install(
                    required_source(request.source.as_deref())?,
                    request.name.as_deref(),
                    request.scope.unwrap_or(if workspace.is_some() {
                        CapSpecForgeScope::Project
                    } else {
                        CapSpecForgeScope::Global
                    }),
                    workspace,
                    actor,
                )
            }
        }
    }

    pub fn capspec_management_list(
        &self,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
    ) -> Result<Value, String> {
        let views = self.management_views(scope, workspace)?;
        let capabilities = views
            .iter()
            .map(|view| self.management_view_value(view, false))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "scope": scope,
            "count": capabilities.len(),
            "capabilities": capabilities,
        }))
    }

    pub fn capspec_management_inspect(
        &self,
        name: &str,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
        include_source: bool,
    ) -> Result<Value, String> {
        if scope == CapSpecForgeScope::All {
            return Err("inspect requires effective, global or project scope".to_string());
        }
        let view = self
            .management_views(scope, workspace)?
            .into_iter()
            .find(|view| view.name == name)
            .ok_or_else(|| format!("CapSpec '{name}' was not found in {scope:?} scope"))?;
        self.management_view_value(&view, include_source)
    }

    pub fn capspec_management_validate(
        &self,
        source: &str,
        expected_name: Option<&str>,
        actor: &str,
    ) -> Result<Value, String> {
        let result = validate_capspec_source(source, expected_name)
            .map(|compiled| compiled_summary(&compiled, CapabilityStatus::Operational, false));
        self.record_capspec_management_audit(
            actor,
            "validate",
            result
                .as_ref()
                .map(|value| value["source_hash"].as_str().unwrap_or("valid"))
                .unwrap_or("invalid"),
            result.as_ref().map(|_| "ok").unwrap_or("denied"),
        );
        result
    }

    pub fn capspec_management_install(
        &self,
        source: &str,
        expected_name: Option<&str>,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
        actor: &str,
    ) -> Result<Value, String> {
        let result = self.capspec_management_install_inner(source, expected_name, scope, workspace);
        let detail = result
            .as_ref()
            .map(|value| {
                format!(
                    "install name={} hash={} scope={scope:?}",
                    value["name"].as_str().unwrap_or("unknown"),
                    value["selected_hash"].as_str().unwrap_or("unknown")
                )
            })
            .unwrap_or_else(|error| format!("install scope={scope:?}: {error}"));
        self.record_capspec_management_audit(
            actor,
            "source-write",
            &detail,
            result.as_ref().map(|_| "ok").unwrap_or("failed"),
        );
        result
    }

    pub fn capspec_management_decide(
        &self,
        name: &str,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
        expected_hash: &str,
        approve: bool,
        actor: &str,
    ) -> Result<Value, String> {
        let resolved_scope = self.mutation_scope(scope, workspace, false)?;
        let result = if approve {
            self.capspec_registry
                .approve(&resolved_scope, name, expected_hash, actor)
        } else {
            self.capspec_registry
                .reject(&resolved_scope, name, expected_hash, actor)
        }
        .map_err(|error| error.to_string())
        .and_then(|view| self.management_view_value(&view, false));
        let action = if approve { "approve" } else { "reject" };
        self.record_capspec_management_audit(
            actor,
            "exact-hash-decision",
            &format!("{action} name={name} hash={expected_hash} scope={scope:?}"),
            result.as_ref().map(|_| "ok").unwrap_or("failed"),
        );
        result
    }

    pub fn capspec_management_rollback(
        &self,
        name: &str,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
        target_hash: &str,
        actor: &str,
    ) -> Result<Value, String> {
        let resolved_scope = self.mutation_scope(scope, workspace, true)?;
        let result = self
            .capspec_registry
            .rollback(&resolved_scope, name, target_hash, actor)
            .map_err(|error| error.to_string())
            .and_then(|view| self.management_view_value(&view, false));
        self.record_capspec_management_audit(
            actor,
            "revision-restore",
            &format!("rollback name={name} hash={target_hash} scope={scope:?}"),
            result.as_ref().map(|_| "ok").unwrap_or("failed"),
        );
        result
    }

    pub fn capspec_management_disable(
        &self,
        name: &str,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
        actor: &str,
    ) -> Result<Value, String> {
        let resolved_scope = self.mutation_scope(scope, workspace, false)?;
        let result = self
            .capspec_registry
            .remove_source(&resolved_scope, name)
            .map_err(|error| error.to_string())
            .and_then(|(removed, view)| {
                self.management_view_value(&view, false).map(|mut value| {
                    value["source_removed"] = json!(removed);
                    value
                })
            });
        self.record_capspec_management_audit(
            actor,
            "source-remove",
            &format!("disable name={name} scope={scope:?}"),
            result.as_ref().map(|_| "ok").unwrap_or("failed"),
        );
        result
    }

    pub fn capspec_management_runs(&self, limit: usize) -> Result<Value, String> {
        let runs = self
            .capspec_executor
            .list_runs(limit.clamp(1, 500))
            .map_err(|error| error.to_string())?;
        Ok(json!({"count": runs.len(), "runs": runs}))
    }

    pub fn capspec_management_run(&self, run_id: &str) -> Result<Value, String> {
        self.capspec_executor
            .run(run_id)
            .map(|run| json!(run))
            .map_err(|error| error.to_string())
    }

    pub async fn capspec_management_resolve_run(
        self: &Arc<Self>,
        run_id: &str,
        node_id: &str,
        expectation: UncertainNodeExpectation,
        resolution: UncertainResolution,
        actor: &str,
    ) -> Result<Value, String> {
        let resume_required = !matches!(resolution, UncertainResolution::MarkFailed { .. });
        let invoker = if resume_required {
            let context = self
                .capspec_executor
                .resume_context(run_id)
                .map_err(|error| error.to_string())?;
            Some(KernelCapabilityResumeInvoker::prepare(
                Arc::clone(self),
                context,
            )?)
        } else {
            None
        };
        let decision = match &resolution {
            UncertainResolution::ConfirmSucceeded { .. } => "confirm_succeeded",
            UncertainResolution::Retry => "retry",
            UncertainResolution::MarkFailed { .. } => "mark_failed",
        };
        let receipt = self
            .capspec_executor
            .apply_uncertain_resolution(run_id, node_id, &expectation, resolution)
            .map_err(|error| error.to_string())?;
        self.record_capspec_management_audit(
            actor,
            "uncertain-node-decision",
            &format!(
                "decision={decision} run={run_id} node={node_id} attempt={} tool_use_id={}",
                expectation.attempt, expectation.tool_use_id
            ),
            "accepted",
        );

        if let Some(invoker) = invoker {
            super::kernel_capspec_resume_recovery::schedule_capspec_operator_resume(
                Arc::clone(self),
                run_id.to_string(),
                Some(invoker),
            );
        }
        Ok(json!({
            "accepted": true,
            "decision": decision,
            "resume_scheduled": receipt.resume_required,
            "run": receipt.run,
        }))
    }

    pub fn capspec_control_actor() -> &'static str {
        CONTROL_ACTOR
    }

    fn capspec_management_install_inner(
        &self,
        source: &str,
        expected_name: Option<&str>,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
    ) -> Result<Value, String> {
        let compiled = validate_capspec_source(source, expected_name)?;
        let resolved_scope = self.mutation_scope(scope, workspace, true)?;
        let source_path = self
            .capspec_registry
            .source_path_for_mutation(&resolved_scope, &compiled.name)
            .map_err(|error| error.to_string())?;
        captain_types::durable_fs::atomic_write(&source_path, source.as_bytes())
            .map_err(|error| format!("cannot persist CapSpec source: {error}"))?;
        match resolved_scope {
            CapabilityScope::Global => self.capspec_registry.reload_global(),
            CapabilityScope::Project(_) => self.capspec_registry.reload_all(),
        }
        .map_err(|error| format!("CapSpec source was written but reload failed: {error}"))?;
        let view = self
            .capspec_registry
            .capability(&resolved_scope, &compiled.name)
            .map_err(|error| error.to_string())?;
        self.management_view_value(&view, false)
    }

    fn mutation_scope(
        &self,
        scope: CapSpecForgeScope,
        workspace: Option<&Path>,
        create_project: bool,
    ) -> Result<CapabilityScope, String> {
        match scope {
            CapSpecForgeScope::Global => Ok(CapabilityScope::Global),
            CapSpecForgeScope::Project => self.register_capspec_project_scope(
                workspace.ok_or_else(|| "project scope requires a workspace".to_string())?,
                create_project,
            ),
            CapSpecForgeScope::Effective | CapSpecForgeScope::All => {
                Err("mutating a CapSpec requires an explicit global or project scope".to_string())
            }
        }
    }

    fn management_views(
        &self,
        selector: CapSpecForgeScope,
        workspace: Option<&Path>,
    ) -> Result<Vec<CapabilityView>, String> {
        let project_scope = self.project_scope_for_read(workspace)?;
        if selector == CapSpecForgeScope::Project && project_scope.is_none() {
            return Err(
                "project scope requires a workspace with .captain/capabilities".to_string(),
            );
        }
        let mut views = self
            .capspec_registry
            .list()
            .map_err(|error| error.to_string())?;
        views.retain(|view| match selector {
            CapSpecForgeScope::Global => view.scope == CapabilityScope::Global,
            CapSpecForgeScope::Project => project_scope.as_ref() == Some(&view.scope),
            CapSpecForgeScope::All | CapSpecForgeScope::Effective => {
                view.scope == CapabilityScope::Global || project_scope.as_ref() == Some(&view.scope)
            }
        });
        if selector == CapSpecForgeScope::Effective {
            let mut effective = BTreeMap::<String, CapabilityView>::new();
            for view in views
                .iter()
                .filter(|view| view.scope == CapabilityScope::Global)
            {
                effective.insert(view.name.clone(), view.clone());
            }
            if let Some(project_scope) = &project_scope {
                for view in views.iter().filter(|view| {
                    &view.scope == project_scope && view.status != CapabilityStatus::Disabled
                }) {
                    effective.insert(view.name.clone(), view.clone());
                }
            }
            views = effective.into_values().collect();
        }
        views.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.scope.key().cmp(&right.scope.key()))
        });
        Ok(views)
    }

    fn project_scope_for_read(
        &self,
        workspace: Option<&Path>,
    ) -> Result<Option<CapabilityScope>, String> {
        let Some(workspace) = workspace else {
            return Ok(None);
        };
        let canonical = workspace
            .canonicalize()
            .map_err(|error| format!("invalid CapSpec project workspace: {error}"))?;
        if !canonical.join(".captain/capabilities").is_dir() {
            return Ok(None);
        }
        self.register_capspec_project_scope(&canonical, false)
            .map(Some)
    }

    fn ensure_principal_capspec_proposer(
        &self,
        caller_agent_id: Option<&str>,
    ) -> Result<(), String> {
        let caller = caller_agent_id
            .ok_or_else(|| "CapSpec proposals require the principal Captain agent".to_string())?;
        let agent_id = caller
            .parse::<AgentId>()
            .map_err(|_| "CapSpec proposals require a valid principal agent ID".to_string())?;
        let is_principal = self
            .registry
            .get(agent_id)
            .is_some_and(|entry| entry.manifest.name == PRINCIPAL_AGENT_NAME);
        if is_principal {
            Ok(())
        } else {
            Err("CapSpec proposals are reserved for the principal Captain agent".to_string())
        }
    }

    pub(super) fn record_capspec_management_audit(
        &self,
        actor: &str,
        operation: &str,
        detail: &str,
        outcome: &str,
    ) {
        self.audit_log.record(
            actor,
            AuditAction::CapabilityCheck,
            format!("Captain Forge {operation}: {detail}"),
            outcome,
        );
    }
}

#[cfg(test)]
#[path = "kernel_capspec_management_tests.rs"]
mod tests;
