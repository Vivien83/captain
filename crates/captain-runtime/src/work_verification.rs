//! Deterministic, redacted evidence policy for an agent turn.
//!
//! The model chooses the smallest useful check. This module decides whether
//! the resulting receipts are recent and strong enough to support delivery.
//! Raw tool inputs, outputs, paths and hidden reasoning never enter receipts.

use captain_capspec::{reviewed_effect, Effect};
use captain_types::tool::{ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const MAX_VERIFICATION_CORRECTION_ROUNDS: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkEffect {
    Observation,
    Verification,
    LocalMutation,
    DurableMutation,
    ExternalEffect,
    HumanInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    None,
    Receipt,
    Inspection,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolVerificationReceipt {
    pub tool_call_id: String,
    pub sequence: u32,
    pub input_sha256: String,
    pub effect: WorkEffect,
    pub evidence: EvidenceStrength,
    pub scope_digests: Vec<String>,
}

impl ToolVerificationReceipt {
    pub fn from_tool_call(tool_call: &ToolCall, result: &ToolResult, sequence: u32) -> Self {
        let effect = classify_tool_call(tool_call);
        let evidence = if result.is_error {
            EvidenceStrength::None
        } else {
            crate::work_verification_structured::known_evidence_strength(
                &tool_call.name,
                &tool_call.input,
                &result.content,
            )
            .unwrap_or_else(|| evidence_strength(effect, false))
        };
        let mut scope_digests = tool_scope_digests(tool_call);
        scope_digests.extend(
            crate::work_verification_structured::output_identity_values(
                &tool_call.name,
                &result.content,
            )
            .into_iter()
            .map(|identity| external_scope(&identity)),
        );
        scope_digests.sort();
        scope_digests.dedup();
        Self {
            tool_call_id: tool_call.id.clone(),
            sequence,
            input_sha256: crate::tool_runs::input_digest(&tool_call.input),
            effect,
            evidence,
            scope_digests,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationDisposition {
    NotRequired,
    Verified,
    NeedsCorrection,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationGap {
    pub code: &'static str,
    pub tool_name: String,
    pub sequence: u32,
    pub scope_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkVerificationReport {
    pub disposition: VerificationDisposition,
    pub observed_receipts: Vec<String>,
    pub gaps: Vec<VerificationGap>,
}

impl WorkVerificationReport {
    pub fn requires_correction(&self) -> bool {
        self.disposition == VerificationDisposition::NeedsCorrection
    }

    pub fn correction_nudge(&self) -> String {
        let requirements = self
            .gaps
            .iter()
            .take(4)
            .map(|gap| match gap.code {
                "receipt_missing" => {
                    "do not claim completion because an executed tool has no verifiable receipt"
                        .to_string()
                }
                "postcondition_missing" => format!(
                    "run one targeted post-condition check after {} (receipt #{})",
                    gap.tool_name,
                    gap.sequence + 1
                ),
                "effect_failed" => format!(
                    "resolve or explicitly leave incomplete the failed {} effect (receipt #{})",
                    gap.tool_name,
                    gap.sequence + 1
                ),
                "receipt_unconfirmed" => format!(
                    "inspect the current state of {} without replaying it (receipt #{})",
                    gap.tool_name,
                    gap.sequence + 1
                ),
                "subject_pending" => format!(
                    "revisit the pending {} subject by its returned id (receipt #{})",
                    gap.tool_name,
                    gap.sequence + 1
                ),
                "subject_unsuccessful" => format!(
                    "report or resolve the non-successful {} result without blind replay (receipt #{})",
                    gap.tool_name,
                    gap.sequence + 1
                ),
                _ => format!("resolve {} at receipt #{}", gap.code, gap.sequence + 1),
            })
            .collect::<Vec<_>>()
            .join("; ");

        format!(
            "[System: Delivery verification is incomplete. {requirements}. Use the smallest relevant check, then correct only what the evidence disproves. Never replay an uncertain external effect. Do not merely claim that verification passed.]"
        )
    }

    pub fn incomplete_notice(&self) -> String {
        let tools = self
            .gaps
            .iter()
            .map(|gap| gap.tool_name.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(4)
            .collect::<Vec<_>>()
            .join(", ");
        if tools.is_empty() {
            "Verification incomplete: the available evidence did not prove the requested outcome."
                .to_string()
        } else {
            format!(
                "Verification incomplete: the available evidence did not close the post-condition for {tools}."
            )
        }
    }
}

pub fn evaluate_tool_receipts(
    records: &[crate::agent_loop_tool_record::ToolCallRecord],
    any_tools_executed: bool,
    correction_round: u8,
) -> WorkVerificationReport {
    let mut observed_receipts = Vec::new();
    let mut gaps = Vec::new();
    let mut saw_effectful_work = false;

    if any_tools_executed && records.is_empty() {
        saw_effectful_work = true;
        gaps.push(VerificationGap {
            code: "receipt_missing",
            tool_name: "tool execution".to_string(),
            sequence: 0,
            scope_hint: None,
        });
    }

    for (index, record) in records.iter().enumerate() {
        let receipt = receipt_or_fallback(record, index as u32);
        if !record.is_error && receipt.evidence != EvidenceStrength::None {
            observed_receipts.push(receipt.tool_call_id.clone());
        }

        match receipt.effect {
            WorkEffect::LocalMutation => {
                saw_effectful_work = true;
                if record.is_error {
                    if !has_later_recovery(records, index, record, &receipt) {
                        gaps.push(gap("effect_failed", record, &receipt));
                    }
                } else if !has_postcondition(records, index, &receipt) {
                    gaps.push(gap("postcondition_missing", record, &receipt));
                }
            }
            WorkEffect::DurableMutation | WorkEffect::ExternalEffect => {
                saw_effectful_work = true;
                if record.is_error && !has_later_recovery(records, index, record, &receipt) {
                    gaps.push(gap("effect_failed", record, &receipt));
                } else if record.tool_name == "agent_delegate"
                    && receipt.evidence == EvidenceStrength::Inspection
                {
                    gaps.push(gap("subject_pending", record, &receipt));
                } else if !record.is_error && receipt.evidence == EvidenceStrength::None {
                    gaps.push(gap("receipt_unconfirmed", record, &receipt));
                }
            }
            WorkEffect::Verification => {
                if requires_confirmed_subject_result(&record.tool_name)
                    && receipt.evidence != EvidenceStrength::Check
                {
                    saw_effectful_work = true;
                    gaps.push(gap(
                        if receipt.evidence == EvidenceStrength::Inspection {
                            "subject_pending"
                        } else {
                            "subject_unsuccessful"
                        },
                        record,
                        &receipt,
                    ));
                } else if record.is_error && has_prior_unverified_mutation(records, index) {
                    gaps.push(gap("effect_failed", record, &receipt));
                }
            }
            WorkEffect::Observation | WorkEffect::HumanInput => {}
        }
    }

    gaps.sort_by_key(|gap| (gap.sequence, gap.code));
    gaps.dedup_by(|left, right| {
        left.sequence == right.sequence
            && left.code == right.code
            && left.tool_name == right.tool_name
    });

    let disposition = if gaps.is_empty() {
        if saw_effectful_work {
            VerificationDisposition::Verified
        } else {
            VerificationDisposition::NotRequired
        }
    } else if correction_round < MAX_VERIFICATION_CORRECTION_ROUNDS {
        VerificationDisposition::NeedsCorrection
    } else {
        VerificationDisposition::Incomplete
    };

    WorkVerificationReport {
        disposition,
        observed_receipts,
        gaps,
    }
}

fn receipt_or_fallback(
    record: &crate::agent_loop_tool_record::ToolCallRecord,
    sequence: u32,
) -> ToolVerificationReceipt {
    record
        .verification
        .clone()
        .unwrap_or_else(|| ToolVerificationReceipt {
            tool_call_id: format!("legacy-{sequence}"),
            sequence,
            input_sha256: digest_bytes(record.input_summary.as_bytes()),
            effect: classify_tool_name(&record.tool_name, None),
            evidence: evidence_strength(
                classify_tool_name(&record.tool_name, None),
                record.is_error,
            ),
            scope_digests: fallback_scope(&record.tool_name),
        })
}

fn has_later_recovery(
    records: &[crate::agent_loop_tool_record::ToolCallRecord],
    failed_index: usize,
    failed_record: &crate::agent_loop_tool_record::ToolCallRecord,
    failed_receipt: &ToolVerificationReceipt,
) -> bool {
    records
        .iter()
        .enumerate()
        .skip(failed_index + 1)
        .any(|(index, candidate)| {
            if candidate.is_error || candidate.tool_name != failed_record.tool_name {
                return false;
            }
            let candidate_receipt = receipt_or_fallback(candidate, index as u32);
            scopes_overlap(
                &failed_receipt.scope_digests,
                &candidate_receipt.scope_digests,
            )
        })
}

fn has_postcondition(
    records: &[crate::agent_loop_tool_record::ToolCallRecord],
    mutation_index: usize,
    mutation: &ToolVerificationReceipt,
) -> bool {
    let mut inspected_targets = BTreeSet::new();
    for (index, candidate) in records.iter().enumerate().skip(mutation_index + 1) {
        if candidate.is_error {
            continue;
        }
        let receipt = receipt_or_fallback(candidate, index as u32);
        match receipt.evidence {
            EvidenceStrength::Check
                if check_covers(&mutation.scope_digests, &receipt.scope_digests) =>
            {
                return true;
            }
            EvidenceStrength::Inspection => {
                inspected_targets.extend(receipt.scope_digests.iter().cloned());
                if target_inspection_covers(&mutation.scope_digests, &inspected_targets) {
                    return true;
                }
            }
            EvidenceStrength::None | EvidenceStrength::Receipt | EvidenceStrength::Check => {}
        }
    }
    false
}

fn has_prior_unverified_mutation(
    records: &[crate::agent_loop_tool_record::ToolCallRecord],
    before_index: usize,
) -> bool {
    records
        .iter()
        .enumerate()
        .take(before_index)
        .any(|(index, record)| {
            let receipt = receipt_or_fallback(record, index as u32);
            !record.is_error
                && receipt.effect == WorkEffect::LocalMutation
                && !has_postcondition(records, index, &receipt)
        })
}

fn gap(
    code: &'static str,
    record: &crate::agent_loop_tool_record::ToolCallRecord,
    receipt: &ToolVerificationReceipt,
) -> VerificationGap {
    VerificationGap {
        code,
        tool_name: record.tool_name.clone(),
        sequence: receipt.sequence,
        scope_hint: receipt
            .scope_digests
            .first()
            .map(|scope| scope.chars().take(20).collect()),
    }
}

fn classify_tool_call(tool_call: &ToolCall) -> WorkEffect {
    classify_tool_name(&tool_call.name, Some(&tool_call.input))
}

/// Classify a tool offered to a remote-operated Node. Unlike ordinary turn
/// verification, an unknown shell command is an external effect: a false
/// positive only asks for approval, while a false local classification could
/// replay an unobserved side effect after a partition.
pub fn classify_distributed_tool_effect(tool_name: &str, input: &serde_json::Value) -> WorkEffect {
    if !is_command_tool(tool_name) {
        return classify_tool_name(tool_name, Some(input));
    }
    input
        .as_object()
        .and_then(|_| command_from_input(input))
        .map(classify_distributed_command)
        .unwrap_or(WorkEffect::ExternalEffect)
}

fn classify_tool_name(tool_name: &str, input: Option<&serde_json::Value>) -> WorkEffect {
    if tool_name == "ask_user" {
        return WorkEffect::HumanInput;
    }
    if is_explicit_verification_tool(tool_name) {
        return WorkEffect::Verification;
    }
    if is_local_mutation_tool(tool_name) {
        return WorkEffect::LocalMutation;
    }
    if is_durable_receipt_tool(tool_name) {
        return WorkEffect::DurableMutation;
    }
    if is_external_effect_tool(tool_name) {
        return WorkEffect::ExternalEffect;
    }

    match reviewed_effect(tool_name) {
        Effect::Read => WorkEffect::Observation,
        Effect::Write => WorkEffect::LocalMutation,
        Effect::Destructive if is_command_tool(tool_name) => {
            let effect = input
                .and_then(command_from_input)
                .map(classify_command)
                .unwrap_or(WorkEffect::LocalMutation);
            if tool_name == "ssh_exec" && effect == WorkEffect::LocalMutation {
                WorkEffect::ExternalEffect
            } else {
                effect
            }
        }
        Effect::Destructive => WorkEffect::DurableMutation,
        Effect::External if looks_like_observation(tool_name) => WorkEffect::Observation,
        Effect::External => WorkEffect::ExternalEffect,
    }
}

fn evidence_strength(effect: WorkEffect, is_error: bool) -> EvidenceStrength {
    if is_error {
        return EvidenceStrength::None;
    }
    match effect {
        WorkEffect::Observation => EvidenceStrength::Inspection,
        WorkEffect::Verification => EvidenceStrength::Check,
        WorkEffect::DurableMutation | WorkEffect::ExternalEffect | WorkEffect::HumanInput => {
            EvidenceStrength::Receipt
        }
        WorkEffect::LocalMutation => EvidenceStrength::None,
    }
}

fn is_explicit_verification_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "web_citation_audit"
            | "skill_check"
            | "ssh_health_check"
            | "channel_test"
            | "email_test"
            | "model_test"
            | "artifact_inspect"
            | "tool_run_result"
            | "tool_run_status"
            | "agent_job_result"
    ) || tool_name.ends_with("_verify")
        || tool_name.ends_with("_audit")
        || tool_name.ends_with("_health")
        || tool_name.ends_with("_check")
        || tool_name.ends_with("_test")
}

fn requires_confirmed_subject_result(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "tool_run_result" | "agent_job_result" | "artifact_inspect"
    )
}

fn is_local_mutation_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_write"
            | "apply_patch"
            | "edit_file"
            | "multi_edit"
            | "file_delete"
            | "document_create"
            | "document_pipeline"
            | "system_update"
    )
}

fn is_durable_receipt_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "memory_save"
            | "memory_store"
            | "memory_forget"
            | "checkpoint_save"
            | "secret_write"
            | "goal_create"
            | "goal_pause"
            | "goal_resume"
            | "goal_delete"
            | "project_create"
            | "project_update"
            | "project_delete"
            | "agent_spawn"
            | "agent_kill"
            | "agent_delegate"
            | "artifact_publish"
            | "tool_run_start"
            | "tool_run_cancel"
            | "tool_run_retry"
    )
}

fn is_external_effect_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "channel_send"
            | "artifact_deliver"
            | "email_send"
            | "webhook_send"
            | "agent_send"
            | "ssh_upload"
            | "ssh_download"
    ) || tool_name.ends_with("_send")
        || tool_name.ends_with("_deliver")
        || tool_name.ends_with("_publish")
}

