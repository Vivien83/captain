//! Crash-safe Gmail synchronization and deterministic rule matching.

use std::sync::Arc;
use std::time::Duration;

use captain_extensions::gmail_api::{
    GmailApiClient, GmailApiError, GmailHistoryMessageAdded, GmailHistoryRequest,
};
use captain_memory::gmail_accounts::GmailAccountRecord;
use captain_memory::gmail_automation::{
    GmailAutomationError, GmailAutomationEventDecision, GmailAutomationRuleRecord,
    GmailAutomationStore, GmailSyncCheckpointRecord, GmailSyncMode, NewGmailAutomationMatch,
};
use captain_types::email::{
    GmailAccountAlias, GmailAccountStatus, GmailMessageSummary, GmailSearchRequest,
};
use futures::{stream, StreamExt, TryStreamExt};
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::kernel_email_credentials::{GmailCredentialManager, GmailRequiredAccess};
use super::CaptainKernel;

const INITIAL_DELAY: Duration = Duration::from_secs(15);
const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const MAX_RULES: usize = 1_000;
const MAX_PAGES_PER_ACCOUNT_TICK: usize = 2;
const HISTORY_PAGE_SIZE: u16 = 100;
const RECOVERY_PAGE_SIZE: u16 = 50;
const METADATA_CONCURRENCY: usize = 4;
const RECOVERY_OVERLAP_SECS: i64 = 5 * 60;

#[derive(Debug)]
struct SyncedMessage {
    history_id: String,
    summary: GmailMessageSummary,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct QueueStats {
    queued: usize,
    suppressed: usize,
}

#[derive(Debug)]
enum GmailSyncPageError {
    Api(GmailApiError),
    Store(GmailAutomationError),
}

impl From<GmailApiError> for GmailSyncPageError {
    fn from(error: GmailApiError) -> Self {
        Self::Api(error)
    }
}

impl From<GmailAutomationError> for GmailSyncPageError {
    fn from(error: GmailAutomationError) -> Self {
        Self::Store(error)
    }
}

pub(super) fn spawn_gmail_automation_worker(kernel: Arc<CaptainKernel>) {
    let automation = kernel.memory.gmail_automation().clone();
    match automation.reconcile_outbox_after_restart(now_unix_ms()) {
        Ok(summary) if summary.uncertain > 0 => warn!(
            uncertain = summary.uncertain,
            "Gmail deliveries interrupted by restart require operator review"
        ),
        Ok(_) => {}
        Err(error) => warn!(error = %error, "Gmail outbox restart reconciliation failed"),
    }

    tokio::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(error) = run_sync_tick(&kernel).await {
                warn!(error = %error, "Gmail automation synchronization tick failed");
            }
        }
    });
}

async fn run_sync_tick(kernel: &Arc<CaptainKernel>) -> Result<(), String> {
    let automation = kernel.memory.gmail_automation().clone();
    let rules = automation
        .list_rules(MAX_RULES)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|rule| rule.enabled)
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return Ok(());
    }

    let accounts = kernel
        .memory
        .gmail_accounts()
        .list()
        .map_err(|error| error.to_string())?;
    let credentials = GmailCredentialManager::new(
        kernel.config.home_dir.clone(),
        kernel.memory.gmail_accounts().clone(),
    );
    for account in accounts.into_iter().filter(syncable_account) {
        let account_rules = rules
            .iter()
            .filter(|rule| rule.account_alias == account.summary.alias)
            .cloned()
            .collect::<Vec<_>>();
        if account_rules.is_empty() {
            continue;
        }
        if let Err(error) = sync_account(&automation, &credentials, account, &account_rules).await {
            warn!(
                alias = %account_rules[0].account_alias,
                error = %error,
                "Gmail account synchronization paused until the next tick"
            );
        }
    }
    Ok(())
}

fn syncable_account(account: &GmailAccountRecord) -> bool {
    account.summary.enabled
        && account.summary.status == GmailAccountStatus::Ready
        && account.summary.access_profile.can_read()
        && account.summary.history_id.is_some()
}

