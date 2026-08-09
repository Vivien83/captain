//! Execution approval manager — gates dangerous operations behind human approval.

use captain_types::approval::{
    is_valid_approval_action_digest, normalize_approval_reason, ApprovalDecision, ApprovalOutcome,
    ApprovalPolicy, ApprovalRequest, ApprovalResponse, ApprovalRule, ApprovalRuleEffect, RiskLevel,
};
use captain_types::approval_suggestions::{ApprovalSuggestion, ApprovalSuggestionStatus};
use chrono::Utc;
use dashmap::DashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::approval_rules::ApprovalRuleStore;
use crate::approval_suggestions::ApprovalSuggestionStore;
use captain_runtime::audit::{AuditAction, AuditLog};

/// Max pending requests per agent.
const MAX_PENDING_PER_AGENT: usize = 5;

/// Manages approval requests with oneshot channels for blocking resolution.
pub struct ApprovalManager {
    pending: DashMap<Uuid, PendingRequest>,
    policy: RwLock<ApprovalPolicy>,
    /// Session approval cache scoped to the exact agent/tool/action tuple.
    /// Cleared on daemon restart. Populated when user picks
    /// a session-scoped allow or deny decision.
    session_cache: DashMap<ApprovalActionKey, ApprovalOutcome>,
    rules: ApprovalRuleStore,
    suggestions: ApprovalSuggestionStore,
    suggestion_error: RwLock<Option<String>>,
    audit_log: Option<Arc<AuditLog>>,
    resolution_lock: Mutex<()>,
}

struct PendingRequest {
    request: ApprovalRequest,
    sender: tokio::sync::oneshot::Sender<ApprovalOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalActionKey {
    agent_id: String,
    tool_name: String,
    action_digest: String,
}

impl ApprovalActionKey {
    fn from_request(request: &ApprovalRequest) -> Self {
        Self {
            agent_id: request.agent_id.clone(),
            tool_name: request.tool_name.clone(),
            action_digest: request.action_digest.clone(),
        }
    }
}

impl ApprovalManager {
    pub fn new(policy: ApprovalPolicy) -> Self {
        Self {
            pending: DashMap::new(),
            policy: RwLock::new(policy),
            session_cache: DashMap::new(),
            rules: ApprovalRuleStore::in_memory(),
            suggestions: ApprovalSuggestionStore::in_memory(),
            suggestion_error: RwLock::new(None),
            audit_log: None,
            resolution_lock: Mutex::new(()),
        }
    }

    /// Create a manager backed by a crash-safe, human-readable exact-rule file.
    pub fn with_persistence(
        policy: ApprovalPolicy,
        captain_home: &Path,
        audit_log: Arc<AuditLog>,
    ) -> Result<Self, String> {
        let rules = ApprovalRuleStore::load(captain_home.join("approval-rules.json"))?;
        let suggestion_path = captain_home.join("approval-suggestions.json");
        let (suggestions, mut suggestion_error) = match ApprovalSuggestionStore::load(
            suggestion_path,
        ) {
            Ok(store) => (store, None),
            Err(error) => {
                warn!(error = %error, "Approval suggestions disabled by persistence circuit breaker");
                (ApprovalSuggestionStore::in_memory(), Some(error))
            }
        };
        if suggestion_error.is_none() {
            let covered = rules
                .list()
                .into_iter()
                .map(|rule| (rule.agent_id, rule.tool_name, rule.action_digest))
                .collect();
            if let Err(error) = suggestions.remove_covered_bindings(&covered) {
                warn!(error = %error, "Approval suggestions disabled during boot reconciliation");
                suggestion_error = Some(error);
            }
        }
        Ok(Self {
            pending: DashMap::new(),
            policy: RwLock::new(policy),
            session_cache: DashMap::new(),
            rules,
            suggestions,
            suggestion_error: RwLock::new(suggestion_error),
            audit_log: Some(audit_log),
            resolution_lock: Mutex::new(()),
        })
    }

    /// Check if a tool requires approval based on current policy.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
        policy.require_approval.iter().any(|t| t == tool_name)
    }