fn looks_like_observation(tool_name: &str) -> bool {
    [
        "_read", "_list", "_get", "_find", "_search", "_recall", "_status", "_inspect", "_result",
        "_tail", "_docs",
    ]
    .iter()
    .any(|suffix| tool_name.ends_with(suffix))
        || matches!(
            tool_name,
            "tool_search" | "capability_search" | "captain_docs" | "system_time"
        )
}

fn is_command_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "shell_exec"
            | "shell_exec_critical"
            | "execute_code"
            | "ssh_exec"
            | "docker_exec"
            | "docker_run"
            | "cargo"
            | "npm"
            | "pip"
    )
}

fn command_from_input(input: &serde_json::Value) -> Option<&str> {
    ["command", "cmd", "script"]
        .iter()
        .find_map(|key| input.get(*key).and_then(serde_json::Value::as_str))
        .or_else(|| input.get("subcommand").and_then(serde_json::Value::as_str))
}

fn classify_command(command: &str) -> WorkEffect {
    classify_normalized_command(command, WorkEffect::LocalMutation)
}

fn classify_distributed_command(command: &str) -> WorkEffect {
    let normalized = normalize_command(command);
    if command_has_external_effect(&normalized) {
        WorkEffect::ExternalEffect
    } else if command_has_mutation(&normalized) || command_has_verification(&normalized) {
        // Build, test, health, and status commands may execute project hooks,
        // populate caches, or contact a daemon. They are useful evidence but
        // are not replay-safe observations on a remote Node.
        WorkEffect::LocalMutation
    } else if command_is_observation(&normalized) {
        WorkEffect::Observation
    } else {
        WorkEffect::ExternalEffect
    }
}

