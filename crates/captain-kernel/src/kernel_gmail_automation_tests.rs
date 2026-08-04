use captain_memory::gmail_accounts::NewGmailAccount;
use captain_memory::gmail_automation::{
    GmailAutomationAction, GmailAutomationCondition, NewGmailAutomationRule,
};
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentId;
use captain_types::email::{GmailAccessProfile, GmailAccountAlias, GmailMessageSummary};
use chrono::{TimeZone, Utc};

use super::*;

fn message(subject: &str) -> SyncedMessage {
    SyncedMessage {
        history_id: "101".to_string(),
        summary: GmailMessageSummary {
            id: "message_1".to_string(),
            thread_id: "thread_1".to_string(),
            label_ids: vec!["INBOX".to_string()],
            snippet: "Invoice attached".to_string(),
            from: Some("billing@example.com".to_string()),
            to: Some("owner@example.com".to_string()),
            cc: None,
            subject: Some(subject.to_string()),
            received_at: None,
            size_estimate: 42,
        },
    }
}

fn memory_with_rule() -> (MemorySubstrate, GmailAutomationRuleRecord) {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let alias = GmailAccountAlias::parse("work").unwrap();
    memory
        .gmail_accounts()
        .upsert(NewGmailAccount {
            alias: alias.clone(),
            email_address: "owner@example.com".to_string(),
            access_profile: GmailAccessProfile::Assistant,
            granted_scopes: GmailAccessProfile::Assistant
                .required_scopes()
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            token_vault_key: "CAPTAIN_GMAIL_TOKEN_TEST".to_string(),
            client_vault_key: "CAPTAIN_GMAIL_CLIENT_TEST".to_string(),
            history_id: Some("100".to_string()),
            make_default: true,
        })
        .unwrap();
    let rule = memory
        .gmail_automation()
        .create_rule(NewGmailAutomationRule {
            id: "invoice".to_string(),
            account_alias: alias,
            name: "Invoice review".to_string(),
            condition: GmailAutomationCondition {
                from_contains: Some("billing@example.com".to_string()),
                recipient_contains: None,
                subject_contains: Some("invoice".to_string()),
                all_label_ids: vec!["INBOX".to_string()],
                any_label_ids: Vec::new(),
            },
            action: GmailAutomationAction {
                target_agent_id: AgentId::from_string("captain"),
                instruction: "Review the invoice".to_string(),
                include_body: false,
                max_body_bytes: 16 * 1024,
                max_delivery_attempts: 3,
            },
            enabled: true,
            max_fires_per_hour: 10,
            created_at_unix_ms: 1_000,
        })
        .unwrap();
    (memory, rule)
}

#[test]
fn matching_is_deterministic_and_replay_safe() {
    let (memory, rule) = memory_with_rule();
    let alias = rule.account_alias.clone();
    let stats = queue_matching_rules(
        memory.gmail_automation(),
        std::slice::from_ref(&rule),
        &alias,
        &[message("July invoice")],
        10_000,
    )
    .unwrap();
    assert_eq!(stats.queued, 1);

    let replay = queue_matching_rules(
        memory.gmail_automation(),
        &[rule],
        &alias,
        &[SyncedMessage {
            history_id: "999".to_string(),
            ..message("July invoice")
        }],
        11_000,
    )
    .unwrap();
    assert_eq!(replay.queued, 1);
}

#[test]
fn non_matching_metadata_never_creates_an_agent_delivery() {
    let (memory, rule) = memory_with_rule();
    let stats = queue_matching_rules(
        memory.gmail_automation(),
        std::slice::from_ref(&rule),
        &rule.account_alias,
        &[message("Team lunch")],
        10_000,
    )
    .unwrap();
    assert_eq!(stats, QueueStats::default());
}

#[test]
fn recovery_uses_epoch_seconds_with_a_small_idempotent_overlap() {
    let (memory, _) = memory_with_rule();
    let alias = GmailAccountAlias::parse("work").unwrap();
    let mut record = memory.gmail_accounts().get(&alias).unwrap().unwrap();
    record.summary.created_at = Utc.timestamp_opt(10_000, 0).unwrap();
    record.summary.last_sync_at = Some(Utc.timestamp_opt(20_000, 0).unwrap());
    assert_eq!(recovery_query(&record), "after:19700");
}

#[test]
fn event_keys_are_stable_but_distinguish_rule_account_and_message() {
    let work = GmailAccountAlias::parse("work").unwrap();
    let personal = GmailAccountAlias::parse("personal").unwrap();
    let first = stable_event_key("invoice", &work, "message_1");
    assert_eq!(first, stable_event_key("invoice", &work, "message_1"));
    assert_ne!(first, stable_event_key("invoice", &work, "message_2"));
    assert_ne!(first, stable_event_key("invoice", &personal, "message_1"));
    assert_ne!(first, stable_event_key("receipt", &work, "message_1"));
}

#[test]
fn local_sync_failures_keep_api_credentials_out_of_the_error_path() {
    let (memory, _) = memory_with_rule();
    let alias = GmailAccountAlias::parse("work").unwrap();
    let message = record_store_failure(
        memory.gmail_automation(),
        &alias,
        GmailAutomationError::Conflict("cursor changed".to_string()),
    );

    assert!(message.contains("durable Gmail synchronization state failed"));
    assert_eq!(
        memory
            .gmail_accounts()
            .get(&alias)
            .unwrap()
            .unwrap()
            .summary
            .last_error_code
            .as_deref(),
        Some("gmail_sync_conflict")
    );
}