    /// Submit an approval request. Broad administrator policy remains a
    /// compatibility override; interactive session and durable decisions are
    /// always scoped to the exact `(agent, tool, action digest)` tuple.
    pub async fn request_approval(&self, req: ApprovalRequest) -> ApprovalOutcome {
        if !is_valid_approval_action_digest(&req.action_digest) {
            warn!(
                agent = %req.agent_id,
                tool = %req.tool_name,
                "Approval request denied: malformed exact-action digest"
            );
            return ApprovalOutcome {
                decision: ApprovalDecision::Denied,
                reason: Some(
                    "Approval request was malformed and was denied without executing the action."
                        .to_string(),
                ),
                rule_id: None,
            };
        }
        let cache_key = ApprovalActionKey::from_request(&req);

        // 1. Durable exact-action rule. Explicit deny wins over broad config.
        if let Some(rule) = self.rules.matching(
            &cache_key.agent_id,
            &cache_key.tool_name,
            &cache_key.action_digest,
        ) {
            let decision = match rule.effect {
                ApprovalRuleEffect::Allow => ApprovalDecision::ApprovedAlways,
                ApprovalRuleEffect::Deny => ApprovalDecision::DeniedAlways,
            };
            let outcome = ApprovalOutcome {
                decision,
                reason: rule.reason.clone(),
                rule_id: Some(rule.id),
            };
            self.audit_outcome(&req, &outcome, "durable-rule", "applied");
            debug!(
                agent = %req.agent_id,
                tool = %req.tool_name,
                rule_id = %rule.id,
                ?decision,
                "Approval resolved by durable exact-action rule"
            );
            return outcome;
        }

        // 2. Administrator-configured broad allow shortcut.
        {
            let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
            if policy.allow_always.iter().any(|t| t == &req.tool_name) {
                debug!(
                    tool = %req.tool_name,
                    "Approval auto-granted: tool in allow_always policy"
                );
                return ApprovalOutcome::from_decision(ApprovalDecision::ApprovedAlways);
            }
        }
        // 3. Exact session-cache shortcut (in-memory until daemon restart).
        if let Some(outcome) = self.session_cache.get(&cache_key) {
            debug!(
                agent = %req.agent_id, tool = %req.tool_name,
                decision = ?outcome.decision,
                "Approval resolved by exact session rule"
            );
            return outcome.clone();
        }

        // Check per-agent pending limit
        let agent_pending = self
            .pending
            .iter()
            .filter(|r| r.value().request.agent_id == req.agent_id)
            .count();
        if agent_pending >= MAX_PENDING_PER_AGENT {
            warn!(agent_id = %req.agent_id, "Approval request rejected: too many pending");
            return ApprovalOutcome {
                decision: ApprovalDecision::Denied,
                reason: Some(
                    "Too many approval requests are already pending for this agent.".to_string(),
                ),
                rule_id: None,
            };
        }

        let timeout = std::time::Duration::from_secs(req.timeout_secs);
        let id = req.id;

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.insert(
            id,
            PendingRequest {
                request: req,
                sender: tx,
            },
        );

        info!(request_id = %id, "Approval request submitted, waiting for resolution");

        let mut rx = rx;
        tokio::select! {
            result = &mut rx => {
                let decision = result.unwrap_or_else(|_| ApprovalOutcome {
                    decision: ApprovalDecision::TimedOut,
                    reason: Some("Approval resolver closed without a decision.".to_string()),
                    rule_id: None,
                });
                debug!(request_id = %id, ?decision, "Approval resolved");
                decision
            }
            _ = tokio::time::sleep(timeout) => {
                let _resolution = self
                    .resolution_lock
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                // A resolver that entered the critical section just before the
                // deadline may complete while this branch waits for the lock.
                // Keep the receiver alive so that exact outcome wins over a
                // false timeout after a session or durable rule was committed.
                if let Ok(outcome) = rx.try_recv() {
                    return outcome;
                }
                self.pending.remove(&id);
                warn!(request_id = %id, "Approval request timed out");
                ApprovalOutcome {
                    decision: ApprovalDecision::TimedOut,
                    reason: Some("Approval expired before an operator responded.".to_string()),
                    rule_id: None,
                }
            }
        }
    }

    fn persist_outcome(
        &self,
        request: &ApprovalRequest,
        outcome: &mut ApprovalOutcome,
        decided_by: &str,
    ) -> Result<(), String> {
        let cache_key = ApprovalActionKey::from_request(request);
        match outcome.decision {
            ApprovalDecision::ApprovedSession | ApprovalDecision::DeniedSession => {
                self.session_cache
                    .insert(cache_key.clone(), outcome.clone());
                info!(
                    agent = %cache_key.agent_id,
                    tool = %cache_key.tool_name,
                    decision = ?outcome.decision,
                    "Exact approval decision cached for session"
                );
            }
            ApprovalDecision::ApprovedAlways | ApprovalDecision::DeniedAlways => {
                if outcome.decision == ApprovalDecision::DeniedAlways && outcome.reason.is_none() {
                    return Err("a durable deny rule requires an operator reason".to_string());
                }
                if let Some(reason) = outcome.reason.as_deref() {
                    if let Some(kind) = captain_runtime::memory_policy::scan_for_secrets(reason) {
                        return Err(format!(
                            "approval reason contains secret-like material ({kind}); remove it before persisting the rule"
                        ));
                    }
                }
                let effect = if outcome.decision.is_approved() {
                    ApprovalRuleEffect::Allow
                } else {
                    ApprovalRuleEffect::Deny
                };
                let rule = self.rules.upsert(ApprovalRule {
                    id: Uuid::new_v4(),
                    effect,
                    agent_id: cache_key.agent_id.clone(),
                    tool_name: cache_key.tool_name.clone(),
                    action_digest: cache_key.action_digest.clone(),
                    created_at: Utc::now(),
                    created_by: decided_by.to_string(),
                    reason: outcome.reason.clone(),
                })?;
                outcome.rule_id = Some(rule.id);
                info!(
                    agent = %cache_key.agent_id,
                    tool = %cache_key.tool_name,
                    rule_id = %rule.id,
                    ?effect,
                    "Exact approval rule persisted"
                );
            }
            _ => {}
        }
        Ok(())
    }

