//! Execution approval types for the Captain agent OS.
//!
//! When an agent attempts a dangerous operation (e.g. `shell_exec`), the kernel
//! creates an [`ApprovalRequest`] and pauses the agent until a human operator
//! responds with an [`ApprovalResponse`]. The [`ApprovalPolicy`] configures
//! which tools require approval and how long to wait before auto-denying.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum length of tool names (chars).
const MAX_TOOL_NAME_LEN: usize = 64;

/// Maximum length of an agent identifier (chars).
const MAX_AGENT_ID_LEN: usize = 128;

/// Maximum length of a request description (chars).
const MAX_DESCRIPTION_LEN: usize = 1024;

/// Maximum length of an action summary (chars).
const MAX_ACTION_SUMMARY_LEN: usize = 512;

/// Maximum length of an operator-provided approval reason (chars).
pub const MAX_APPROVAL_REASON_LEN: usize = 280;

/// Minimum approval timeout in seconds.
const MIN_TIMEOUT_SECS: u64 = 10;

/// Maximum approval timeout in seconds.
const MAX_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// RiskLevel
// ---------------------------------------------------------------------------

/// Risk level of an operation requiring approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Returns a warning emoji suitable for display in dashboards and chat.
    pub fn emoji(&self) -> &'static str {
        match self {
            RiskLevel::Low => "\u{2139}\u{fe0f}",      // information source
            RiskLevel::Medium => "\u{26a0}\u{fe0f}",   // warning sign
            RiskLevel::High => "\u{1f6a8}",            // rotating light
            RiskLevel::Critical => "\u{2620}\u{fe0f}", // skull and crossbones
        }
    }
}

// ---------------------------------------------------------------------------
// ApprovalDecision
// ---------------------------------------------------------------------------

/// Decision on an approval request: symmetric allow/deny scopes plus timeout.
///
/// Serde accepts both English (`approved` / `approved_session` / etc.) and
/// French (`approuver` / `session` / etc.) variants — useful for TOML
/// config and FR API clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Approve this single occurrence. Next call to the same tool will
    /// re-prompt. Backwards-compatible alias of `approve_once`.
    #[serde(
        alias = "approve_once",
        alias = "approuver",
        alias = "approuver_une_fois"
    )]
    Approved,
    /// Approve this exact agent/tool/action tuple until the daemon restarts
    /// (or the session cache is cleared).
    #[serde(alias = "session", alias = "approuver_session")]
    ApprovedSession,
    /// Persist an allow rule for this exact agent/tool/action tuple. Manual
    /// `approval.allow_always` config entries remain the separate broad admin
    /// override for backwards compatibility.
    #[serde(alias = "always", alias = "approuver_toujours", alias = "toujours")]
    ApprovedAlways,
    /// Refuse this call. Next call will re-prompt.
    #[serde(alias = "decline", alias = "refuser", alias = "refuse")]
    Denied,
    /// Refuse this exact action for this agent until the daemon restarts.
    #[serde(alias = "reject_session", alias = "refuser_session")]
    DeniedSession,
    /// Refuse this exact action for this agent until its durable rule is revoked.
    #[serde(alias = "reject_always", alias = "refuser_toujours")]
    DeniedAlways,
    /// User did not respond within `timeout_secs` — treated as denial.
    #[serde(alias = "timed_out", alias = "expire")]
    TimedOut,
}

impl ApprovalDecision {
    /// True if the decision authorises execution (one of the three
    /// "approved" variants).
    pub fn is_approved(&self) -> bool {
        matches!(
            self,
            ApprovalDecision::Approved
                | ApprovalDecision::ApprovedSession
                | ApprovalDecision::ApprovedAlways
        )
    }

    /// True when the operator explicitly rejected the action.
    pub fn is_denied(&self) -> bool {
        matches!(
            self,
            ApprovalDecision::Denied
                | ApprovalDecision::DeniedSession
                | ApprovalDecision::DeniedAlways
                | ApprovalDecision::TimedOut
        )
    }

    /// True when this decision applies until daemon restart.
    pub fn is_session_scoped(&self) -> bool {
        matches!(
            self,
            ApprovalDecision::ApprovedSession | ApprovalDecision::DeniedSession
        )
    }

