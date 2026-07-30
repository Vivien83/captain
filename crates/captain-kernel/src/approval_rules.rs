use captain_types::approval::{
    is_valid_approval_action_digest, normalize_approval_reason, ApprovalRule, ApprovalRuleEffect,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use uuid::Uuid;

const RULE_FILE_VERSION: u32 = 1;
const MAX_DURABLE_RULES: usize = 256;
const MAX_RULE_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRuleFile {
    schema_version: u32,
    rules: Vec<ApprovalRule>,
}

#[derive(Debug)]
pub(super) struct ApprovalRuleStore {
    path: Option<PathBuf>,
    rules: RwLock<Vec<ApprovalRule>>,
}

impl ApprovalRuleStore {
    pub(super) fn in_memory() -> Self {
        Self {
            path: None,
            rules: RwLock::new(Vec::new()),
        }
    }

    pub(super) fn load(path: PathBuf) -> Result<Self, String> {
        let rules = if path.exists() {
            let metadata = fs::metadata(&path)
                .map_err(|e| format!("read approval rule metadata {}: {e}", path.display()))?;
            if metadata.len() > MAX_RULE_FILE_BYTES {
                return Err(format!(
                    "approval rule file {} is too large ({} bytes, max {MAX_RULE_FILE_BYTES})",
                    path.display(),
                    metadata.len()
                ));
            }
            let raw = fs::read_to_string(&path)
                .map_err(|e| format!("read approval rules {}: {e}", path.display()))?;
            let file: ApprovalRuleFile = serde_json::from_str(&raw)
                .map_err(|e| format!("parse approval rules {}: {e}", path.display()))?;
            if file.schema_version != RULE_FILE_VERSION {
                return Err(format!(
                    "unsupported approval rule schema {} in {} (expected {RULE_FILE_VERSION})",
                    file.schema_version,
                    path.display()
                ));
            }
            validate_rules(&file.rules)?;
            file.rules
        } else {
            Vec::new()
        };
        Ok(Self {
            path: Some(path),
            rules: RwLock::new(rules),
        })
    }

    pub(super) fn list(&self) -> Vec<ApprovalRule> {
        let rules = self.rules.read().unwrap_or_else(|e| e.into_inner());
        let mut out = rules.clone();
        out.sort_by_key(|rule| std::cmp::Reverse(rule.created_at));
        out
    }

    pub(super) fn matching(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_digest: &str,
    ) -> Option<ApprovalRule> {
        self.rules
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|rule| {
                rule.agent_id == agent_id
                    && rule.tool_name == tool_name
                    && rule.action_digest == action_digest
            })
            .cloned()
    }

    pub(super) fn upsert(&self, rule: ApprovalRule) -> Result<ApprovalRule, String> {
        validate_rule(&rule)?;
        let mut guard = self.rules.write().unwrap_or_else(|e| e.into_inner());
        let mut next = guard.clone();
        next.retain(|existing| {
            existing.agent_id != rule.agent_id
                || existing.tool_name != rule.tool_name
                || existing.action_digest != rule.action_digest
        });
        if next.len() >= MAX_DURABLE_RULES {
            return Err(format!(
                "approval rule limit reached ({MAX_DURABLE_RULES}); revoke an old rule first"
            ));
        }
        next.push(rule.clone());
        self.persist(&next)?;
        *guard = next;
        Ok(rule)
    }

    pub(super) fn revoke(&self, id: Uuid) -> Result<Option<ApprovalRule>, String> {
        let mut guard = self.rules.write().unwrap_or_else(|e| e.into_inner());
        let Some(rule) = guard.iter().find(|rule| rule.id == id).cloned() else {
            return Ok(None);
        };
        let mut next = guard.clone();
        next.retain(|existing| existing.id != id);
        self.persist(&next)?;
        *guard = next;
        Ok(Some(rule))
    }

    fn persist(&self, rules: &[ApprovalRule]) -> Result<(), String> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let mut payload = serde_json::to_vec_pretty(&ApprovalRuleFile {
            schema_version: RULE_FILE_VERSION,
            rules: rules.to_vec(),
        })
        .map_err(|e| format!("serialize approval rules: {e}"))?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_RULE_FILE_BYTES {
            return Err(format!(
                "serialized approval rules exceed {MAX_RULE_FILE_BYTES} bytes"
            ));
        }

        captain_types::durable_fs::atomic_write(path, &payload)
            .map_err(|e| format!("persist approval rules {}: {e}", path.display()))
    }
}