    /// Q.11 — Clear the session cache (e.g. on user logout).
    pub fn clear_session_cache(&self) {
        let n = self.session_cache.len();
        self.session_cache.clear();
        info!(removed = n, "Session approval cache cleared");
    }

    /// Number of cached exact `(agent, tool, action digest)` session rules.
    pub fn session_cache_size(&self) -> usize {
        self.session_cache.len()
    }

    /// Resolve a pending request (called by API/UI).
    pub fn resolve(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> Result<ApprovalResponse, String> {
        self.resolve_with_reason(request_id, decision, None, decided_by)
    }

    /// Resolve a pending request with bounded operator context.
    pub fn resolve_with_reason(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        reason: Option<&str>,
        decided_by: Option<String>,
    ) -> Result<ApprovalResponse, String> {
        let reason = normalize_approval_reason(reason)?;
        if let Some(reason) = reason.as_deref() {
            if let Some(kind) = captain_runtime::memory_policy::scan_for_secrets(reason) {
                return Err(format!(
                    "approval reason contains secret-like material ({kind}); remove it before sending the decision"
                ));
            }
        }
        let actor = normalize_actor(decided_by.as_deref())?;
        let _resolution = self
            .resolution_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let request = self
            .pending
            .get(&request_id)
            .map(|pending| pending.request.clone())
            .ok_or_else(|| format!("No pending approval request with id {request_id}"))?;
        let expires_at = request.requested_at
            + chrono::Duration::seconds(request.timeout_secs.min(i64::MAX as u64) as i64);
        if Utc::now() >= expires_at {
            self.pending.remove(&request_id);
            return Err(format!("Approval request {request_id} has expired"));
        }

        let mut outcome = ApprovalOutcome {
            decision,
            reason,
            rule_id: None,
        };
        self.persist_outcome(&request, &mut outcome, &actor)?;
        let suggestion = self.record_approval_suggestion(&request, decision);
        let (_, pending) = self
            .pending
            .remove(&request_id)
            .ok_or_else(|| format!("Approval request {request_id} was already resolved"))?;
        let response = ApprovalResponse {
            request_id,
            decision,
            decided_at: Utc::now(),
            decided_by: Some(actor.clone()),
            reason: outcome.reason.clone(),
            rule_id: outcome.rule_id,
            suggestion,
        };
        let _ = pending.sender.send(outcome.clone());
        self.audit_outcome(&request, &outcome, &actor, "operator");
        info!(request_id = %request_id, ?decision, rule_id = ?outcome.rule_id, "Approval request resolved");
        Ok(response)
    }

    /// List all pending requests (for API/dashboard display).
    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        self.pending
            .iter()
            .map(|r| r.value().request.clone())
            .collect()
    }

    /// Number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// List durable exact-action rules without exposing raw action payloads.
    pub fn list_rules(&self) -> Vec<ApprovalRule> {
        self.rules.list()
    }

    /// List pending, exact-action suggestions without raw action material.
    pub fn list_suggestions(&self) -> Vec<ApprovalSuggestion> {
        if self.ensure_suggestions_available().is_err() {
            return Vec::new();
        }
        let policy = self
            .policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .suggestions
            .clone();
        if !policy.enabled || policy.validate().is_err() {
            return Vec::new();
        }
        self.suggestions
            .list_pending(&policy, Utc::now())
            .into_iter()
            .filter(|suggestion| {
                self.rules
                    .matching(
                        &suggestion.agent_id,
                        &suggestion.tool_name,
                        &suggestion.action_digest,
                    )
                    .is_none()
            })
            .collect()
    }

    pub fn suggestion_status(&self) -> ApprovalSuggestionStatus {
        let policy = self
            .policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .suggestions
            .clone();
        let policy_valid = policy.validate().is_ok();
        let enabled = policy.enabled;
        let blocked = self
            .suggestion_error
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_some();
        ApprovalSuggestionStatus {
            enabled,
            healthy: !blocked && policy_valid,
            pending_count: if blocked || !enabled || !policy_valid {
                0
            } else {
                self.list_suggestions().len()
            },
            blocked_reason: if blocked {
                Some("approval suggestion persistence is unavailable".to_string())
            } else if !policy_valid {
                Some("approval suggestion policy is invalid".to_string())
            } else {
                None
            },
        }
    }

