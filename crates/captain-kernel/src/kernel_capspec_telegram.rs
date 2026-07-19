use super::CaptainKernel;
use captain_capspec::{
    CapabilityNodeStatus, CapabilityScope, CapabilityStatus, PermissionSet,
    UncertainNodeExpectation, UncertainResolution,
};
use captain_channels::telegram::{parse_capspec_callback, CapSpecTelegramAction};
use captain_runtime::audit::AuditAction;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapSpecTelegramPromptKind {
    Approval,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapSpecTelegramPrompt {
    pub kind: CapSpecTelegramPromptKind,
    pub token: String,
    pub name: String,
    pub scope: String,
    pub description: String,
    pub version: String,
    pub source_hash: String,
    pub authority: Vec<String>,
    pub run_id: Option<String>,
    pub node_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub attempt: Option<u32>,
    pub origin: Option<String>,
}

#[derive(Clone)]
struct PendingApproval {
    scope: CapabilityScope,
    name: String,
    source_hash: String,
}

#[derive(Clone)]
struct PendingUncertainNode {
    run_id: String,
    node_id: String,
    tool_use_id: String,
    attempt: u32,
}

impl CaptainKernel {
    pub fn capspec_telegram_prompts(&self) -> Result<Vec<CapSpecTelegramPrompt>, String> {
        let mut prompts = Vec::new();
        for candidate in self.pending_capspec_approvals()? {
            let compiled = self
                .capspec_registry
                .compiled_revision(&candidate.scope, &candidate.name, &candidate.source_hash)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    format!(
                        "pending CapSpec '{}' revision '{}' is unavailable",
                        candidate.name, candidate.source_hash
                    )
                })?;
            prompts.push(CapSpecTelegramPrompt {
                kind: CapSpecTelegramPromptKind::Approval,
                token: approval_token(&candidate),
                name: candidate.name,
                scope: candidate.scope.label(),
                description: compiled.description.clone(),
                version: compiled.version.clone(),
                source_hash: candidate.source_hash,
                authority: permission_summary(&compiled.permissions),
                run_id: None,
                node_id: None,
                tool_name: None,
                tool_use_id: None,
                attempt: None,
                origin: None,
            });
        }
        for (run, candidate) in self.pending_capspec_uncertain_nodes()? {
            let node = run
                .nodes
                .iter()
                .find(|node| node.step_id == candidate.node_id)
                .ok_or_else(|| "uncertain CapSpec node disappeared while listing".to_string())?;
            prompts.push(CapSpecTelegramPrompt {
                kind: CapSpecTelegramPromptKind::Uncertain,
                token: uncertain_token(&candidate),
                name: run.capability_name,
                scope: run.scope.label(),
                description: "The runtime cannot prove whether this side effect completed."
                    .to_string(),
                version: String::new(),
                source_hash: run.source_hash,
                authority: Vec::new(),
                run_id: Some(candidate.run_id),
                node_id: Some(candidate.node_id),
                tool_name: Some(node.tool_name.clone()),
                tool_use_id: Some(candidate.tool_use_id),
                attempt: Some(candidate.attempt),
                origin: Some(run.origin),
            });
        }
        prompts.sort_by(|left, right| {
            prompt_rank(left.kind)
                .cmp(&prompt_rank(right.kind))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.token.cmp(&right.token))
        });
        let mut identities = BTreeSet::new();
        for prompt in &prompts {
            if !identities.insert((prompt_rank(prompt.kind), prompt.token.clone())) {
                return Err(
                    "Captain Forge callback token collision; use Control or the TUI".to_string(),
                );
            }
        }
        Ok(prompts)
    }

    pub async fn capspec_resolve_telegram_callback(
        self: &Arc<Self>,
        callback_data: &str,
        actor: &str,
    ) -> Result<String, String> {
        if !actor.starts_with("telegram:") || actor == "telegram:unknown" {
            return Err("Captain Forge requires an authenticated Telegram operator".to_string());
        }
        let callback = parse_capspec_callback(callback_data)
            .ok_or_else(|| "Invalid or expired Captain Forge callback".to_string())?;
        match callback.action {
            CapSpecTelegramAction::Approve | CapSpecTelegramAction::Reject => {
                let candidate = unique_approval(
                    self.pending_capspec_approvals()?
                        .into_iter()
                        .filter(|candidate| approval_token(candidate) == callback.token)
                        .collect(),
                )?;
                let approve = callback.action == CapSpecTelegramAction::Approve;
                let result = if approve {
                    self.capspec_registry.approve(
                        &candidate.scope,
                        &candidate.name,
                        &candidate.source_hash,
                        actor,
                    )
                } else {
                    self.capspec_registry.reject(
                        &candidate.scope,
                        &candidate.name,
                        &candidate.source_hash,
                        actor,
                    )
                }
                .map_err(|error| error.to_string());
                let decision = if approve { "approved" } else { "rejected" };
                let decision_display = if approve { "approuvé" } else { "refusé" };
                self.audit_log.record(
                    actor,
                    AuditAction::CapabilityCheck,
                    format!(
                        "Captain Forge Telegram {decision}: name={} hash={} scope={}",
                        candidate.name,
                        candidate.source_hash,
                        candidate.scope.key()
                    ),
                    if result.is_ok() { "accepted" } else { "failed" },
                );
                result?;
                Ok(format!(
                    "CapSpec `{}` {} pour le hash exact `{}` dans `{}`.",
                    candidate.name,
                    decision_display,
                    candidate.source_hash,
                    candidate.scope.label()
                ))
            }
            CapSpecTelegramAction::Retry
            | CapSpecTelegramAction::ConfirmSucceeded
            | CapSpecTelegramAction::MarkFailed => {
                let candidate = unique_uncertain(
                    self.pending_capspec_uncertain_nodes()?
                        .into_iter()
                        .map(|(_, candidate)| candidate)
                        .filter(|candidate| uncertain_token(candidate) == callback.token)
                        .collect(),
                )?;
                let (resolution, label) = match callback.action {
                    CapSpecTelegramAction::Retry => (UncertainResolution::Retry, "réessai"),
                    CapSpecTelegramAction::ConfirmSucceeded => (
                        UncertainResolution::ConfirmSucceeded {
                            output: serde_json::Value::Null,
                        },
                        "confirmation avec sortie null",
                    ),
                    CapSpecTelegramAction::MarkFailed => (
                        UncertainResolution::MarkFailed {
                            reason: format!("Marked failed by {actor} through Telegram"),
                        },
                        "échec",
                    ),
                    _ => unreachable!(),
                };
                self.capspec_management_resolve_run(
                    &candidate.run_id,
                    &candidate.node_id,
                    UncertainNodeExpectation {
                        tool_use_id: candidate.tool_use_id.clone(),
                        attempt: candidate.attempt,
                    },
                    resolution,
                    actor,
                )
                .await?;
                Ok(format!(
                    "Décision CapSpec `{label}` acceptée pour le run `{}`, le nœud `{}`, la tentative {} et le tool use `{}`.",
                    candidate.run_id, candidate.node_id, candidate.attempt, candidate.tool_use_id
                ))
            }
        }
    }

    fn pending_capspec_approvals(&self) -> Result<Vec<PendingApproval>, String> {
        let views = self
            .capspec_registry
            .list()
            .map_err(|error| error.to_string())?;
        Ok(views
            .into_iter()
            .filter(|view| {
                matches!(
                    view.status,
                    CapabilityStatus::PendingApproval | CapabilityStatus::UpdatePendingApproval
                )
            })
            .filter_map(|view| {
                view.pending_hash.map(|source_hash| PendingApproval {
                    scope: view.scope,
                    name: view.name,
                    source_hash,
                })
            })
            .collect())
    }

    fn pending_capspec_uncertain_nodes(
        &self,
    ) -> Result<Vec<(captain_capspec::CapabilityRunView, PendingUncertainNode)>, String> {
        let runs = self
            .capspec_executor
            .list_waiting_runs(5_000)
            .map_err(|error| error.to_string())?;
        let mut pending = Vec::new();
        for run in runs {
            for node in run
                .nodes
                .iter()
                .filter(|node| node.status == CapabilityNodeStatus::Uncertain)
            {
                let Some(tool_use_id) = node.tool_use_id.clone() else {
                    continue;
                };
                pending.push((
                    run.clone(),
                    PendingUncertainNode {
                        run_id: run.run_id.clone(),
                        node_id: node.step_id.clone(),
                        tool_use_id,
                        attempt: node.attempts,
                    },
                ));
            }
        }
        Ok(pending)
    }
}