    /// True when this decision creates a durable exact-action rule.
    pub fn is_persistent(&self) -> bool {
        matches!(
            self,
            ApprovalDecision::ApprovedAlways | ApprovalDecision::DeniedAlways
        )
    }
}

// ---------------------------------------------------------------------------
// ApprovalOutcome / ApprovalRule
// ---------------------------------------------------------------------------

/// Complete result returned to the blocked tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalOutcome {
    pub decision: ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<Uuid>,
}

impl ApprovalOutcome {
    pub fn from_decision(decision: ApprovalDecision) -> Self {
        Self {
            decision,
            reason: None,
            rule_id: None,
        }
    }

    pub fn is_approved(&self) -> bool {
        self.decision.is_approved()
    }
}

impl From<ApprovalDecision> for ApprovalOutcome {
    fn from(decision: ApprovalDecision) -> Self {
        Self::from_decision(decision)
    }
}

impl PartialEq<ApprovalDecision> for ApprovalOutcome {
    fn eq(&self, other: &ApprovalDecision) -> bool {
        self.decision == *other
    }
}

/// Effect of one durable exact-action approval rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRuleEffect {
    Allow,
    Deny,
}

/// Human-readable metadata for a durable rule. The raw action is deliberately
/// not persisted: only its digest is stored so commands and secrets cannot leak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRule {
    pub id: Uuid,
    pub effect: ApprovalRuleEffect,
    pub agent_id: String,
    pub tool_name: String,
    pub action_digest: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Normalize an operator reason before display, propagation or persistence.
pub fn normalize_approval_reason(reason: Option<&str>) -> Result<Option<String>, String> {
    let Some(reason) = reason else {
        return Ok(None);
    };
    if reason
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err("approval reason contains control characters".to_string());
    }
    let normalized = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Ok(None);
    }
    let count = normalized.chars().count();
    if count > MAX_APPROVAL_REASON_LEN {
        return Err(format!(
            "approval reason too long ({count} chars, max {MAX_APPROVAL_REASON_LEN})"
        ));
    }
    Ok(Some(normalized))
}

// ---------------------------------------------------------------------------
// ApprovalRequest
// ---------------------------------------------------------------------------

/// An approval request for a dangerous agent operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub agent_id: String,
    pub tool_name: String,
    pub description: String,
    /// The specific action being requested (sanitized for display).
    pub action_summary: String,
    /// BLAKE3 digest of the complete action input. This is deliberately
    /// separate from `action_summary`: display previews may be truncated,
    /// while session and durable decisions must bind to the exact input.
    pub action_digest: String,
    pub risk_level: RiskLevel,
    pub requested_at: DateTime<Utc>,
    /// Auto-deny timeout in seconds.
    pub timeout_secs: u64,
}