    /// Convert one pending suggestion into the existing revocable exact rule.
    pub fn accept_suggestion(
        &self,
        id: Uuid,
        decided_by: Option<&str>,
    ) -> Result<ApprovalRule, String> {
        let actor = normalize_actor(decided_by)?;
        let _resolution = self
            .resolution_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.ensure_suggestions_available()?;
        let policy = self
            .policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .suggestions
            .clone();
        if !policy.enabled {
            return Err(
                "approval suggestions are disabled; enable them explicitly first".to_string(),
            );
        }
        policy.validate()?;
        let pending = self
            .suggestions
            .pending_for_accept(id, &policy, Utc::now())
            .ok_or_else(|| format!("No pending approval suggestion with id {id}"))?;

        if let Some(existing) = self.rules.matching(
            &pending.suggestion.agent_id,
            &pending.suggestion.tool_name,
            &pending.suggestion.action_digest,
        ) {
            if existing.effect != ApprovalRuleEffect::Allow {
                return Err("a durable deny already covers this exact action".to_string());
            }
            if let Err(error) = self.suggestions.remove(id) {
                self.trip_suggestion_circuit(error);
            }
            return Ok(existing);
        }

        let rule = self.rules.upsert(ApprovalRule {
            id: pending.proposed_rule_id,
            effect: ApprovalRuleEffect::Allow,
            agent_id: pending.suggestion.agent_id.clone(),
            tool_name: pending.suggestion.tool_name.clone(),
            action_digest: pending.suggestion.action_digest.clone(),
            created_at: Utc::now(),
            created_by: actor.clone(),
            reason: None,
        })?;
        if let Err(error) = self.suggestions.remove(id) {
            self.trip_suggestion_circuit(error);
        }
        if let Some(audit) = self.audit_log.as_ref() {
            audit.record_or_alert(
                rule.agent_id.clone(),
                AuditAction::ApprovalDecision,
                format!(
                    "suggestion_id={id} rule_id={} tool={} action_digest={} actor={actor}",
                    rule.id, rule.tool_name, rule.action_digest
                ),
                "suggestion-accepted",
            );
        }
        Ok(rule)
    }

    pub fn dismiss_suggestion(
        &self,
        id: Uuid,
        decided_by: Option<&str>,
    ) -> Result<Option<ApprovalSuggestion>, String> {
        let actor = normalize_actor(decided_by)?;
        let _resolution = self
            .resolution_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.ensure_suggestions_available()?;
        let policy = self
            .policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .suggestions
            .clone();
        if !policy.enabled {
            return Err(
                "approval suggestions are disabled; enable them explicitly first".to_string(),
            );
        }
        policy.validate()?;
        let dismissed = self
            .suggestions
            .dismiss(id, &policy, Utc::now())
            .inspect_err(|error| {
                self.trip_suggestion_circuit(error.clone());
            })?;
        if let (Some(audit), Some(suggestion)) = (self.audit_log.as_ref(), dismissed.as_ref()) {
            audit.record_or_alert(
                suggestion.agent_id.clone(),
                AuditAction::ApprovalDecision,
                format!(
                    "suggestion_id={} tool={} action_digest={} actor={actor}",
                    suggestion.id, suggestion.tool_name, suggestion.action_digest
                ),
                "suggestion-dismissed",
            );
        }
        Ok(dismissed)
    }

    /// Revoke one durable rule and record the operator action.
    pub fn revoke_rule(
        &self,
        id: Uuid,
        decided_by: Option<&str>,
    ) -> Result<Option<ApprovalRule>, String> {
        let actor = normalize_actor(decided_by)?;
        let revoked = self.rules.revoke(id)?;
        if let Some(rule) = revoked.as_ref() {
            if let Some(audit) = self.audit_log.as_ref() {
                audit.record_or_alert(
                    rule.agent_id.clone(),
                    AuditAction::ApprovalDecision,
                    format!(
                        "rule_id={} tool={} action_digest={} actor={actor}",
                        rule.id, rule.tool_name, rule.action_digest
                    ),
                    "revoked",
                );
            }
        }
        Ok(revoked)
    }

    /// Update the approval policy (for hot-reload).
    pub fn update_policy(&self, policy: ApprovalPolicy) {
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy;
    }