async fn sync_account(
    automation: &GmailAutomationStore,
    credentials: &GmailCredentialManager,
    account: GmailAccountRecord,
    rules: &[GmailAutomationRuleRecord],
) -> Result<(), String> {
    let alias = account.summary.alias.clone();
    let context = credentials
        .authorize(Some(alias.clone()), GmailRequiredAccess::Read)
        .await?;
    let client = GmailApiClient::new(context.tokens.access_token()).map_err(|e| e.to_string())?;
    let start_history_id = context
        .record
        .summary
        .history_id
        .as_deref()
        .ok_or_else(|| format!("Gmail account '{alias}' has no synchronization cursor"))?;
    let mut checkpoint = automation
        .begin_sync(&alias, start_history_id, now_unix_ms())
        .map_err(|error| error.to_string())?;

    for _ in 0..MAX_PAGES_PER_ACCOUNT_TICK {
        let result = match checkpoint.mode {
            GmailSyncMode::Incremental => {
                sync_incremental_page(automation, &client, rules, &checkpoint).await
            }
            GmailSyncMode::Recovery => {
                sync_recovery_page(automation, &client, rules, &context.record, &checkpoint).await
            }
        };
        match result {
            Ok(Some(next)) => checkpoint = next,
            Ok(None) => {
                debug!(alias = %alias, "Gmail synchronization cursor advanced");
                return Ok(());
            }
            Err(GmailSyncPageError::Api(GmailApiError::HistoryExpired))
                if checkpoint.mode == GmailSyncMode::Incremental =>
            {
                let profile = match client.mailbox_profile().await {
                    Ok(profile) => profile,
                    Err(error) => {
                        return Err(record_api_failure(
                            automation,
                            credentials,
                            &context.record,
                            error,
                        )
                        .await)
                    }
                };
                if !profile
                    .email_address
                    .eq_ignore_ascii_case(&context.record.summary.email_address)
                {
                    return Err("Gmail profile identity changed during synchronization".to_string());
                }
                checkpoint = automation
                    .mark_sync_recovery(
                        &alias,
                        &checkpoint.start_history_id,
                        &profile.history_id,
                        now_unix_ms(),
                    )
                    .map_err(|error| error.to_string())?;
                info!(alias = %alias, "Gmail history cursor expired; resumable recovery started");
            }
            Err(GmailSyncPageError::Api(error)) => {
                return Err(
                    record_api_failure(automation, credentials, &context.record, error).await,
                )
            }
            Err(GmailSyncPageError::Store(error)) => {
                return Err(record_store_failure(automation, &alias, error));
            }
        }
    }
    Ok(())
}

async fn sync_incremental_page(
    automation: &GmailAutomationStore,
    client: &GmailApiClient,
    rules: &[GmailAutomationRuleRecord],
    checkpoint: &GmailSyncCheckpointRecord,
) -> Result<Option<GmailSyncCheckpointRecord>, GmailSyncPageError> {
    let page = client
        .list_history(&GmailHistoryRequest {
            start_history_id: checkpoint.start_history_id.clone(),
            page_token: checkpoint.page_token.clone(),
            max_results: HISTORY_PAGE_SIZE,
        })
        .await?;
    let messages = fetch_history_summaries(client, page.messages_added).await?;
    let stats = queue_matching_rules(
        automation,
        rules,
        &checkpoint.account_alias,
        &messages,
        now_unix_ms(),
    )?;
    debug!(
        alias = %checkpoint.account_alias,
        queued = stats.queued,
        suppressed = stats.suppressed,
        messages = messages.len(),
        "Gmail incremental page persisted"
    );
    Ok(automation.commit_sync_page(
        &checkpoint.account_alias,
        GmailSyncMode::Incremental,
        &checkpoint.start_history_id,
        checkpoint.page_token.as_deref(),
        page.next_page_token.as_deref(),
        &page.history_id,
        messages.len(),
        now_unix_ms(),
    )?)
}

async fn sync_recovery_page(
    automation: &GmailAutomationStore,
    client: &GmailApiClient,
    rules: &[GmailAutomationRuleRecord],
    account: &GmailAccountRecord,
    checkpoint: &GmailSyncCheckpointRecord,
) -> Result<Option<GmailSyncCheckpointRecord>, GmailSyncPageError> {
    let page = client
        .list_message_ids(&GmailSearchRequest {
            account_alias: Some(checkpoint.account_alias.clone()),
            query: recovery_query(account),
            label_ids: Vec::new(),
            max_results: RECOVERY_PAGE_SIZE,
            page_token: checkpoint.page_token.clone(),
            include_spam_trash: true,
        })
        .await?;
    let requested = page.message_ids.len();
    let messages = fetch_message_summaries(
        client,
        page.message_ids
            .into_iter()
            .map(|message_id| (checkpoint.target_history_id.clone(), message_id))
            .collect(),
    )
    .await?;
    let stats = queue_matching_rules(
        automation,
        rules,
        &checkpoint.account_alias,
        &messages,
        now_unix_ms(),
    )?;
    debug!(
        alias = %checkpoint.account_alias,
        queued = stats.queued,
        suppressed = stats.suppressed,
        messages = messages.len(),
        "Gmail recovery page persisted"
    );
    Ok(automation.commit_sync_page(
        &checkpoint.account_alias,
        GmailSyncMode::Recovery,
        &checkpoint.start_history_id,
        checkpoint.page_token.as_deref(),
        page.next_page_token.as_deref(),
        &checkpoint.target_history_id,
        requested,
        now_unix_ms(),
    )?)
}

async fn fetch_history_summaries(
    client: &GmailApiClient,
    events: Vec<GmailHistoryMessageAdded>,
) -> Result<Vec<SyncedMessage>, GmailApiError> {
    fetch_message_summaries(
        client,
        events
            .into_iter()
            .map(|event| (event.history_id, event.message_id))
            .collect(),
    )
    .await
}

