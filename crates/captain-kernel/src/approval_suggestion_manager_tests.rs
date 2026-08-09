use crate::approval::ApprovalManager;
use captain_runtime::audit::AuditLog;
use captain_types::approval::{
    approval_action_digest, ApprovalDecision, ApprovalPolicy, ApprovalRequest, RiskLevel,
};
use captain_types::approval_suggestions::ApprovalSuggestionPolicy;
use chrono::Utc;
use std::fs;
use std::sync::Arc;
use uuid::Uuid;

fn manager() -> Arc<ApprovalManager> {
    Arc::new(ApprovalManager::new(ApprovalPolicy {
        suggestions: ApprovalSuggestionPolicy {
            enabled: true,
            minimum_approvals: 3,
            observation_window_hours: 24,
            dismissal_cooldown_hours: 24,
        },
        ..ApprovalPolicy::default()
    }))
}

fn request(id: Uuid) -> ApprovalRequest {
    ApprovalRequest {
        id,
        agent_id: "captain".to_string(),
        tool_name: "web_fetch".to_string(),
        description: "Fetch a reviewed public endpoint".to_string(),
        action_summary: "https://example.invalid/redacted".to_string(),
        action_digest: approval_action_digest(
            "web_fetch",
            br#"{"url":"https://example.invalid/same"}"#,
        ),
        risk_level: RiskLevel::Medium,
        requested_at: Utc::now(),
        timeout_secs: 60,
    }
}

async fn approve_once(manager: &Arc<ApprovalManager>) -> captain_types::approval::ApprovalResponse {
    let id = Uuid::new_v4();
    let pending = request(id);
    let waiter = {
        let manager = Arc::clone(manager);
        tokio::spawn(async move { manager.request_approval(pending).await })
    };
    for _ in 0..100 {
        if manager
            .list_pending()
            .iter()
            .any(|request| request.id == id)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let response = manager
        .resolve(
            id,
            ApprovalDecision::Approved,
            Some("operator:test".to_string()),
        )
        .unwrap();
    assert_eq!(waiter.await.unwrap(), ApprovalDecision::Approved);
    response
}

#[tokio::test]
async fn repeated_exact_approvals_propose_only_an_explicit_revocable_rule() {
    let manager = manager();
    assert!(approve_once(&manager).await.suggestion.is_none());
    assert!(approve_once(&manager).await.suggestion.is_none());
    let suggestion = approve_once(&manager).await.suggestion.unwrap();

    assert_eq!(suggestion.observation_count, 3);
    assert_eq!(manager.list_suggestions(), vec![suggestion.clone()]);
    assert_eq!(manager.suggestion_status().pending_count, 1);

    let rule = manager
        .accept_suggestion(suggestion.id, Some("operator:test"))
        .unwrap();
    assert_eq!(rule.agent_id, suggestion.agent_id);
    assert_eq!(rule.tool_name, suggestion.tool_name);
    assert_eq!(rule.action_digest, suggestion.action_digest);
    assert!(manager.list_suggestions().is_empty());

    let outcome = manager.request_approval(request(Uuid::new_v4())).await;
    assert_eq!(outcome, ApprovalDecision::ApprovedAlways);
    assert_eq!(outcome.rule_id, Some(rule.id));

    assert!(manager
        .revoke_rule(rule.id, Some("operator:test"))
        .unwrap()
        .is_some());
    assert!(manager.list_rules().is_empty());
}

#[tokio::test]
async fn disabled_learning_neither_records_nor_changes_the_approval_response() {
    let manager = Arc::new(ApprovalManager::new(ApprovalPolicy::default()));
    for _ in 0..4 {
        assert!(approve_once(&manager).await.suggestion.is_none());
    }
    let status = manager.suggestion_status();
    assert!(!status.enabled);
    assert!(status.healthy);
    assert_eq!(status.pending_count, 0);
    assert!(manager.list_suggestions().is_empty());
}

#[tokio::test]
async fn dismissing_a_suggestion_adds_no_authority() {
    let manager = manager();
    approve_once(&manager).await;
    approve_once(&manager).await;
    let suggestion = approve_once(&manager).await.suggestion.unwrap();

    assert!(manager
        .dismiss_suggestion(suggestion.id, Some("operator:test"))
        .unwrap()
        .is_some());
    assert!(manager.list_suggestions().is_empty());
    assert!(manager.list_rules().is_empty());
}

#[tokio::test]
async fn disabling_learning_hides_existing_suggestions_and_authority_stays_empty() {
    let manager = manager();
    approve_once(&manager).await;
    approve_once(&manager).await;
    assert!(approve_once(&manager).await.suggestion.is_some());
    assert_eq!(manager.list_suggestions().len(), 1);

    manager.update_policy(ApprovalPolicy::default());
    assert!(manager.list_suggestions().is_empty());
    let status = manager.suggestion_status();
    assert!(!status.enabled);
    assert_eq!(status.pending_count, 0);
    assert!(manager.list_rules().is_empty());
}

#[tokio::test]
async fn corrupt_optional_store_opens_its_circuit_without_blocking_one_time_approval() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("approval-suggestions.json");
    fs::write(&path, b"not-json").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let manager = Arc::new(
        ApprovalManager::with_persistence(
            ApprovalPolicy {
                suggestions: ApprovalSuggestionPolicy {
                    enabled: true,
                    ..ApprovalSuggestionPolicy::default()
                },
                ..ApprovalPolicy::default()
            },
            directory.path(),
            Arc::new(AuditLog::new()),
        )
        .unwrap(),
    );
    let status = manager.suggestion_status();
    assert!(status.enabled);
    assert!(!status.healthy);
    assert_eq!(status.pending_count, 0);

    let response = approve_once(&manager).await;
    assert_eq!(response.decision, ApprovalDecision::Approved);
    assert!(response.suggestion.is_none());
    assert!(manager.list_rules().is_empty());
}

#[tokio::test]
async fn boot_reconciles_a_rule_committed_before_stale_suggestion_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let suggestions_path = directory.path().join("approval-suggestions.json");
    let policy = ApprovalPolicy {
        suggestions: ApprovalSuggestionPolicy {
            enabled: true,
            minimum_approvals: 3,
            observation_window_hours: 24,
            dismissal_cooldown_hours: 24,
        },
        ..ApprovalPolicy::default()
    };
    let manager = Arc::new(
        ApprovalManager::with_persistence(
            policy.clone(),
            directory.path(),
            Arc::new(AuditLog::new()),
        )
        .unwrap(),
    );
    approve_once(&manager).await;
    approve_once(&manager).await;
    let suggestion = approve_once(&manager).await.suggestion.unwrap();
    let stale_suggestions = fs::read(&suggestions_path).unwrap();

    let rule = manager
        .accept_suggestion(suggestion.id, Some("operator:test"))
        .unwrap();
    fs::write(&suggestions_path, stale_suggestions).unwrap();
    drop(manager);

    let manager =
        ApprovalManager::with_persistence(policy, directory.path(), Arc::new(AuditLog::new()))
            .unwrap();
    assert!(manager.suggestion_status().healthy);
    assert!(manager.list_suggestions().is_empty());
    assert!(!fs::read_to_string(&suggestions_path)
        .unwrap()
        .contains(&suggestion.id.to_string()));

    let outcome = manager.request_approval(request(Uuid::new_v4())).await;
    assert_eq!(outcome, ApprovalDecision::ApprovedAlways);
    assert_eq!(outcome.rule_id, Some(rule.id));
}