    /// Get a copy of the current policy.
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn record_approval_suggestion(
        &self,
        request: &ApprovalRequest,
        decision: ApprovalDecision,
    ) -> Option<ApprovalSuggestion> {
        if self.ensure_suggestions_available().is_err() {
            return None;
        }
        let policy = self
            .policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .suggestions
            .clone();
        if !policy.enabled {
            return None;
        }
        if let Err(error) = policy.validate() {
            warn!(error = %error, "Approval suggestion policy is invalid");
            return None;
        }
        match self
            .suggestions
            .observe(&policy, request, decision, Utc::now())
        {
            Ok(suggestion) => suggestion,
            Err(error) => {
                self.trip_suggestion_circuit(error);
                None
            }
        }
    }

    fn ensure_suggestions_available(&self) -> Result<(), String> {
        if self
            .suggestion_error
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            Err("approval suggestion persistence is unavailable".to_string())
        } else {
            Ok(())
        }
    }

    fn trip_suggestion_circuit(&self, error: String) {
        warn!(error = %error, "Approval suggestion persistence circuit breaker opened");
        *self
            .suggestion_error
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
    }

    /// Classify the risk level of a tool invocation.
    pub fn classify_risk(tool_name: &str) -> RiskLevel {
        match tool_name {
            "shell_exec" => RiskLevel::Critical,
            "file_write" | "file_delete" => RiskLevel::High,
            "web_fetch" | "web_download" | "browser_navigate" => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }

    fn audit_outcome(
        &self,
        request: &ApprovalRequest,
        outcome: &ApprovalOutcome,
        actor: &str,
        source: &str,
    ) {
        let Some(audit) = self.audit_log.as_ref() else {
            return;
        };
        let detail = format!(
            "request_id={} tool={} action_digest={} actor={} source={} rule_id={}",
            request.id,
            request.tool_name,
            request.action_digest,
            actor,
            source,
            outcome
                .rule_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string())
        );
        let mut status = format!("{:?}", outcome.decision).to_lowercase();
        if let Some(reason) = outcome.reason.as_deref() {
            status.push_str(": ");
            status.push_str(reason);
        }
        audit.record_or_alert(
            request.agent_id.clone(),
            AuditAction::ApprovalDecision,
            detail,
            status,
        );
    }
}