async fn fetch_message_summaries(
    client: &GmailApiClient,
    messages: Vec<(String, String)>,
) -> Result<Vec<SyncedMessage>, GmailApiError> {
    let fetched = stream::iter(
        messages
            .into_iter()
            .map(|(history_id, message_id)| async move {
                match client.message_summary(&message_id).await {
                    Ok(summary) => Ok(Some(SyncedMessage {
                        history_id,
                        summary,
                    })),
                    Err(GmailApiError::Rejected { status: 404, .. }) => Ok(None),
                    Err(error) => Err(error),
                }
            }),
    )
    .buffered(METADATA_CONCURRENCY)
    .try_collect::<Vec<_>>()
    .await?;
    Ok(fetched.into_iter().flatten().collect())
}

fn queue_matching_rules(
    automation: &GmailAutomationStore,
    rules: &[GmailAutomationRuleRecord],
    alias: &GmailAccountAlias,
    messages: &[SyncedMessage],
    now: i64,
) -> Result<QueueStats, GmailAutomationError> {
    let mut stats = QueueStats::default();
    for message in messages {
        let metadata_json = serde_json::to_string(&message.summary).map_err(|error| {
            GmailAutomationError::InvalidInput(format!("could not encode Gmail metadata: {error}"))
        })?;
        for rule in rules {
            let Some(outcome) = queue_rule_if_current(
                automation,
                rule.clone(),
                alias,
                message,
                &metadata_json,
                now,
            )?
            else {
                continue;
            };
            match outcome.event.decision {
                GmailAutomationEventDecision::Queued => stats.queued += 1,
                GmailAutomationEventDecision::SuppressedRateLimit => stats.suppressed += 1,
            }
        }
    }
    Ok(stats)
}

fn queue_rule_if_current(
    automation: &GmailAutomationStore,
    mut rule: GmailAutomationRuleRecord,
    alias: &GmailAccountAlias,
    message: &SyncedMessage,
    metadata_json: &str,
    now: i64,
) -> Result<
    Option<captain_memory::gmail_automation::GmailAutomationQueueOutcome>,
    GmailAutomationError,
> {
    for _ in 0..3 {
        if !rule.enabled
            || rule.account_alias != *alias
            || !rule.condition.matches(&message.summary)
        {
            return Ok(None);
        }
        let input = NewGmailAutomationMatch {
            idempotency_key: stable_event_key(&rule.id, alias, &message.summary.id),
            rule_id: rule.id.clone(),
            expected_rule_version: rule.state_version,
            account_alias: alias.clone(),
            message_id: message.summary.id.clone(),
            history_id: message.history_id.clone(),
            metadata_json: metadata_json.to_string(),
            occurred_at_unix_ms: now,
        };
        match automation.enqueue_match(&input) {
            Ok(outcome) => return Ok(Some(outcome)),
            Err(GmailAutomationError::Conflict(reason)) => {
                let Some(latest) = automation.get_rule(&rule.id)? else {
                    return Ok(None);
                };
                if latest.state_version == rule.state_version {
                    return Err(GmailAutomationError::Conflict(reason));
                }
                rule = latest;
            }
            Err(error) => return Err(error),
        }
    }
    Err(GmailAutomationError::Conflict(format!(
        "rule '{}' changed repeatedly while matching a Gmail message",
        rule.id
    )))
}

fn stable_event_key(rule_id: &str, alias: &GmailAccountAlias, message_id: &str) -> String {
    let seed = format!("captain:gmail-automation:v1:{rule_id}:{alias}:{message_id}");
    format!(
        "gmail:v1:{}",
        Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes())
    )
}

fn recovery_query(account: &GmailAccountRecord) -> String {
    let baseline = account
        .summary
        .last_sync_at
        .unwrap_or(account.summary.created_at)
        .timestamp()
        .saturating_sub(RECOVERY_OVERLAP_SECS)
        .max(0);
    format!("after:{baseline}")
}

async fn record_api_failure(
    automation: &GmailAutomationStore,
    credentials: &GmailCredentialManager,
    account: &GmailAccountRecord,
    error: GmailApiError,
) -> String {
    credentials.record_api_failure(account, &error).await;
    if let Err(store_error) =
        automation.record_sync_failure(&account.summary.alias, error.code(), now_unix_ms())
    {
        warn!(
            alias = %account.summary.alias,
            error = %store_error,
            "Gmail synchronization failure bookkeeping failed"
        );
    }
    error.to_string()
}

fn record_store_failure(
    automation: &GmailAutomationStore,
    alias: &GmailAccountAlias,
    error: GmailAutomationError,
) -> String {
    let code = match &error {
        GmailAutomationError::Conflict(_) => "gmail_sync_conflict",
        GmailAutomationError::Sqlite(_) => "gmail_sync_store",
        GmailAutomationError::InvalidInput(_)
        | GmailAutomationError::NotFound(_)
        | GmailAutomationError::CorruptData(_) => "gmail_sync_state",
    };
    if let Err(bookkeeping_error) = automation.record_sync_failure(alias, code, now_unix_ms()) {
        warn!(
            alias = %alias,
            error = %bookkeeping_error,
            "Gmail local synchronization failure bookkeeping failed"
        );
    }
    format!("durable Gmail synchronization state failed: {error}")
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis().max(0)
}

#[cfg(test)]
#[path = "kernel_gmail_automation_tests.rs"]
mod tests;