fn validate_rules(rules: &[ApprovalRule]) -> Result<(), String> {
    if rules.len() > MAX_DURABLE_RULES {
        return Err(format!(
            "approval rule file contains {} rules (max {MAX_DURABLE_RULES})",
            rules.len()
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut bindings = std::collections::HashSet::new();
    for rule in rules {
        validate_rule(rule)?;
        if !ids.insert(rule.id) {
            return Err(format!("duplicate approval rule id {}", rule.id));
        }
        let binding = (
            rule.agent_id.as_str(),
            rule.tool_name.as_str(),
            rule.action_digest.as_str(),
        );
        if !bindings.insert(binding) {
            return Err(format!(
                "conflicting duplicate approval rule for agent {} tool {} action {}",
                rule.agent_id, rule.tool_name, rule.action_digest
            ));
        }
    }
    Ok(())
}

fn validate_rule(rule: &ApprovalRule) -> Result<(), String> {
    if rule.agent_id.trim().is_empty()
        || rule.agent_id.chars().count() > 128
        || rule.agent_id.chars().any(char::is_control)
    {
        return Err("approval rule agent_id must contain 1..=128 characters".to_string());
    }
    if rule.tool_name.is_empty()
        || rule.tool_name.len() > 64
        || !rule
            .tool_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(format!(
            "approval rule has invalid tool name {:?}",
            rule.tool_name
        ));
    }
    if !is_valid_approval_action_digest(&rule.action_digest) {
        return Err("approval rule action_digest must be 64 hexadecimal characters".to_string());
    }
    if rule.created_by.trim().is_empty()
        || rule.created_by.chars().count() > 128
        || rule.created_by.chars().any(char::is_control)
    {
        return Err("approval rule created_by must contain 1..=128 characters".to_string());
    }
    let normalized = normalize_approval_reason(rule.reason.as_deref())?;
    if normalized != rule.reason {
        return Err("approval rule reason is not normalized".to_string());
    }
    if let Some(reason) = rule.reason.as_deref() {
        if let Some(kind) = captain_runtime::memory_policy::scan_for_secrets(reason) {
            return Err(format!(
                "approval rule reason contains secret-like material ({kind})"
            ));
        }
    }
    if rule.effect == ApprovalRuleEffect::Deny && rule.reason.is_none() {
        return Err("a durable deny rule requires an operator reason".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn rule(effect: ApprovalRuleEffect) -> ApprovalRule {
        ApprovalRule {
            id: Uuid::new_v4(),
            effect,
            agent_id: "captain".to_string(),
            tool_name: "shell_exec".to_string(),
            action_digest: "a".repeat(64),
            created_at: Utc::now(),
            created_by: "test".to_string(),
            reason: (effect == ApprovalRuleEffect::Deny).then(|| "Action interdite".to_string()),
        }
    }

    #[test]
    fn durable_rule_roundtrip_and_revoke_are_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approval-rules.json");
        let store = ApprovalRuleStore::load(path.clone()).unwrap();
        let created = store.upsert(rule(ApprovalRuleEffect::Deny)).unwrap();

        let reloaded = ApprovalRuleStore::load(path).unwrap();
        assert_eq!(reloaded.list(), vec![created.clone()]);
        assert_eq!(reloaded.revoke(created.id).unwrap(), Some(created));
        assert!(reloaded.list().is_empty());
    }

    #[test]
    fn upsert_replaces_conflicting_exact_action_rule() {
        let store = ApprovalRuleStore::in_memory();
        let denied = store.upsert(rule(ApprovalRuleEffect::Deny)).unwrap();
        let allowed = store.upsert(rule(ApprovalRuleEffect::Allow)).unwrap();

        assert_eq!(store.list(), vec![allowed]);
        assert_ne!(store.list()[0].id, denied.id);
    }

    #[test]
    fn corrupt_rule_file_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approval-rules.json");
        fs::write(&path, b"not-json").unwrap();

        assert!(ApprovalRuleStore::load(path).unwrap_err().contains("parse"));
    }

    #[test]
    fn manually_edited_rule_with_secret_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approval-rules.json");
        let mut unsafe_rule = rule(ApprovalRuleEffect::Deny);
        unsafe_rule.reason = Some("Authorization: Bearer abcd1234567890abcdef==".to_string());
        let payload = serde_json::to_vec(&ApprovalRuleFile {
            schema_version: RULE_FILE_VERSION,
            rules: vec![unsafe_rule],
        })
        .unwrap();
        fs::write(&path, payload).unwrap();

        assert!(ApprovalRuleStore::load(path)
            .unwrap_err()
            .contains("secret-like material"));
    }
}