fn normalize_actor(actor: Option<&str>) -> Result<String, String> {
    let actor = actor.unwrap_or("operator");
    if actor.chars().any(char::is_control) {
        return Err("decided_by must not contain control characters".to_string());
    }
    let actor = actor.trim();
    if actor.is_empty() || actor.chars().count() > 128 {
        return Err("decided_by must contain 1..=128 characters".to_string());
    }
    Ok(actor.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::approval::ApprovalPolicy;
    use std::sync::Arc;

    fn default_manager() -> ApprovalManager {
        ApprovalManager::new(ApprovalPolicy::default())
    }

    fn make_request(agent_id: &str, tool_name: &str, timeout_secs: u64) -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            description: "test operation".to_string(),
            action_summary: "test action".to_string(),
            action_digest: captain_types::approval::approval_action_digest(
                tool_name,
                b"test action",
            ),
            risk_level: RiskLevel::High,
            requested_at: Utc::now(),
            timeout_secs,
        }
    }

    // -----------------------------------------------------------------------
    // requires_approval
    // -----------------------------------------------------------------------

    #[test]
    fn test_requires_approval_default() {
        let mgr = default_manager();
        // Default policy: auto_approve=true, require_approval=[] → nothing requires approval
        assert!(!mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_read"));
    }

    #[test]
    fn test_requires_approval_custom_policy() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string(), "file_delete".to_string()],
            allow_always: vec![],
            timeout_secs: 30,
            auto_approve_autonomous: false,
            auto_approve: false,
            suggestions: Default::default(),
        };
        let mgr = ApprovalManager::new(policy);
        assert!(mgr.requires_approval("file_write"));
        assert!(mgr.requires_approval("file_delete"));
        assert!(!mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_read"));
    }

    // -----------------------------------------------------------------------
    // classify_risk
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_risk() {
        assert_eq!(
            ApprovalManager::classify_risk("shell_exec"),
            RiskLevel::Critical
        );
        assert_eq!(
            ApprovalManager::classify_risk("file_write"),
            RiskLevel::High
        );
        assert_eq!(
            ApprovalManager::classify_risk("file_delete"),
            RiskLevel::High
        );
        assert_eq!(
            ApprovalManager::classify_risk("web_fetch"),
            RiskLevel::Medium
        );
        assert_eq!(
            ApprovalManager::classify_risk("web_download"),
            RiskLevel::Medium
        );
        assert_eq!(
            ApprovalManager::classify_risk("browser_navigate"),
            RiskLevel::Medium
        );
        assert_eq!(ApprovalManager::classify_risk("file_read"), RiskLevel::Low);
        assert_eq!(
            ApprovalManager::classify_risk("unknown_tool"),
            RiskLevel::Low
        );
    }

    // -----------------------------------------------------------------------
    // resolve nonexistent
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_nonexistent() {
        let mgr = default_manager();
        let result = mgr.resolve(Uuid::new_v4(), ApprovalDecision::Approved, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending approval request"));
    }

    // -----------------------------------------------------------------------
    // list_pending empty
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_pending_empty() {
        let mgr = default_manager();
        assert!(mgr.list_pending().is_empty());
    }

    // -----------------------------------------------------------------------
    // update_policy
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_policy() {
        let mgr = default_manager();
        // Default: auto_approve=true → nothing requires approval
        assert!(!mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_write"));

        let new_policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            allow_always: vec![],
            timeout_secs: 120,
            auto_approve_autonomous: true,
            auto_approve: false,
            suggestions: Default::default(),
        };
        mgr.update_policy(new_policy);

        assert!(!mgr.requires_approval("shell_exec"));
        assert!(mgr.requires_approval("file_write"));

        let policy = mgr.policy();
        assert_eq!(policy.timeout_secs, 120);
        assert!(policy.auto_approve_autonomous);
    }

    // -----------------------------------------------------------------------
    // pending_count
    // -----------------------------------------------------------------------

    #[test]
    fn test_pending_count() {
        let mgr = default_manager();
        assert_eq!(mgr.pending_count(), 0);
    }

    // -----------------------------------------------------------------------
    // request_approval — timeout
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_timeout() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 10);
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::TimedOut);
        // After timeout, pending map should be cleaned up
        assert_eq!(mgr.pending_count(), 0);
    }

    // -----------------------------------------------------------------------
    // request_approval — approve
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_approve() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            // Small delay to let the request register
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(
                request_id,
                ApprovalDecision::Approved,
                Some("admin".to_string()),
            );
            assert!(result.is_ok());
            let resp = result.unwrap();
            assert_eq!(resp.decision, ApprovalDecision::Approved);
            assert_eq!(resp.decided_by, Some("admin".to_string()));
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
    }

    // -----------------------------------------------------------------------
    // request_approval — deny
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_deny() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(request_id, ApprovalDecision::Denied, None);
            assert!(result.is_ok());
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Denied);
    }

    // -----------------------------------------------------------------------
    // max pending per agent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_max_pending_per_agent() {
        let mgr = Arc::new(default_manager());

        // Fill up 5 pending requests for agent-1 (they will all be waiting)
        let mut ids = Vec::new();
        for _ in 0..MAX_PENDING_PER_AGENT {
            let req = make_request("agent-1", "shell_exec", 300);
            ids.push(req.id);
            let mgr_clone = Arc::clone(&mgr);
            tokio::spawn(async move {
                mgr_clone.request_approval(req).await;
            });
        }

        // Give spawned tasks time to register
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT);

        // 6th request for the same agent should be immediately denied
        let req6 = make_request("agent-1", "shell_exec", 300);
        let decision = mgr.request_approval(req6).await;
        assert_eq!(decision, ApprovalDecision::Denied);

        // A different agent should still be able to submit
        let req_other = make_request("agent-2", "shell_exec", 300);
        let other_id = req_other.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr2.request_approval(req_other).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT + 1);

        // Cleanup: resolve all pending to avoid hanging tasks
        for id in &ids {
            let _ = mgr.resolve(*id, ApprovalDecision::Denied, None);
        }
        let _ = mgr.resolve(other_id, ApprovalDecision::Denied, None);
    }

    // -----------------------------------------------------------------------
    // policy defaults
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Q.11 — allow_always shortcut
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_q11_allow_always_short_circuits_to_approved_always() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            allow_always: vec!["file_write".to_string()],
            timeout_secs: 60,
            auto_approve_autonomous: false,
            auto_approve: false,
            suggestions: Default::default(),
        };
        let mgr = ApprovalManager::new(policy);
        let req = make_request("agent-x", "file_write", 60);
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::ApprovedAlways);
        assert!(decision.is_approved());
        assert_eq!(mgr.pending_count(), 0, "no prompt should have been created");
    }

    // -----------------------------------------------------------------------
    // Q.11 — session cache shortcut
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_q11_session_cache_short_circuits_after_first_approval() {
        let mgr = Arc::new(default_manager());

        // First call → user picks ApprovedSession via resolve()
        let req1 = make_request("agent-y", "shell_exec", 60);
        let id1 = req1.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = mgr2.resolve(
                id1,
                ApprovalDecision::ApprovedSession,
                Some("user".to_string()),
            );
        });
        let d1 = mgr.request_approval(req1).await;
        assert_eq!(d1, ApprovalDecision::ApprovedSession);
        assert_eq!(mgr.session_cache_size(), 1);

        // Same exact action → short-circuited, no resolve needed.
        let req2 = make_request("agent-y", "shell_exec", 60);
        let d2 = mgr.request_approval(req2).await;
        assert_eq!(d2, ApprovalDecision::ApprovedSession);
        assert_eq!(mgr.pending_count(), 0);

        // Same agent and tool but a different action must never inherit the
        // cached decision.
        let mut req_different_action = make_request("agent-y", "shell_exec", 1);
        req_different_action.action_summary = "a different command".to_string();
        req_different_action.action_digest =
            captain_types::approval::approval_action_digest("shell_exec", b"a different command");
        assert_eq!(
            mgr.request_approval(req_different_action).await,
            ApprovalDecision::TimedOut,
            "a session decision must be bound to the exact action digest"
        );

        // Different tool for same agent → still prompted (separate cache key)
        let req3 = make_request("agent-y", "file_write", 1); // 1s timeout to avoid blocking
        let d3 = mgr.request_approval(req3).await;
        assert_eq!(
            d3,
            ApprovalDecision::TimedOut,
            "different tool must prompt (timeout here)"
        );
    }

    #[tokio::test]
    async fn exact_decision_never_uses_a_truncated_display_preview_as_its_binding() {
        let mgr = Arc::new(default_manager());
        let preview = "x".repeat(512);
        let mut first = make_request("agent-y", "shell_exec", 60);
        first.action_summary = preview.clone();
        first.action_digest = captain_types::approval::approval_action_digest(
            "shell_exec",
            format!("{preview}A").as_bytes(),
        );
        let id = first.id;
        let resolver = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            resolver
                .resolve(
                    id,
                    ApprovalDecision::ApprovedSession,
                    Some("test".to_string()),
                )
                .unwrap();
        });
        assert_eq!(
            mgr.request_approval(first).await,
            ApprovalDecision::ApprovedSession
        );

        let mut second = make_request("agent-y", "shell_exec", 1);
        second.action_summary = preview.clone();
        second.action_digest = captain_types::approval::approval_action_digest(
            "shell_exec",
            format!("{preview}B").as_bytes(),
        );
        assert_eq!(
            mgr.request_approval(second).await,
            ApprovalDecision::TimedOut,
            "identical truncated previews must not share an approval decision"
        );
    }

    #[tokio::test]
    async fn malformed_action_digest_fails_closed_before_queueing() {
        let mgr = default_manager();
        let mut request = make_request("agent-y", "shell_exec", 60);
        request.action_digest = "invalid".to_string();

        let outcome = mgr.request_approval(request).await;

        assert_eq!(outcome, ApprovalDecision::Denied);
        assert_eq!(mgr.pending_count(), 0);
    }

    // -----------------------------------------------------------------------
    // Durable exact-action rule persistence
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn approved_always_persists_exact_rule_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(AuditLog::new());
        let mgr = Arc::new(
            ApprovalManager::with_persistence(
                ApprovalPolicy::default(),
                dir.path(),
                Arc::clone(&audit),
            )
            .unwrap(),
        );
        assert!(mgr.policy().allow_always.is_empty());

        let req = make_request("agent-z", "shell_exec", 60);
        let exact_key = ApprovalActionKey::from_request(&req);
        let id = req.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = mgr2.resolve(id, ApprovalDecision::ApprovedAlways, None);
        });
        let d = mgr.request_approval(req).await;
        assert_eq!(d, ApprovalDecision::ApprovedAlways);

        assert!(mgr.policy().allow_always.is_empty());
        assert_eq!(mgr.list_rules().len(), 1);
        assert!(mgr
            .rules
            .matching(
                &exact_key.agent_id,
                &exact_key.tool_name,
                &exact_key.action_digest
            )
            .is_some());
        assert!(mgr
            .rules
            .matching("agent-other", "shell_exec", &exact_key.action_digest)
            .is_none());

        drop(mgr);
        let mgr = ApprovalManager::with_persistence(ApprovalPolicy::default(), dir.path(), audit)
            .unwrap();
        let req2 = make_request("agent-z", "shell_exec", 60);
        let d2 = mgr.request_approval(req2).await;
        assert_eq!(d2, ApprovalDecision::ApprovedAlways);
        assert!(d2.rule_id.is_some());
    }

    #[tokio::test]
    async fn durable_deny_returns_reason_and_can_be_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Arc::new(
            ApprovalManager::with_persistence(
                ApprovalPolicy::default(),
                dir.path(),
                Arc::new(AuditLog::new()),
            )
            .unwrap(),
        );
        let request = make_request("agent-z", "shell_exec", 60);
        let id = request.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            mgr2.resolve_with_reason(
                id,
                ApprovalDecision::DeniedAlways,
                Some("Use the staging host instead"),
                Some("test".to_string()),
            )
            .unwrap();
        });

        let outcome = mgr.request_approval(request).await;
        assert_eq!(outcome, ApprovalDecision::DeniedAlways);
        assert_eq!(
            outcome.reason.as_deref(),
            Some("Use the staging host instead")
        );
        let rule_id = outcome.rule_id.expect("durable rule id");

        let repeated = mgr
            .request_approval(make_request("agent-z", "shell_exec", 60))
            .await;
        assert_eq!(repeated, ApprovalDecision::DeniedAlways);
        assert_eq!(repeated.reason, outcome.reason);
        assert!(mgr.revoke_rule(rule_id, Some("test")).unwrap().is_some());
        assert!(mgr.list_rules().is_empty());
    }

    #[tokio::test]
    async fn durable_deny_requires_non_secret_reason_without_consuming_request() {
        let mgr = Arc::new(default_manager());
        let request = make_request("agent-z", "shell_exec", 60);
        let id = request.id;
        let mgr2 = Arc::clone(&mgr);
        let task = tokio::spawn(async move { mgr2.request_approval(request).await });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert!(mgr
            .resolve_with_reason(
                id,
                ApprovalDecision::DeniedAlways,
                None,
                Some("test".to_string())
            )
            .unwrap_err()
            .contains("requires an operator reason"));
        assert_eq!(mgr.pending_count(), 1);
        assert!(mgr
            .resolve_with_reason(
                id,
                ApprovalDecision::DeniedAlways,
                Some("Authorization: Bearer abcd1234567890abcdef=="),
                Some("test".to_string())
            )
            .unwrap_err()
            .contains("secret-like material"));
        assert_eq!(mgr.pending_count(), 1);
        mgr.resolve_with_reason(
            id,
            ApprovalDecision::Denied,
            Some("No"),
            Some("test".to_string()),
        )
        .unwrap();
        assert_eq!(task.await.unwrap().decision, ApprovalDecision::Denied);
    }

    #[tokio::test]
    async fn approval_decision_and_revocation_are_audited_without_raw_action() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(AuditLog::new());
        let mgr = Arc::new(
            ApprovalManager::with_persistence(
                ApprovalPolicy::default(),
                dir.path(),
                Arc::clone(&audit),
            )
            .unwrap(),
        );
        let mut request = make_request("agent-audit", "shell_exec", 60);
        request.action_summary = "sensitive raw command".to_string();
        request.action_digest =
            captain_types::approval::approval_action_digest("shell_exec", b"sensitive raw command");
        let id = request.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            mgr2.resolve_with_reason(
                id,
                ApprovalDecision::DeniedAlways,
                Some("Use staging"),
                Some("api:test".to_string()),
            )
            .unwrap();
        });
        let outcome = mgr.request_approval(request).await;
        mgr.revoke_rule(outcome.rule_id.unwrap(), Some("api:test"))
            .unwrap();

        let entries = audit.recent(10);
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|entry| entry.action == AuditAction::ApprovalDecision));
        assert!(entries
            .iter()
            .all(|entry| !entry.detail.contains("sensitive raw command")));
        assert!(entries[0].detail.contains("action_digest="));
        assert_eq!(entries[1].outcome, "revoked");
    }

    // -----------------------------------------------------------------------
    // Q.11 — clear_session_cache resets the session-only entries
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_q11_clear_session_cache_drops_all_pairs() {
        let mgr = Arc::new(default_manager());

        let req = make_request("a", "shell_exec", 60);
        let id = req.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = mgr2.resolve(id, ApprovalDecision::ApprovedSession, None);
        });
        let _ = mgr.request_approval(req).await;
        assert_eq!(mgr.session_cache_size(), 1);

        mgr.clear_session_cache();
        assert_eq!(mgr.session_cache_size(), 0);

        // Re-prompt confirmed: a follow-up call doesn't short-circuit any more
        let req2 = make_request("a", "shell_exec", 1);
        assert_eq!(
            mgr.request_approval(req2).await,
            ApprovalDecision::TimedOut,
            "after clear_session_cache, the next call must reach the prompt"
        );
    }

    // -----------------------------------------------------------------------
    // Q.11 — is_approved() helper covers the 3 approve variants
    // -----------------------------------------------------------------------

    #[test]
    fn test_q11_is_approved_helper() {
        assert!(ApprovalDecision::Approved.is_approved());
        assert!(ApprovalDecision::ApprovedSession.is_approved());
        assert!(ApprovalDecision::ApprovedAlways.is_approved());
        assert!(!ApprovalDecision::Denied.is_approved());
        assert!(!ApprovalDecision::DeniedSession.is_approved());
        assert!(!ApprovalDecision::DeniedAlways.is_approved());
        assert!(!ApprovalDecision::TimedOut.is_approved());
    }

    #[test]
    fn test_policy_defaults() {
        let mgr = default_manager();
        let policy = mgr.policy();
        assert_eq!(policy.require_approval, Vec::<String>::new());
        assert_eq!(policy.timeout_secs, 60);
        assert!(policy.auto_approve_autonomous);
    }
}