fn classify_normalized_command(command: &str, unknown: WorkEffect) -> WorkEffect {
    let normalized = normalize_command(command);
    if command_has_external_effect(&normalized) {
        WorkEffect::ExternalEffect
    } else if command_has_mutation(&normalized) {
        WorkEffect::LocalMutation
    } else if command_has_verification(&normalized) {
        WorkEffect::Verification
    } else if command_is_observation(&normalized) {
        WorkEffect::Observation
    } else {
        unknown
    }
}

fn normalize_command(command: &str) -> String {
    format!(
        " {} ",
        command
            .to_ascii_lowercase()
            .replace('\r', " ")
            .replace('\n', " ; ")
    )
}

fn command_has_external_effect(command: &str) -> bool {
    [
        " git push",
        " docker push",
        " kubectl apply",
        " kubectl create",
        " kubectl delete",
        " gh release create",
        " gh pr create",
        " gh pr merge",
        " gh pr close",
        " gh issue create",
        " curl -x post",
        " curl -x put",
        " curl -x patch",
        " curl -x delete",
        " curl --request post",
        " curl --request put",
        " curl --request patch",
        " curl --request delete",
        " curl -d ",
        " curl --data",
        " curl -t ",
        " curl --upload-file",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn command_has_mutation(command: &str) -> bool {
    [
        " rm ",
        " mv ",
        " cp ",
        " mkdir ",
        " touch ",
        " chmod ",
        " chown ",
        " tee ",
        " sed -i",
        " git add",
        " git commit",
        " git checkout",
        " git switch",
        " git reset",
        " git restore",
        " git clean",
        " cargo install",
        " npm install",
        " npm update",
        " pip install",
        " apt install",
        " apt-get install",
        " brew install",
        " docker restart",
        " docker start",
        " docker stop",
        " docker rm",
        " docker compose up",
        " docker compose down",
        " systemctl restart",
        " systemctl start",
        " systemctl stop",
        " systemctl enable",
        " systemctl disable",
        " captain update",
    ]
    .iter()
    .any(|needle| command.contains(needle))
        || (command.contains(" cargo fmt") && !command.contains("--check"))
        || command.contains(" > ")
        || command.contains(" >> ")
}

fn command_has_verification(command: &str) -> bool {
    [
        " cargo test",
        " cargo check",
        " cargo clippy",
        " cargo build",
        " cargo fmt --check",
        " npm test",
        " npm run test",
        " npm run lint",
        " pytest",
        " go test",
        " git status",
        " git diff",
        " git show",
        " git log",
        " git rev-parse",
        " captain doctor",
        " captain status",
        " systemctl status",
        " systemctl is-active",
        " journalctl",
        " docker ps",
        " docker logs",
        " docker inspect",
        " health_check",
        " integrity_check",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn command_is_observation(command: &str) -> bool {
    let mut saw_command = false;
    let only_observations = command
        .split([';', '&', '|'])
        .filter_map(|segment| segment.split_whitespace().next())
        .filter(|word| !matches!(*word, "set" | "then" | "do" | "done"))
        .all(|word| {
            saw_command = true;
            matches!(
                word,
                "cat"
                    | "date"
                    | "df"
                    | "du"
                    | "echo"
                    | "env"
                    | "false"
                    | "find"
                    | "free"
                    | "grep"
                    | "head"
                    | "id"
                    | "jq"
                    | "ls"
                    | "memory_pressure"
                    | "pgrep"
                    | "printenv"
                    | "printf"
                    | "ps"
                    | "pwd"
                    | "rg"
                    | "ss"
                    | "stat"
                    | "sysctl"
                    | "tail"
                    | "test"
                    | "true"
                    | "uname"
                    | "uptime"
                    | "vm_stat"
                    | "wc"
                    | "which"
                    | "whoami"
            )
        });
    saw_command && only_observations
}

fn tool_scope_digests(tool_call: &ToolCall) -> Vec<String> {
    let mut scopes = BTreeSet::new();
    collect_scope_values(&tool_call.input, None, &mut scopes);
    if tool_call.name == "apply_patch" {
        if let Some(patch) = tool_call
            .input
            .get("patch")
            .and_then(serde_json::Value::as_str)
        {
            for line in patch.lines() {
                for marker in ["*** Add File: ", "*** Update File: ", "*** Delete File: "] {
                    if let Some(path) = line.strip_prefix(marker) {
                        scopes.insert(target_scope(path));
                    }
                }
            }
        }
    }
    if scopes.is_empty() {
        fallback_scope(&tool_call.name)
    } else {
        scopes.into_iter().collect()
    }
}

fn collect_scope_values(
    value: &serde_json::Value,
    parent_key: Option<&str>,
    scopes: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                collect_scope_values(child, Some(key), scopes);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_scope_values(child, parent_key, scopes);
            }
        }
        serde_json::Value::String(text) => match parent_key.unwrap_or_default() {
            "path" | "file" | "filename" | "target" => {
                scopes.insert(target_scope(text));
            }
            "cwd" | "workdir" | "workspace" | "workspace_root" => {
                scopes.insert(workspace_scope(text));
            }
            "url" | "host" | "project_id" | "agent_id" | "run_id" | "job_id" | "artifact_id" => {
                scopes.insert(external_scope(text));
            }
            _ => {}
        },
        _ => {}
    }
}

fn fallback_scope(tool_name: &str) -> Vec<String> {
    match classify_tool_name(tool_name, None) {
        WorkEffect::LocalMutation | WorkEffect::Verification | WorkEffect::Observation => {
            vec!["workspace".to_string()]
        }
        _ => vec!["external".to_string()],
    }
}

fn target_scope(value: &str) -> String {
    format!("target:{}", short_digest(value.as_bytes()))
}

fn workspace_scope(value: &str) -> String {
    format!("workspace:{}", short_digest(value.as_bytes()))
}

fn external_scope(value: &str) -> String {
    format!("external:{}", short_digest(value.as_bytes()))
}

fn short_digest(value: &[u8]) -> String {
    digest_bytes(value).chars().take(16).collect()
}

fn digest_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(crate) fn verification_identifier_digest(value: &str) -> String {
    digest_bytes(value.as_bytes())
}

fn scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|scope| right.contains(scope))
        || (left.iter().any(|scope| scope == "external")
            && right.iter().any(|scope| scope == "external"))
}