impl ApprovalRequest {
    /// Validate this request's fields.
    ///
    /// Returns `Ok(())` or an error message describing the first validation failure.
    pub fn validate(&self) -> Result<(), String> {
        // -- agent_id --
        if self.agent_id.trim().is_empty()
            || self.agent_id.chars().count() > MAX_AGENT_ID_LEN
            || self.agent_id.chars().any(char::is_control)
        {
            return Err(format!(
                "agent_id must contain 1..={MAX_AGENT_ID_LEN} characters without controls"
            ));
        }

        // -- tool_name --
        if self.tool_name.is_empty() {
            return Err("tool_name must not be empty".into());
        }
        if self.tool_name.len() > MAX_TOOL_NAME_LEN {
            return Err(format!(
                "tool_name too long ({} chars, max {MAX_TOOL_NAME_LEN})",
                self.tool_name.len()
            ));
        }
        if !self
            .tool_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
        {
            return Err(
                "tool_name may only contain alphanumeric characters and underscores".into(),
            );
        }

        // -- description --
        if self.description.len() > MAX_DESCRIPTION_LEN {
            return Err(format!(
                "description too long ({} chars, max {MAX_DESCRIPTION_LEN})",
                self.description.len()
            ));
        }

        // -- action_summary --
        if self.action_summary.len() > MAX_ACTION_SUMMARY_LEN {
            return Err(format!(
                "action_summary too long ({} chars, max {MAX_ACTION_SUMMARY_LEN})",
                self.action_summary.len()
            ));
        }

        if !is_valid_approval_action_digest(&self.action_digest) {
            return Err("action_digest must be 64 lowercase hexadecimal characters".to_string());
        }

        // -- timeout_secs --
        if self.timeout_secs < MIN_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too small ({}, min {MIN_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }
        if self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too large ({}, max {MAX_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }

        Ok(())
    }
}

/// Compute the non-reversible binding used by session and durable approval
/// decisions. Callers must provide the complete, untruncated action input.
pub fn approval_action_digest(tool_name: &str, action_input: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new_derive_key("captain approval action digest v1");
    hasher.update(&(tool_name.len() as u64).to_le_bytes());
    hasher.update(tool_name.as_bytes());
    hasher.update(&(action_input.len() as u64).to_le_bytes());
    hasher.update(action_input);
    hasher.finalize().to_hex().to_string()
}

pub fn is_valid_approval_action_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// ---------------------------------------------------------------------------
// ApprovalResponse
// ---------------------------------------------------------------------------

/// Response to an approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResponse {
    pub request_id: Uuid,
    pub decision: ApprovalDecision,
    pub decided_at: DateTime<Utc>,
    pub decided_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<Uuid>,
}

// ---------------------------------------------------------------------------
// ApprovalPolicy
// ---------------------------------------------------------------------------

/// Configurable approval policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalPolicy {
    /// Tools that always require approval. Default: `["shell_exec"]`.
    ///
    /// Accepts either a list of tool names or a boolean shorthand:
    /// - `require_approval = false` → empty list (no tools require approval)
    /// - `require_approval = true`  → `["shell_exec"]` (the default set)
    #[serde(deserialize_with = "deserialize_require_approval")]
    pub require_approval: Vec<String>,
    /// Broad administrator-owned "always allow" list. Tools listed here are
    /// short-circuited before any prompt. Interactive persistent decisions use
    /// exact rules in `approval-rules.json` and never widen this list.
    #[serde(default)]
    pub allow_always: Vec<String>,
    /// Timeout in seconds. Default: 60, range: 10..=300.
    pub timeout_secs: u64,
    /// Auto-approve in autonomous mode. Default: `false`.
    pub auto_approve_autonomous: bool,
    /// Alias: if `auto_approve = true`, clears the require list at boot.
    #[serde(default, alias = "auto_approve")]
    pub auto_approve: bool,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            require_approval: vec![],
            allow_always: vec![],
            timeout_secs: 60,
            auto_approve_autonomous: true,
            auto_approve: true,
        }
    }
}

/// Custom deserializer that accepts:
/// - A list of strings: `["shell_exec", "file_write"]`
/// - A boolean: `false` → `[]`, `true` → `["shell_exec"]`
fn deserialize_require_approval<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct RequireApprovalVisitor;

    impl<'de> de::Visitor<'de> for RequireApprovalVisitor {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list of tool names or a boolean")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            Ok(if v {
                vec!["shell_exec".to_string()]
            } else {
                vec![]
            })
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut v = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                v.push(s);
            }
            Ok(v)
        }
    }

    deserializer.deserialize_any(RequireApprovalVisitor)
}

impl ApprovalPolicy {
    /// Apply the `auto_approve` shorthand: if true, clears the require list.
    pub fn apply_shorthands(&mut self) {
        if self.auto_approve {
            self.require_approval.clear();
        }
    }

    /// Validate this policy's fields.
    ///
    /// Returns `Ok(())` or an error message describing the first validation failure.
    pub fn validate(&self) -> Result<(), String> {
        // -- timeout_secs --
        if self.timeout_secs < MIN_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too small ({}, min {MIN_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }
        if self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!(
                "timeout_secs too large ({}, max {MAX_TIMEOUT_SECS})",
                self.timeout_secs
            ));
        }

