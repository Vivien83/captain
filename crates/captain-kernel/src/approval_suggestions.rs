use captain_types::approval::{
    is_valid_approval_action_digest, ApprovalDecision, ApprovalRequest, RiskLevel,
};
use captain_types::approval_suggestions::{
    risk_is_suggestion_eligible, ApprovalSuggestion, ApprovalSuggestionPolicy,
    MAX_SUGGESTION_APPROVALS,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use uuid::Uuid;

const SUGGESTION_FILE_VERSION: u32 = 1;
const MAX_SUGGESTION_CANDIDATES: usize = 256;
const MAX_SUGGESTION_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalSuggestionFile {
    schema_version: u32,
    candidates: Vec<ApprovalSuggestionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalSuggestionCandidate {
    id: Uuid,
    proposed_rule_id: Uuid,
    agent_id: String,
    tool_name: String,
    action_digest: String,
    risk_level: RiskLevel,
    approvals: Vec<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_since: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dismissed_until: Option<DateTime<Utc>>,
}

impl ApprovalSuggestionCandidate {
    fn matches(&self, request: &ApprovalRequest) -> bool {
        self.agent_id == request.agent_id
            && self.tool_name == request.tool_name
            && self.action_digest == request.action_digest
    }

    fn public(
        &self,
        policy: &ApprovalSuggestionPolicy,
        now: DateTime<Utc>,
    ) -> Option<ApprovalSuggestion> {
        if !policy.enabled
            || self
                .dismissed_until
                .is_some_and(|dismissed_until| dismissed_until > now)
        {
            return None;
        }
        let created_at = self.pending_since?;
        if created_at > now {
            return None;
        }
        let window_start = now - duration_hours(policy.observation_window_hours).ok()?;
        let recent = self
            .approvals
            .iter()
            .copied()
            .filter(|observed_at| *observed_at >= window_start && *observed_at <= now)
            .collect::<Vec<_>>();
        if recent.len() < policy.minimum_approvals as usize {
            return None;
        }
        let first_observed_at = *recent.first()?;
        let last_observed_at = *recent.last()?;
        Some(ApprovalSuggestion {
            id: self.id,
            agent_id: self.agent_id.clone(),
            tool_name: self.tool_name.clone(),
            action_digest: self.action_digest.clone(),
            risk_level: self.risk_level,
            observation_count: recent.len().min(u16::MAX as usize) as u16,
            first_observed_at,
            last_observed_at,
            created_at,
        })
    }

    fn prune(
        &mut self,
        policy: &ApprovalSuggestionPolicy,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let window_start = now - duration_hours(policy.observation_window_hours)?;
        self.approvals
            .retain(|observed_at| *observed_at >= window_start && *observed_at <= now);
        self.approvals.sort_unstable();
        if self.approvals.len() < policy.minimum_approvals as usize {
            self.pending_since = None;
        }
        if self
            .dismissed_until
            .is_some_and(|dismissed_until| dismissed_until <= now)
        {
            self.dismissed_until = None;
        }
        Ok(())
    }

    fn should_retain(&self) -> bool {
        self.pending_since.is_some() || self.dismissed_until.is_some() || !self.approvals.is_empty()
    }
}

pub(super) struct PendingApprovalSuggestion {
    pub(super) suggestion: ApprovalSuggestion,
    pub(super) proposed_rule_id: Uuid,
}

#[derive(Debug)]
pub(super) struct ApprovalSuggestionStore {
    path: Option<PathBuf>,
    candidates: RwLock<Vec<ApprovalSuggestionCandidate>>,
}

impl ApprovalSuggestionStore {
    pub(super) fn in_memory() -> Self {
        Self {
            path: None,
            candidates: RwLock::new(Vec::new()),
        }
    }

    pub(super) fn load(path: PathBuf) -> Result<Self, String> {
        let candidates = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(format!(
                        "approval suggestion state {} must be a regular file, not a symlink",
                        path.display()
                    ));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o077 != 0 {
                        return Err(format!(
                            "approval suggestion state {} must be owner-only (0600)",
                            path.display()
                        ));
                    }
                }
                if metadata.len() > MAX_SUGGESTION_FILE_BYTES {
                    return Err(format!(
                        "approval suggestion file {} is too large ({} bytes, max {MAX_SUGGESTION_FILE_BYTES})",
                        path.display(),
                        metadata.len()
                    ));
                }
                let raw = fs::read_to_string(&path).map_err(|error| {
                    format!("read approval suggestions {}: {error}", path.display())
                })?;
                let file: ApprovalSuggestionFile = serde_json::from_str(&raw).map_err(|error| {
                    format!("parse approval suggestions {}: {error}", path.display())
                })?;
                if file.schema_version != SUGGESTION_FILE_VERSION {
                    return Err(format!(
                        "unsupported approval suggestion schema {} in {} (expected {SUGGESTION_FILE_VERSION})",
                        file.schema_version,
                        path.display()
                    ));
                }
                validate_candidates(&file.candidates)?;
                file.candidates
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(format!(
                    "read approval suggestion metadata {}: {error}",
                    path.display()
                ));
            }
        };
        Ok(Self {
            path: Some(path),
            candidates: RwLock::new(candidates),
        })
    }

    pub(super) fn observe(
        &self,
        policy: &ApprovalSuggestionPolicy,
        request: &ApprovalRequest,
        decision: ApprovalDecision,
        now: DateTime<Utc>,
    ) -> Result<Option<ApprovalSuggestion>, String> {
        if !policy.enabled {
            return Ok(None);
        }
        policy.validate()?;
        request.validate()?;

        let mut guard = self
            .candidates
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let mut next = guard.clone();
        for candidate in &mut next {
            candidate.prune(policy, now)?;
        }
        next.retain(ApprovalSuggestionCandidate::should_retain);

        let eligible = decision == ApprovalDecision::Approved
            && risk_is_suggestion_eligible(request.risk_level);
        let mut newly_pending = false;
        if !eligible {
            next.retain(|candidate| !candidate.matches(request));
        } else if let Some(candidate) = next.iter_mut().find(|candidate| candidate.matches(request))
        {
            if candidate.pending_since.is_none() && candidate.dismissed_until.is_none() {
                candidate.approvals.push(now);
                candidate.approvals.sort_unstable();
                if candidate.approvals.len() > MAX_SUGGESTION_APPROVALS as usize {
                    let excess = candidate.approvals.len() - MAX_SUGGESTION_APPROVALS as usize;
                    candidate.approvals.drain(..excess);
                }
                if candidate.approvals.len() >= policy.minimum_approvals as usize {
                    candidate.pending_since = candidate.approvals.last().copied();
                    newly_pending = true;
                }
            }
        } else {
            if next.len() >= MAX_SUGGESTION_CANDIDATES {
                let evictable = next
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate.pending_since.is_none() && candidate.dismissed_until.is_none()
                    })
                    .min_by_key(|(_, candidate)| candidate.approvals.last().copied())
                    .map(|(index, _)| index);
                if let Some(index) = evictable {
                    next.remove(index);
                }
            }
            if next.len() < MAX_SUGGESTION_CANDIDATES {
                next.push(ApprovalSuggestionCandidate {
                    id: Uuid::new_v4(),
                    proposed_rule_id: Uuid::new_v4(),
                    agent_id: request.agent_id.clone(),
                    tool_name: request.tool_name.clone(),
                    action_digest: request.action_digest.clone(),
                    risk_level: request.risk_level,
                    approvals: vec![now],
                    pending_since: None,
                    dismissed_until: None,
                });
            }
        }

        validate_candidates(&next)?;
        let created = newly_pending
            .then(|| {
                next.iter()
                    .find(|candidate| candidate.matches(request))
                    .and_then(|candidate| candidate.public(policy, now))
            })
            .flatten();
        if next != *guard {
            self.persist(&next)?;
            *guard = next;
        }
        Ok(created)
    }

    pub(super) fn list_pending(
        &self,
        policy: &ApprovalSuggestionPolicy,
        now: DateTime<Utc>,
    ) -> Vec<ApprovalSuggestion> {
        let guard = self
            .candidates
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let mut suggestions = guard
            .iter()
            .filter_map(|candidate| candidate.public(policy, now))
            .collect::<Vec<_>>();
        suggestions.sort_by_key(|suggestion| std::cmp::Reverse(suggestion.created_at));
        suggestions
    }

    pub(super) fn pending_for_accept(
        &self,
        id: Uuid,
        policy: &ApprovalSuggestionPolicy,
        now: DateTime<Utc>,
    ) -> Option<PendingApprovalSuggestion> {
        self.candidates
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|candidate| candidate.id == id)
            .and_then(|candidate| {
                candidate
                    .public(policy, now)
                    .map(|suggestion| PendingApprovalSuggestion {
                        suggestion,
                        proposed_rule_id: candidate.proposed_rule_id,
                    })
            })
    }

    pub(super) fn dismiss(
        &self,
        id: Uuid,
        policy: &ApprovalSuggestionPolicy,
        now: DateTime<Utc>,
    ) -> Result<Option<ApprovalSuggestion>, String> {
        policy.validate()?;
        let mut guard = self
            .candidates
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let Some(existing) = guard
            .iter()
            .find(|candidate| candidate.id == id)
            .and_then(|candidate| candidate.public(policy, now))
        else {
            return Ok(None);
        };
        let mut next = guard.clone();
        let candidate = next
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .expect("candidate existed under the same write lock");
        candidate.approvals.clear();
        candidate.pending_since = None;
        candidate.dismissed_until = Some(now + duration_hours(policy.dismissal_cooldown_hours)?);
        self.persist(&next)?;
        *guard = next;
        Ok(Some(existing))
    }

    pub(super) fn remove(&self, id: Uuid) -> Result<bool, String> {
        let mut guard = self
            .candidates
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let mut next = guard.clone();
        next.retain(|candidate| candidate.id != id);
        if next.len() == guard.len() {
            return Ok(false);
        }
        self.persist(&next)?;
        *guard = next;
        Ok(true)
    }

    pub(super) fn remove_covered_bindings(
        &self,
        bindings: &std::collections::HashSet<(String, String, String)>,
    ) -> Result<usize, String> {
        let mut guard = self
            .candidates
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let mut next = guard.clone();
        next.retain(|candidate| {
            !bindings.contains(&(
                candidate.agent_id.clone(),
                candidate.tool_name.clone(),
                candidate.action_digest.clone(),
            ))
        });
        let removed = guard.len().saturating_sub(next.len());
        if removed == 0 {
            return Ok(0);
        }
        self.persist(&next)?;
        *guard = next;
        Ok(removed)
    }

    fn persist(&self, candidates: &[ApprovalSuggestionCandidate]) -> Result<(), String> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let mut payload = serde_json::to_vec_pretty(&ApprovalSuggestionFile {
            schema_version: SUGGESTION_FILE_VERSION,
            candidates: candidates.to_vec(),
        })
        .map_err(|error| format!("serialize approval suggestions: {error}"))?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_SUGGESTION_FILE_BYTES {
            return Err(format!(
                "serialized approval suggestions exceed {MAX_SUGGESTION_FILE_BYTES} bytes"
            ));
        }
        captain_types::durable_fs::atomic_write(path, &payload)
            .map_err(|error| format!("persist approval suggestions {}: {error}", path.display()))
    }
}