fn check_covers(mutation: &[String], evidence: &[String]) -> bool {
    if evidence.iter().any(|scope| scope.starts_with("workspace")) {
        return true;
    }
    let evidence = evidence.iter().cloned().collect::<BTreeSet<_>>();
    target_inspection_covers(mutation, &evidence)
}

fn target_inspection_covers(mutation: &[String], evidence: &BTreeSet<String>) -> bool {
    let targets = mutation
        .iter()
        .filter(|scope| scope.starts_with("target:"))
        .collect::<Vec<_>>();
    !targets.is_empty() && targets.iter().all(|scope| evidence.contains(*scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop_tool_record::ToolCallRecord;

    fn call(name: &str, input: serde_json::Value, error: bool, sequence: u32) -> ToolCallRecord {
        call_with_content(
            name,
            input,
            error,
            sequence,
            if error { "failed" } else { "ok" },
        )
    }

    fn call_with_content(
        name: &str,
        input: serde_json::Value,
        error: bool,
        sequence: u32,
        content: &str,
    ) -> ToolCallRecord {
        let tool_call = ToolCall {
            id: format!("call-{sequence}"),
            name: name.to_string(),
            input,
        };
        let result = ToolResult {
            tool_use_id: tool_call.id.clone(),
            content: content.to_string(),
            is_error: error,
            transient_content: Vec::new(),
        };
        ToolCallRecord {
            tool_name: name.to_string(),
            reason: "test".to_string(),
            is_error: error,
            duration_ms: 1,
            input_summary: String::new(),
            output_summary: result.content.clone(),
            verification: Some(ToolVerificationReceipt::from_tool_call(
                &tool_call, &result, sequence,
            )),
        }
    }

    #[test]
    fn read_only_turn_needs_no_extra_pass() {
        let records = vec![call(
            "file_read",
            serde_json::json!({"path": "README.md"}),
            false,
            0,
        )];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::NotRequired
        );
    }

    #[test]
    fn local_mutation_requires_a_newer_postcondition() {
        let records = vec![call(
            "file_write",
            serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"}),
            false,
            0,
        )];
        let report = evaluate_tool_receipts(&records, true, 0);
        assert_eq!(report.disposition, VerificationDisposition::NeedsCorrection);
        assert_eq!(report.gaps[0].code, "postcondition_missing");
    }

    #[test]
    fn matching_inspection_after_write_is_evidence() {
        let records = vec![
            call(
                "file_write",
                serde_json::json!({"path": "notes.txt", "content": "done"}),
                false,
                0,
            ),
            call(
                "file_read",
                serde_json::json!({"path": "notes.txt"}),
                false,
                1,
            ),
        ];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::Verified
        );
    }

    #[test]
    fn unrelated_inspection_does_not_cover_write() {
        let records = vec![
            call(
                "file_write",
                serde_json::json!({"path": "notes.txt", "content": "done"}),
                false,
                0,
            ),
            call(
                "file_read",
                serde_json::json!({"path": "other.txt"}),
                false,
                1,
            ),
        ];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::NeedsCorrection
        );
    }

    #[test]
    fn check_before_new_mutation_is_stale() {
        let records = vec![
            call(
                "shell_exec",
                serde_json::json!({"command": "cargo test -p app"}),
                false,
                0,
            ),
            call(
                "apply_patch",
                serde_json::json!({"patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch"}),
                false,
                1,
            ),
        ];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::NeedsCorrection
        );
    }

    #[test]
    fn workspace_check_after_patch_closes_the_contract() {
        let records = vec![
            call(
                "apply_patch",
                serde_json::json!({"patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch"}),
                false,
                0,
            ),
            call(
                "shell_exec",
                serde_json::json!({"command": "cargo test -p app"}),
                false,
                1,
            ),
        ];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::Verified
        );
    }

    #[test]
    fn successful_external_effect_uses_its_receipt_without_replay() {
        let records = vec![call(
            "channel_send",
            serde_json::json!({"channel": "telegram", "message": "ready"}),
            false,
            0,
        )];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::Verified
        );
    }

    #[test]
    fn detached_subject_status_is_not_a_check_until_successfully_terminal() {
        let running = call_with_content(
            "tool_run_status",
            serde_json::json!({"run_id": "run-7"}),
            false,
            0,
            r#"{"run_id":"run-7","status":"running"}"#,
        );
        let completed = call_with_content(
            "tool_run_result",
            serde_json::json!({"run_id": "run-7"}),
            false,
            1,
            r#"{"run_id":"run-7","status":"completed","is_error":false,"result":"ok"}"#,
        );

        assert_eq!(
            running.verification.as_ref().unwrap().evidence,
            EvidenceStrength::Inspection
        );
        assert_eq!(
            completed.verification.as_ref().unwrap().evidence,
            EvidenceStrength::Check
        );
        assert!(scopes_overlap(
            &running.verification.as_ref().unwrap().scope_digests,
            &completed.verification.as_ref().unwrap().scope_digests,
        ));
    }

    #[test]
    fn delegation_launch_can_finish_but_wait_mode_cannot_hide_pending_work() {
        let output = r#"{"job_id":"job-7","status":"running"}"#;
        let launched = vec![call_with_content(
            "agent_delegate",
            serde_json::json!({"agent_id": "worker", "wait_for_result": false}),
            false,
            0,
            output,
        )];
        let waiting = vec![call_with_content(
            "agent_delegate",
            serde_json::json!({"agent_id": "worker", "wait_for_result": true}),
            false,
            0,
            output,
        )];

        assert_eq!(
            evaluate_tool_receipts(&launched, true, 0).disposition,
            VerificationDisposition::Verified
        );
        let report = evaluate_tool_receipts(&waiting, true, 0);
        assert_eq!(report.disposition, VerificationDisposition::NeedsCorrection);
        assert_eq!(report.gaps[0].code, "subject_pending");
    }

    #[test]
    fn unsuccessful_detached_result_cannot_support_delivery() {
        let records = vec![call_with_content(
            "agent_job_result",
            serde_json::json!({"job_id": "job-7"}),
            false,
            0,
            r#"{"job_id":"job-7","status":"uncertain","result_available":false,"result":null}"#,
        )];
        let report = evaluate_tool_receipts(&records, true, 0);

        assert_eq!(report.disposition, VerificationDisposition::NeedsCorrection);
        assert_eq!(report.gaps[0].code, "subject_unsuccessful");
    }

    #[test]
    fn structured_subject_identity_and_result_are_only_retained_as_digests() {
        let record = call_with_content(
            "agent_job_result",
            serde_json::json!({"job_id": "private-job-7"}),
            false,
            0,
            r#"{"job_id":"private-job-7","status":"succeeded","result_available":true,"result":"private delegated result"}"#,
        );
        let serialized = serde_json::to_string(&record.verification).unwrap();

        assert!(!serialized.contains("private-job-7"));
        assert!(!serialized.contains("private delegated result"));
        assert_eq!(
            record.verification.as_ref().unwrap().evidence,
            EvidenceStrength::Check
        );
    }

    #[test]
    fn malformed_native_artifact_receipt_cannot_prove_publication() {
        let records = vec![call_with_content(
            "artifact_publish",
            serde_json::json!({"path": "report.pdf", "title": "Report"}),
            false,
            0,
            r#"{"success":true}"#,
        )];
        let report = evaluate_tool_receipts(&records, true, 0);

        assert_eq!(report.disposition, VerificationDisposition::NeedsCorrection);
        assert_eq!(report.gaps[0].code, "receipt_unconfirmed");
    }

    #[test]
    fn failed_external_effect_requires_resolution() {
        let records = vec![call(
            "email_send",
            serde_json::json!({"account": "primary"}),
            true,
            0,
        )];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::NeedsCorrection
        );
    }

    #[test]
    fn later_same_scope_success_resolves_failed_effect() {
        let records = vec![
            call(
                "email_send",
                serde_json::json!({"account": "primary"}),
                true,
                0,
            ),
            call(
                "email_send",
                serde_json::json!({"account": "primary"}),
                false,
                1,
            ),
        ];
        assert_eq!(
            evaluate_tool_receipts(&records, true, 0).disposition,
            VerificationDisposition::Verified
        );
    }

    #[test]
    fn circuit_breaker_finishes_incomplete_after_two_corrections() {
        let records = vec![call(
            "file_write",
            serde_json::json!({"path": "notes.txt", "content": "done"}),
            false,
            0,
        )];
        assert_eq!(
            evaluate_tool_receipts(&records, true, MAX_VERIFICATION_CORRECTION_ROUNDS).disposition,
            VerificationDisposition::Incomplete
        );
    }

    #[test]
    fn shell_classifier_separates_checks_from_mutations() {
        assert_eq!(
            classify_command("cargo test --workspace"),
            WorkEffect::Verification
        );
        assert_eq!(
            classify_command("git status --short"),
            WorkEffect::Verification
        );
        assert_eq!(
            classify_command("git add README.md"),
            WorkEffect::LocalMutation
        );
        assert_eq!(classify_command("cargo fmt"), WorkEffect::LocalMutation);
        assert_eq!(
            classify_command("cargo fmt --check"),
            WorkEffect::Verification
        );
        assert_eq!(classify_command("uptime"), WorkEffect::Observation);
        assert_eq!(
            classify_command("set -o pipefail\nprintf 'host\\n'\nuptime || true"),
            WorkEffect::Observation
        );
        assert_eq!(
            classify_command("git push origin main"),
            WorkEffect::ExternalEffect
        );
        assert_eq!(classify_command(""), WorkEffect::LocalMutation);

        let remote_status = ToolCall {
            id: "remote-status".to_string(),
            name: "ssh_exec".to_string(),
            input: serde_json::json!({"command": "systemctl status tempo"}),
        };
        let remote_restart = ToolCall {
            id: "remote-restart".to_string(),
            name: "ssh_exec".to_string(),
            input: serde_json::json!({"command": "systemctl restart tempo"}),
        };
        assert_eq!(classify_tool_call(&remote_status), WorkEffect::Verification);
        assert_eq!(
            classify_tool_call(&remote_restart),
            WorkEffect::ExternalEffect
        );
    }

    #[test]
    fn distributed_shell_classifier_fails_unknown_work_closed() {
        let effect = |command: &str| {
            classify_distributed_tool_effect("shell_exec", &serde_json::json!({"command": command}))
        };
        assert_eq!(effect("pwd"), WorkEffect::Observation);
        assert_eq!(effect("cargo test --workspace"), WorkEffect::LocalMutation);
        assert_eq!(effect("git status --short"), WorkEffect::LocalMutation);
        assert_eq!(effect("git add README.md"), WorkEffect::LocalMutation);
        assert_eq!(effect("git push origin main"), WorkEffect::ExternalEffect);
        assert_eq!(
            effect("custom-deploy production"),
            WorkEffect::ExternalEffect
        );
        assert_eq!(effect(""), WorkEffect::ExternalEffect);
        assert_eq!(
            classify_distributed_tool_effect("shell_exec", &serde_json::json!({})),
            WorkEffect::ExternalEffect
        );
    }

    #[test]
    fn receipt_persists_only_hashes_for_sensitive_input() {
        let record = call(
            "file_write",
            serde_json::json!({"path": "secret.txt", "content": "must-not-survive"}),
            false,
            0,
        );
        let serialized = serde_json::to_string(&record.verification).unwrap();
        assert!(!serialized.contains("secret.txt"));
        assert!(!serialized.contains("must-not-survive"));
        assert!(serialized.contains("input_sha256"));
    }
}