        // -- require_approval tool names --
        for (i, name) in self.require_approval.iter().enumerate() {
            if name.is_empty() {
                return Err(format!("require_approval[{i}] must not be empty"));
            }
            if name.len() > MAX_TOOL_NAME_LEN {
                return Err(format!(
                    "require_approval[{i}] too long ({} chars, max {MAX_TOOL_NAME_LEN})",
                    name.len()
                ));
            }
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(format!(
                    "require_approval[{i}] may only contain alphanumeric characters and underscores: \"{name}\""
                ));
            }
        }

        for (i, name) in self.allow_always.iter().enumerate() {
            if name.is_empty() {
                return Err(format!("allow_always[{i}] must not be empty"));
            }
            if name.len() > MAX_TOOL_NAME_LEN {
                return Err(format!(
                    "allow_always[{i}] too long ({} chars, max {MAX_TOOL_NAME_LEN})",
                    name.len()
                ));
            }
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(format!(
                    "allow_always[{i}] may only contain alphanumeric characters and underscores: \"{name}\""
                ));
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers --

    fn valid_request() -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            agent_id: "agent-001".into(),
            tool_name: "shell_exec".into(),
            description: "Execute rm -rf /tmp/stale_cache".into(),
            action_summary: "rm -rf /tmp/stale_cache".into(),
            action_digest: approval_action_digest("shell_exec", b"rm -rf /tmp/stale_cache"),
            risk_level: RiskLevel::High,
            requested_at: Utc::now(),
            timeout_secs: 60,
        }
    }

    fn valid_policy() -> ApprovalPolicy {
        ApprovalPolicy::default()
    }

    // -----------------------------------------------------------------------
    // RiskLevel
    // -----------------------------------------------------------------------

    #[test]
    fn risk_level_emoji() {
        assert_eq!(RiskLevel::Low.emoji(), "\u{2139}\u{fe0f}");
        assert_eq!(RiskLevel::Medium.emoji(), "\u{26a0}\u{fe0f}");
        assert_eq!(RiskLevel::High.emoji(), "\u{1f6a8}");
        assert_eq!(RiskLevel::Critical.emoji(), "\u{2620}\u{fe0f}");
    }

    #[test]
    fn risk_level_serde_roundtrip() {
        for level in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: RiskLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, back);
        }
    }

    #[test]
    fn risk_level_rename_all() {
        let json = serde_json::to_string(&RiskLevel::Critical).unwrap();
        assert_eq!(json, "\"critical\"");
        let json = serde_json::to_string(&RiskLevel::Low).unwrap();
        assert_eq!(json, "\"low\"");
    }

    // -----------------------------------------------------------------------
    // ApprovalDecision
    // -----------------------------------------------------------------------

    #[test]
    fn decision_serde_roundtrip() {
        for decision in [
            ApprovalDecision::Approved,
            ApprovalDecision::ApprovedSession,
            ApprovalDecision::ApprovedAlways,
            ApprovalDecision::Denied,
            ApprovalDecision::DeniedSession,
            ApprovalDecision::DeniedAlways,
            ApprovalDecision::TimedOut,
        ] {
            let json = serde_json::to_string(&decision).unwrap();
            let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(decision, back);
        }
    }

    #[test]
    fn decision_rename_all() {
        let json = serde_json::to_string(&ApprovalDecision::TimedOut).unwrap();
        assert_eq!(json, "\"timed_out\"");
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — valid
    // -----------------------------------------------------------------------

    #[test]
    fn valid_request_passes() {
        assert!(valid_request().validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — tool_name
    // -----------------------------------------------------------------------

    #[test]
    fn request_empty_tool_name() {
        let mut req = valid_request();
        req.tool_name = String::new();
        let err = req.validate().unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn request_rejects_empty_or_control_character_agent_id() {
        let mut req = valid_request();
        req.agent_id = "  ".to_string();
        assert!(req.validate().unwrap_err().contains("agent_id"));

        req.agent_id = "agent\nforged".to_string();
        assert!(req.validate().unwrap_err().contains("agent_id"));
    }

    #[test]
    fn request_tool_name_too_long() {
        let mut req = valid_request();
        req.tool_name = "a".repeat(65);
        let err = req.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn request_tool_name_64_chars_ok() {
        let mut req = valid_request();
        req.tool_name = "a".repeat(64);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_tool_name_invalid_chars() {
        let mut req = valid_request();
        req.tool_name = "shell-exec".into();
        let err = req.validate().unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn request_tool_name_with_underscore_ok() {
        let mut req = valid_request();
        req.tool_name = "file_write".into();
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — description
    // -----------------------------------------------------------------------

    #[test]
    fn request_description_too_long() {
        let mut req = valid_request();
        req.description = "x".repeat(1025);
        let err = req.validate().unwrap_err();
        assert!(err.contains("description"), "{err}");
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn request_description_1024_ok() {
        let mut req = valid_request();
        req.description = "x".repeat(1024);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_description_empty_ok() {
        let mut req = valid_request();
        req.description = String::new();
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — action_summary
    // -----------------------------------------------------------------------

    #[test]
    fn request_action_summary_too_long() {
        let mut req = valid_request();
        req.action_summary = "x".repeat(513);
        let err = req.validate().unwrap_err();
        assert!(err.contains("action_summary"), "{err}");
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn request_action_summary_512_ok() {
        let mut req = valid_request();
        req.action_summary = "x".repeat(512);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_rejects_malformed_action_digest() {
        let mut req = valid_request();
        req.action_digest = "A".repeat(64);
        let err = req.validate().unwrap_err();
        assert!(err.contains("action_digest"), "{err}");
    }

    #[test]
    fn action_digest_uses_complete_input_and_tool_domain() {
        let common_prefix = "x".repeat(512);
        let first = format!("{common_prefix}A");
        let second = format!("{common_prefix}B");

        assert_ne!(
            approval_action_digest("shell_exec", first.as_bytes()),
            approval_action_digest("shell_exec", second.as_bytes())
        );
        assert_ne!(
            approval_action_digest("shell_exec", first.as_bytes()),
            approval_action_digest("file_write", first.as_bytes())
        );
    }

    // -----------------------------------------------------------------------
    // ApprovalRequest — timeout_secs
    // -----------------------------------------------------------------------

    #[test]
    fn request_timeout_too_small() {
        let mut req = valid_request();
        req.timeout_secs = 9;
        let err = req.validate().unwrap_err();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn request_timeout_too_large() {
        let mut req = valid_request();
        req.timeout_secs = 301;
        let err = req.validate().unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn request_timeout_min_boundary_ok() {
        let mut req = valid_request();
        req.timeout_secs = 10;
        assert!(req.validate().is_ok());
    }

    #[test]
    fn request_timeout_max_boundary_ok() {
        let mut req = valid_request();
        req.timeout_secs = 300;
        assert!(req.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalResponse — serde
    // -----------------------------------------------------------------------

    #[test]
    fn response_serde_roundtrip() {
        let resp = ApprovalResponse {
            request_id: Uuid::new_v4(),
            decision: ApprovalDecision::Approved,
            decided_at: Utc::now(),
            decided_by: Some("admin@example.com".into()),
            reason: Some("Reviewed by operator".into()),
            rule_id: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request_id, resp.request_id);
        assert_eq!(back.decision, ApprovalDecision::Approved);
        assert_eq!(back.decided_by, Some("admin@example.com".into()));
    }

    #[test]
    fn response_decided_by_none() {
        let resp = ApprovalResponse {
            request_id: Uuid::new_v4(),
            decision: ApprovalDecision::TimedOut,
            decided_at: Utc::now(),
            decided_by: None,
            reason: None,
            rule_id: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.decided_by, None);
        assert_eq!(back.decision, ApprovalDecision::TimedOut);
    }

    #[test]
    fn approval_reason_is_bounded_and_normalized() {
        assert_eq!(
            normalize_approval_reason(Some("  pas\nmaintenant  ")).unwrap(),
            Some("pas maintenant".to_string())
        );
        assert!(normalize_approval_reason(Some(&"x".repeat(MAX_APPROVAL_REASON_LEN + 1))).is_err());
        assert!(normalize_approval_reason(Some("bad\0reason")).is_err());
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — defaults
    // -----------------------------------------------------------------------

    #[test]
    fn policy_default_valid() {
        let policy = ApprovalPolicy::default();
        assert!(policy.validate().is_ok());
        assert_eq!(policy.require_approval, Vec::<String>::new());
        assert_eq!(policy.timeout_secs, 60);
        assert!(policy.auto_approve_autonomous);
        assert!(policy.auto_approve);
    }

    #[test]
    fn policy_serde_default() {
        // An empty JSON object should deserialize to defaults via #[serde(default)].
        let policy: ApprovalPolicy = serde_json::from_str("{}").unwrap();
        assert_eq!(policy.timeout_secs, 60);
        // serde default uses Default::default() which has empty require_approval
        assert_eq!(policy.require_approval, Vec::<String>::new());
    }

    #[test]
    fn policy_require_approval_bool_false() {
        // require_approval = false → empty list
        let policy: ApprovalPolicy =
            serde_json::from_str(r#"{"require_approval": false}"#).unwrap();
        assert!(policy.require_approval.is_empty());
    }

    #[test]
    fn policy_require_approval_bool_true() {
        // require_approval = true → ["shell_exec"]
        let policy: ApprovalPolicy = serde_json::from_str(r#"{"require_approval": true}"#).unwrap();
        assert_eq!(policy.require_approval, vec!["shell_exec"]);
    }

    #[test]
    fn policy_auto_approve_clears_list() {
        let mut policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".to_string()],
            allow_always: vec![],
            timeout_secs: 60,
            auto_approve_autonomous: false,
            auto_approve: true,
        };
        assert!(!policy.require_approval.is_empty());
        policy.apply_shorthands();
        assert!(policy.require_approval.is_empty());
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — timeout_secs
    // -----------------------------------------------------------------------

    #[test]
    fn policy_timeout_too_small() {
        let mut policy = valid_policy();
        policy.timeout_secs = 9;
        let err = policy.validate().unwrap_err();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn policy_timeout_too_large() {
        let mut policy = valid_policy();
        policy.timeout_secs = 301;
        let err = policy.validate().unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn policy_timeout_boundaries_ok() {
        let mut policy = valid_policy();
        policy.timeout_secs = 10;
        assert!(policy.validate().is_ok());
        policy.timeout_secs = 300;
        assert!(policy.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // ApprovalPolicy — require_approval tool names
    // -----------------------------------------------------------------------

    #[test]
    fn policy_empty_tool_name() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["shell_exec".into(), "".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("require_approval[1]"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn policy_tool_name_too_long() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["a".repeat(65)];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn policy_tool_name_invalid_chars() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["shell-exec".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn policy_tool_name_with_spaces_rejected() {
        let mut policy = valid_policy();
        policy.require_approval = vec!["shell exec".into()];
        let err = policy.validate().unwrap_err();
        assert!(err.contains("alphanumeric"), "{err}");
    }

    #[test]
    fn policy_allow_always_uses_the_same_tool_name_validation() {
        let mut policy = valid_policy();
        policy.allow_always = vec!["shell exec".to_string()];
        assert!(policy.validate().unwrap_err().contains("allow_always[0]"));
    }

    #[test]
    fn policy_multiple_valid_tools() {
        let mut policy = valid_policy();
        policy.require_approval = vec![
            "shell_exec".into(),
            "file_write".into(),
            "file_delete".into(),
        ];
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn policy_empty_require_approval_ok() {
        let mut policy = valid_policy();
        policy.require_approval = vec![];
        assert!(policy.validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // Full serde roundtrip — ApprovalRequest
    // -----------------------------------------------------------------------

    #[test]
    fn request_serde_roundtrip() {
        let req = valid_request();
        let json = serde_json::to_string_pretty(&req).unwrap();
        let back: ApprovalRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, req.id);
        assert_eq!(back.agent_id, req.agent_id);
        assert_eq!(back.tool_name, req.tool_name);
        assert_eq!(back.description, req.description);
        assert_eq!(back.action_summary, req.action_summary);
        assert_eq!(back.risk_level, req.risk_level);
        assert_eq!(back.timeout_secs, req.timeout_secs);
    }

    // -----------------------------------------------------------------------
    // Full serde roundtrip — ApprovalPolicy
    // -----------------------------------------------------------------------

    #[test]
    fn policy_serde_roundtrip() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".into(), "file_delete".into()],
            allow_always: vec![],
            timeout_secs: 120,
            auto_approve_autonomous: true,
            auto_approve: false,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: ApprovalPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.require_approval, policy.require_approval);
        assert_eq!(back.timeout_secs, 120);
        assert!(back.auto_approve_autonomous);
    }
}