fn duration_hours(hours: u64) -> Result<Duration, String> {
    let hours = i64::try_from(hours)
        .map_err(|_| "approval suggestion duration exceeds supported range".to_string())?;
    Ok(Duration::hours(hours))
}

fn validate_candidates(candidates: &[ApprovalSuggestionCandidate]) -> Result<(), String> {
    if candidates.len() > MAX_SUGGESTION_CANDIDATES {
        return Err(format!(
            "approval suggestion file contains {} candidates (max {MAX_SUGGESTION_CANDIDATES})",
            candidates.len()
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut proposed_rule_ids = std::collections::HashSet::new();
    let mut bindings = std::collections::HashSet::new();
    for candidate in candidates {
        if !ids.insert(candidate.id) {
            return Err(format!("duplicate approval suggestion id {}", candidate.id));
        }
        if !proposed_rule_ids.insert(candidate.proposed_rule_id) {
            return Err(format!(
                "duplicate proposed approval rule id {}",
                candidate.proposed_rule_id
            ));
        }
        let binding = (
            candidate.agent_id.as_str(),
            candidate.tool_name.as_str(),
            candidate.action_digest.as_str(),
        );
        if !bindings.insert(binding) {
            return Err("duplicate approval suggestion binding".to_string());
        }
        validate_candidate(candidate)?;
    }
    Ok(())
}

fn validate_candidate(candidate: &ApprovalSuggestionCandidate) -> Result<(), String> {
    if candidate.agent_id.trim().is_empty()
        || candidate.agent_id.chars().count() > 128
        || candidate.agent_id.chars().any(char::is_control)
    {
        return Err("approval suggestion agent_id must contain 1..=128 characters".to_string());
    }
    if candidate.tool_name.is_empty()
        || candidate.tool_name.len() > 64
        || !candidate
            .tool_name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("approval suggestion tool_name is invalid".to_string());
    }
    if !is_valid_approval_action_digest(&candidate.action_digest) {
        return Err("approval suggestion action_digest is invalid".to_string());
    }
    if !risk_is_suggestion_eligible(candidate.risk_level) {
        return Err("approval suggestion risk is not eligible".to_string());
    }
    if candidate.approvals.len() > MAX_SUGGESTION_APPROVALS as usize {
        return Err(format!(
            "approval suggestion contains too many observations (max {MAX_SUGGESTION_APPROVALS})"
        ));
    }
    if candidate.pending_since.is_some() && candidate.approvals.is_empty() {
        return Err("pending approval suggestion has no observations".to_string());
    }
    if candidate.pending_since.is_some() && candidate.dismissed_until.is_some() {
        return Err("approval suggestion cannot be pending and dismissed".to_string());
    }
    if candidate
        .approvals
        .windows(2)
        .any(|window| window[0] > window[1])
    {
        return Err("approval suggestion observations are not chronological".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::approval::{approval_action_digest, ApprovalRequest};

    fn policy() -> ApprovalSuggestionPolicy {
        ApprovalSuggestionPolicy {
            enabled: true,
            minimum_approvals: 3,
            observation_window_hours: 24,
            dismissal_cooldown_hours: 24,
        }
    }

    fn request(digest_seed: &str, risk_level: RiskLevel) -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            agent_id: "captain".to_string(),
            tool_name: "web_fetch".to_string(),
            description: "Fetch a public source".to_string(),
            action_summary: "redacted display only".to_string(),
            action_digest: approval_action_digest("web_fetch", digest_seed.as_bytes()),
            risk_level,
            requested_at: Utc::now(),
            timeout_secs: 60,
        }
    }

    #[test]
    fn disabled_policy_records_nothing() {
        let store = ApprovalSuggestionStore::in_memory();
        let mut disabled = policy();
        disabled.enabled = false;
        assert!(store
            .observe(
                &disabled,
                &request("same", RiskLevel::Medium),
                ApprovalDecision::Approved,
                Utc::now(),
            )
            .unwrap()
            .is_none());
        assert!(store.list_pending(&disabled, Utc::now()).is_empty());
    }

    #[test]
    fn three_exact_one_time_approvals_create_one_pending_suggestion() {
        let store = ApprovalSuggestionStore::in_memory();
        let request = request("same", RiskLevel::Medium);
        let now = Utc::now();
        assert!(store
            .observe(&policy(), &request, ApprovalDecision::Approved, now)
            .unwrap()
            .is_none());
        assert!(store
            .observe(
                &policy(),
                &request,
                ApprovalDecision::Approved,
                now + Duration::minutes(1),
            )
            .unwrap()
            .is_none());
        let suggestion = store
            .observe(
                &policy(),
                &request,
                ApprovalDecision::Approved,
                now + Duration::minutes(2),
            )
            .unwrap()
            .unwrap();
        assert_eq!(suggestion.observation_count, 3);
        assert_eq!(
            store.list_pending(&policy(), now + Duration::minutes(2)),
            vec![suggestion]
        );
    }

    #[test]
    fn bindings_do_not_merge_and_high_risk_never_learns() {
        let store = ApprovalSuggestionStore::in_memory();
        let now = Utc::now();
        for index in 0..3 {
            assert!(store
                .observe(
                    &policy(),
                    &request(&format!("different-{index}"), RiskLevel::Medium),
                    ApprovalDecision::Approved,
                    now + Duration::minutes(index),
                )
                .unwrap()
                .is_none());
        }
        for index in 0..3 {
            assert!(store
                .observe(
                    &policy(),
                    &request("critical", RiskLevel::Critical),
                    ApprovalDecision::Approved,
                    now + Duration::minutes(index),
                )
                .unwrap()
                .is_none());
        }
        assert!(store
            .list_pending(&policy(), now + Duration::minutes(3))
            .is_empty());
    }

    #[test]
    fn denial_resets_observations_and_dismissal_requires_a_fresh_window() {
        let store = ApprovalSuggestionStore::in_memory();
        let request = request("same", RiskLevel::Low);
        let now = Utc::now();
        for index in 0..2 {
            store
                .observe(
                    &policy(),
                    &request,
                    ApprovalDecision::Approved,
                    now + Duration::minutes(index),
                )
                .unwrap();
        }
        store
            .observe(
                &policy(),
                &request,
                ApprovalDecision::Denied,
                now + Duration::minutes(3),
            )
            .unwrap();
        assert!(store
            .list_pending(&policy(), now + Duration::minutes(3))
            .is_empty());

        let later = now + Duration::minutes(4);
        for index in 0..3 {
            store
                .observe(
                    &policy(),
                    &request,
                    ApprovalDecision::Approved,
                    later + Duration::minutes(index),
                )
                .unwrap();
        }
        let proposed_at = later + Duration::minutes(2);
        let suggestion = store.list_pending(&policy(), proposed_at).pop().unwrap();
        let dismissed_at = later + Duration::minutes(3);
        store
            .dismiss(suggestion.id, &policy(), dismissed_at)
            .unwrap();
        assert!(store.list_pending(&policy(), dismissed_at).is_empty());
        for index in 0..3 {
            assert!(store
                .observe(
                    &policy(),
                    &request,
                    ApprovalDecision::Approved,
                    dismissed_at + Duration::minutes(index + 1),
                )
                .unwrap()
                .is_none());
        }

        let after_cooldown = dismissed_at + Duration::hours(25);
        for index in 0..3 {
            store
                .observe(
                    &policy(),
                    &request,
                    ApprovalDecision::Approved,
                    after_cooldown + Duration::minutes(index),
                )
                .unwrap();
        }
        assert_eq!(
            store
                .list_pending(&policy(), after_cooldown + Duration::minutes(2))
                .len(),
            1
        );
    }

    #[test]
    fn persistence_survives_restart_without_raw_action_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("approval-suggestions.json");
        let request = request("same", RiskLevel::Medium);
        let now = Utc::now();
        {
            let store = ApprovalSuggestionStore::load(path.clone()).unwrap();
            for index in 0..3 {
                store
                    .observe(
                        &policy(),
                        &request,
                        ApprovalDecision::Approved,
                        now + Duration::minutes(index),
                    )
                    .unwrap();
            }
        }
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("action_summary"));
        assert!(!raw.contains("description"));
        assert!(!raw.contains("redacted display only"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            ApprovalSuggestionStore::load(path)
                .unwrap()
                .list_pending(&policy(), now + Duration::minutes(3))
                .len(),
            1
        );
    }

    #[test]
    fn expired_pending_suggestion_is_retired_without_opening_an_error() {
        let store = ApprovalSuggestionStore::in_memory();
        let request = request("expires", RiskLevel::Medium);
        let now = Utc::now();
        for index in 0..3 {
            store
                .observe(
                    &policy(),
                    &request,
                    ApprovalDecision::Approved,
                    now + Duration::minutes(index),
                )
                .unwrap();
        }
        assert_eq!(
            store
                .list_pending(&policy(), now + Duration::minutes(2))
                .len(),
            1
        );

        let expired = now + Duration::hours(25);
        assert!(store
            .observe(&policy(), &request, ApprovalDecision::Approved, expired)
            .unwrap()
            .is_none());
        assert!(store.list_pending(&policy(), expired).is_empty());
    }

    #[test]
    fn capacity_evicts_oldest_observation_instead_of_failing_learning() {
        let store = ApprovalSuggestionStore::in_memory();
        let now = Utc::now();
        for index in 0..MAX_SUGGESTION_CANDIDATES {
            store
                .observe(
                    &policy(),
                    &request(&format!("candidate-{index}"), RiskLevel::Low),
                    ApprovalDecision::Approved,
                    now + Duration::seconds(index as i64),
                )
                .unwrap();
        }
        let newest = request("newest", RiskLevel::Low);
        store
            .observe(
                &policy(),
                &newest,
                ApprovalDecision::Approved,
                now + Duration::minutes(10),
            )
            .unwrap();

        let candidates = store
            .candidates
            .read()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(candidates.len(), MAX_SUGGESTION_CANDIDATES);
        assert!(candidates
            .iter()
            .any(|candidate| candidate.matches(&newest)));
        assert!(!candidates
            .iter()
            .any(|candidate| candidate.matches(&request("candidate-0", RiskLevel::Low))));
    }

    #[cfg(unix)]
    #[test]
    fn suggestion_state_symlink_is_rejected_before_reading() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.json");
        let path = directory.path().join("approval-suggestions.json");
        fs::write(&target, b"not suggestion state").unwrap();
        symlink(&target, &path).unwrap();

        assert!(ApprovalSuggestionStore::load(path)
            .unwrap_err()
            .contains("must be a regular file"));
    }
}