fn unique_approval(mut candidates: Vec<PendingApproval>) -> Result<PendingApproval, String> {
    if candidates.len() == 1 {
        return Ok(candidates.remove(0));
    }
    Err(stale_or_ambiguous(candidates.len()))
}

fn unique_uncertain(
    mut candidates: Vec<PendingUncertainNode>,
) -> Result<PendingUncertainNode, String> {
    if candidates.len() == 1 {
        return Ok(candidates.remove(0));
    }
    Err(stale_or_ambiguous(candidates.len()))
}

fn stale_or_ambiguous(matches: usize) -> String {
    if matches == 0 {
        "This Captain Forge decision is stale or already resolved".to_string()
    } else {
        "Captain Forge callback token is ambiguous; use Control or the TUI".to_string()
    }
}

fn approval_token(candidate: &PendingApproval) -> String {
    callback_token(&[
        "approval",
        &candidate.scope.key(),
        &candidate.name,
        &candidate.source_hash,
    ])
}

fn uncertain_token(candidate: &PendingUncertainNode) -> String {
    callback_token(&[
        "uncertain",
        &candidate.run_id,
        &candidate.node_id,
        &candidate.tool_use_id,
        &candidate.attempt.to_string(),
    ])
}

fn callback_token(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().to_hex()[..20].to_string()
}

fn permission_summary(permissions: &PermissionSet) -> Vec<String> {
    let categories = [
        ("tools", &permissions.tools),
        ("read paths", &permissions.read_paths),
        ("write paths", &permissions.write_paths),
        ("network hosts", &permissions.network_hosts),
        ("SSH hosts", &permissions.ssh_hosts),
        ("shell commands", &permissions.shell_commands),
        ("memory read", &permissions.memory_read),
        ("memory write", &permissions.memory_write),
        ("secrets", &permissions.secrets),
    ];
    categories
        .into_iter()
        .filter(|(_, values)| !values.is_empty())
        .map(|(label, values)| format!("{label}: {}", values.join(", ")))
        .collect()
}

fn prompt_rank(kind: CapSpecTelegramPromptKind) -> u8 {
    match kind {
        CapSpecTelegramPromptKind::Approval => 0,
        CapSpecTelegramPromptKind::Uncertain => 1,
    }
}
