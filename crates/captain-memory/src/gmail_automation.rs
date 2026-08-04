//! Durable deterministic Gmail rules and crash-safe delivery outbox.

use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

use captain_types::agent::{AgentId, SessionId};
use captain_types::email::{GmailAccessProfile, GmailAccountAlias, GmailMessageSummary};
use rusqlite::{
    params, types::Type, Connection, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_RULES: usize = 1_000;
const MAX_OUTBOX_RESULTS: usize = 1_000;
const MAX_RULE_NAME_BYTES: usize = 160;
const MAX_MATCH_BYTES: usize = 512;
const MAX_INSTRUCTION_BYTES: usize = 16 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_PAYLOAD_BYTES: usize = 96 * 1024;
const HOUR_MS: i64 = 60 * 60 * 1_000;

#[derive(Debug, thiserror::Error)]
pub enum GmailAutomationError {
    #[error("invalid Gmail automation input: {0}")]
    InvalidInput(String),
    #[error("Gmail automation record not found: {0}")]
    NotFound(String),
    #[error("Gmail automation conflict: {0}")]
    Conflict(String),
    #[error("corrupt Gmail automation data: {0}")]
    CorruptData(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmailAutomationCondition {
    pub from_contains: Option<String>,
    pub recipient_contains: Option<String>,
    pub subject_contains: Option<String>,
    #[serde(default)]
    pub all_label_ids: Vec<String>,
    #[serde(default)]
    pub any_label_ids: Vec<String>,
}

impl GmailAutomationCondition {
    pub fn matches(&self, message: &GmailMessageSummary) -> bool {
        contains_optional(message.from.as_deref(), self.from_contains.as_deref())
            && contains_recipient(message, self.recipient_contains.as_deref())
            && contains_optional(message.subject.as_deref(), self.subject_contains.as_deref())
            && self
                .all_label_ids
                .iter()
                .all(|required| message.label_ids.iter().any(|label| label == required))
            && (self.any_label_ids.is_empty()
                || self
                    .any_label_ids
                    .iter()
                    .any(|required| message.label_ids.iter().any(|label| label == required)))
    }

    fn canonicalized(mut self) -> Result<Self, GmailAutomationError> {
        self.from_contains = canonical_match(self.from_contains, "from_contains")?;
        self.recipient_contains = canonical_match(self.recipient_contains, "recipient_contains")?;
        self.subject_contains = canonical_match(self.subject_contains, "subject_contains")?;
        canonicalize_labels(&mut self.all_label_ids)?;
        canonicalize_labels(&mut self.any_label_ids)?;
        if self.from_contains.is_none()
            && self.recipient_contains.is_none()
            && self.subject_contains.is_none()
            && self.all_label_ids.is_empty()
            && self.any_label_ids.is_empty()
        {
            return Err(GmailAutomationError::InvalidInput(
                "a rule must define at least one deterministic condition".to_string(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmailAutomationAction {
    pub target_agent_id: AgentId,
    pub instruction: String,
    pub include_body: bool,
    pub max_body_bytes: usize,
    pub max_delivery_attempts: u8,
}

impl GmailAutomationAction {
    fn validated(mut self) -> Result<Self, GmailAutomationError> {
        self.instruction = self.instruction.trim().to_string();
        validate_text("instruction", &self.instruction, 1, MAX_INSTRUCTION_BYTES)?;
        if !(1..=256 * 1024).contains(&self.max_body_bytes) {
            return Err(GmailAutomationError::InvalidInput(
                "max_body_bytes must be between 1 and 262144".to_string(),
            ));
        }
        if !(1..=10).contains(&self.max_delivery_attempts) {
            return Err(GmailAutomationError::InvalidInput(
                "max_delivery_attempts must be between 1 and 10".to_string(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGmailAutomationRule {
    pub id: String,
    pub account_alias: GmailAccountAlias,
    pub name: String,
    pub condition: GmailAutomationCondition,
    pub action: GmailAutomationAction,
    pub enabled: bool,
    pub max_fires_per_hour: u16,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationRuleUpdate {
    pub expected_version: u64,
    pub name: String,
    pub condition: GmailAutomationCondition,
    pub action: GmailAutomationAction,
    pub enabled: bool,
    pub max_fires_per_hour: u16,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAutomationRuleRecord {
    pub id: String,
    pub account_alias: GmailAccountAlias,
    pub name: String,
    pub condition: GmailAutomationCondition,
    pub action: GmailAutomationAction,
    pub enabled: bool,
    pub max_fires_per_hour: u16,
    pub state_version: u64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailAutomationEventDecision {
    Queued,
    SuppressedRateLimit,
}

impl GmailAutomationEventDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::SuppressedRateLimit => "suppressed_rate_limit",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "suppressed_rate_limit" => Some(Self::SuppressedRateLimit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGmailAutomationMatch {
    pub idempotency_key: String,
    pub rule_id: String,
    pub expected_rule_version: u64,
    pub account_alias: GmailAccountAlias,
    pub message_id: String,
    pub history_id: String,
    pub metadata_json: String,
    pub occurred_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAutomationEventRecord {
    pub id: String,
    pub idempotency_key: String,
    pub rule_id: String,
    pub rule_version: u64,
    pub rule_snapshot_json: String,
    pub account_alias: GmailAccountAlias,
    pub message_id: String,
    pub history_id: String,
    pub metadata_json: String,
    pub decision: GmailAutomationEventDecision,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmailAutomationDeliveryPayload {
    pub rule_id: String,
    pub rule_version: u64,
    pub rule_name: String,
    pub account_alias: GmailAccountAlias,
    pub message_id: String,
    pub history_id: String,
    pub instruction: String,
    pub include_body: bool,
    pub max_body_bytes: usize,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailAutomationOutboxStatus {
    Pending,
    Delivering,
    RetryWait,
    Delivered,
    Dead,
    Uncertain,
}

impl GmailAutomationOutboxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::RetryWait => "retry_wait",
            Self::Delivered => "delivered",
            Self::Dead => "dead",
            Self::Uncertain => "uncertain",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "delivering" => Some(Self::Delivering),
            "retry_wait" => Some(Self::RetryWait),
            "delivered" => Some(Self::Delivered),
            "dead" => Some(Self::Dead),
            "uncertain" => Some(Self::Uncertain),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailAutomationOutboxRecord {
    pub id: String,
    pub idempotency_key: String,
    pub event_id: String,
    pub target_agent_id: AgentId,
    pub payload_json: String,
    pub status: GmailAutomationOutboxStatus,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub run_after_unix_ms: i64,
    pub lease_owner: Option<String>,
    pub lease_expires_at_unix_ms: Option<i64>,
    pub delivery_result_json: Option<String>,
    pub last_error: Option<String>,
    pub delivered_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailAutomationQueueOutcome {
    pub event: GmailAutomationEventRecord,
    pub outbox: Option<GmailAutomationOutboxRecord>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GmailAutomationRecoverySummary {
    pub uncertain: usize,
}

pub fn gmail_delivery_session_id(outbox_id: &str) -> SessionId {
    let seed = format!("captain:gmail-delivery-session:v1:{outbox_id}");
    SessionId(Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes()))
}

pub fn gmail_automation_rule_id(account: &GmailAccountAlias, name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !slug.is_empty() {
            slug.push('-');
            separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        let seed = format!("{}:{name}", account.as_str());
        let digest = Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes())
            .simple()
            .to_string();
        return format!("gmail-{}", &digest[..16]);
    }
    let prefix = format!("{}-", account.as_str());
    let keep = 96usize.saturating_sub(prefix.len());
    format!("{prefix}{}", slug.chars().take(keep).collect::<String>())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailSyncMode {
    Incremental,
    Recovery,
}

impl GmailSyncMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "incremental" => Some(Self::Incremental),
            "recovery" => Some(Self::Recovery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailSyncCheckpointRecord {
    pub account_alias: GmailAccountAlias,
    pub mode: GmailSyncMode,
    pub start_history_id: String,
    pub target_history_id: String,
    pub page_token: Option<String>,
    pub pages_processed: u32,
    pub messages_processed: u64,
    pub last_error_code: Option<String>,
    pub started_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone)]
pub struct GmailAutomationStore {
    conn: Arc<Mutex<Connection>>,
}

impl GmailAutomationStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn create_rule(
        &self,
        input: NewGmailAutomationRule,
    ) -> Result<GmailAutomationRuleRecord, GmailAutomationError> {
        let input = validate_new_rule(input)?;
        let condition_json = encode_json(&input.condition, "rule condition")?;
        let action_json = encode_json(&input.action, "rule action")?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_readable_account(&tx, &input.account_alias)?;
        if let Some(existing) = rule_by_id(&tx, &input.id)? {
            if existing.account_alias == input.account_alias
                && existing.name == input.name
                && existing.condition == input.condition
                && existing.action == input.action
                && existing.enabled == input.enabled
                && existing.max_fires_per_hour == input.max_fires_per_hour
            {
                tx.commit()?;
                return Ok(existing);
            }
            return Err(GmailAutomationError::Conflict(format!(
                "rule '{}' already exists with different input",
                input.id
            )));
        }
        tx.execute(
            "INSERT INTO gmail_automation_rules (
                 id, account_alias, name, condition_json, action_json, enabled,
                 max_fires_per_hour, state_version, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
            params![
                input.id,
                input.account_alias.as_str(),
                input.name,
                condition_json,
                action_json,
                i64::from(input.enabled),
                input.max_fires_per_hour,
                input.created_at_unix_ms,
            ],
        )?;
        let created = rule_by_id(&tx, &input.id)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("created rule disappeared".to_string())
        })?;
        tx.commit()?;
        Ok(created)
    }

    pub fn update_rule(
        &self,
        rule_id: &str,
        update: GmailAutomationRuleUpdate,
    ) -> Result<GmailAutomationRuleRecord, GmailAutomationError> {
        validate_token("rule id", rule_id, 96)?;
        if update.expected_version == 0 {
            return Err(GmailAutomationError::InvalidInput(
                "expected_version must be positive".to_string(),
            ));
        }
        let name = validated_name(update.name)?;
        let condition = update.condition.canonicalized()?;
        let action = update.action.validated()?;
        validate_rate_limit(update.max_fires_per_hour)?;
        validate_timestamp("updated_at", update.updated_at_unix_ms)?;
        let condition_json = encode_json(&condition, "rule condition")?;
        let action_json = encode_json(&action, "rule action")?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = rule_by_id(&tx, rule_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(rule_id.to_string()))?;
        if current.state_version != update.expected_version {
            return Err(GmailAutomationError::Conflict(format!(
                "rule version changed (expected {}, found {})",
                update.expected_version, current.state_version
            )));
        }
        let changed = tx.execute(
            "UPDATE gmail_automation_rules
             SET name = ?1, condition_json = ?2, action_json = ?3, enabled = ?4,
                 max_fires_per_hour = ?5, state_version = state_version + 1,
                 updated_at = ?6
             WHERE id = ?7 AND state_version = ?8",
            params![
                name,
                condition_json,
                action_json,
                i64::from(update.enabled),
                update.max_fires_per_hour,
                update.updated_at_unix_ms,
                rule_id,
                update.expected_version,
            ],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "rule changed while updating".to_string(),
            ));
        }
        let updated = rule_by_id(&tx, rule_id)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("updated rule disappeared".to_string())
        })?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn set_rule_enabled(
        &self,
        rule_id: &str,
        expected_version: u64,
        enabled: bool,
        updated_at_unix_ms: i64,
    ) -> Result<GmailAutomationRuleRecord, GmailAutomationError> {
        validate_token("rule id", rule_id, 96)?;
        if expected_version == 0 {
            return Err(GmailAutomationError::InvalidInput(
                "expected_version must be positive".to_string(),
            ));
        }
        validate_timestamp("updated_at", updated_at_unix_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = rule_by_id(&tx, rule_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(rule_id.to_string()))?;
        if current.state_version != expected_version {
            return Err(GmailAutomationError::Conflict(format!(
                "rule version changed (expected {expected_version}, found {})",
                current.state_version
            )));
        }
        if current.enabled == enabled {
            tx.commit()?;
            return Ok(current);
        }
        let changed = tx.execute(
            "UPDATE gmail_automation_rules
             SET enabled = ?1, state_version = state_version + 1, updated_at = ?2
             WHERE id = ?3 AND state_version = ?4",
            params![
                i64::from(enabled),
                updated_at_unix_ms,
                rule_id,
                expected_version
            ],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "rule changed while updating enabled state".to_string(),
            ));
        }
        let updated = rule_by_id(&tx, rule_id)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("updated rule disappeared".to_string())
        })?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn get_rule(
        &self,
        rule_id: &str,
    ) -> Result<Option<GmailAutomationRuleRecord>, GmailAutomationError> {
        validate_token("rule id", rule_id, 96)?;
        let conn = self.lock_conn()?;
        rule_by_id(&conn, rule_id).map_err(Into::into)
    }

    pub fn list_rules(
        &self,
        limit: usize,
    ) -> Result<Vec<GmailAutomationRuleRecord>, GmailAutomationError> {
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(&format!(
            "{RULE_SELECT} ORDER BY enabled DESC, account_alias, name COLLATE NOCASE, id LIMIT ?1"
        ))?;
        let rows = statement.query_map([limit.clamp(1, MAX_RULES) as i64], rule_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_rule(
        &self,
        rule_id: &str,
        expected_version: u64,
    ) -> Result<GmailAutomationRuleRecord, GmailAutomationError> {
        validate_token("rule id", rule_id, 96)?;
        if expected_version == 0 {
            return Err(GmailAutomationError::InvalidInput(
                "expected_version must be positive".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = rule_by_id(&tx, rule_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(rule_id.to_string()))?;
        if current.state_version != expected_version {
            return Err(GmailAutomationError::Conflict(format!(
                "rule version changed (expected {expected_version}, found {})",
                current.state_version
            )));
        }
        let event_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM gmail_automation_events WHERE rule_id = ?1",
            [rule_id],
            |row| row.get(0),
        )?;
        if event_count > 0 {
            return Err(GmailAutomationError::Conflict(
                "a rule with audit history cannot be deleted; disable it instead".to_string(),
            ));
        }
        let changed = tx.execute(
            "DELETE FROM gmail_automation_rules WHERE id = ?1 AND state_version = ?2",
            params![rule_id, expected_version],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "rule changed while deleting".to_string(),
            ));
        }
        tx.commit()?;
        Ok(current)
    }

    /// Resume an existing page checkpoint, or start one from the account's
    /// currently persisted history cursor.
    pub fn begin_sync(
        &self,
        account_alias: &GmailAccountAlias,
        start_history_id: &str,
        now_unix_ms: i64,
    ) -> Result<GmailSyncCheckpointRecord, GmailAutomationError> {
        validate_history_id(start_history_id)?;
        validate_timestamp("now", now_unix_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_account_sync_cursor(&tx, account_alias, start_history_id)?;
        if let Some(existing) = sync_checkpoint_by_alias(&tx, account_alias)? {
            if existing.start_history_id == start_history_id {
                tx.commit()?;
                return Ok(existing);
            }
            tx.execute(
                "DELETE FROM gmail_sync_checkpoints WHERE account_alias = ?1",
                [account_alias.as_str()],
            )?;
        }
        tx.execute(
            "INSERT INTO gmail_sync_checkpoints (
                 account_alias, mode, start_history_id, target_history_id,
                 page_token, pages_processed, messages_processed,
                 started_at, updated_at
             ) VALUES (?1, 'incremental', ?2, ?2, NULL, 0, 0, ?3, ?3)",
            params![account_alias.as_str(), start_history_id, now_unix_ms],
        )?;
        let checkpoint = sync_checkpoint_by_alias(&tx, account_alias)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("created sync checkpoint disappeared".to_string())
        })?;
        tx.commit()?;
        Ok(checkpoint)
    }

    pub fn mark_sync_recovery(
        &self,
        account_alias: &GmailAccountAlias,
        expected_start_history_id: &str,
        target_history_id: &str,
        now_unix_ms: i64,
    ) -> Result<GmailSyncCheckpointRecord, GmailAutomationError> {
        validate_history_id(expected_start_history_id)?;
        validate_history_id(target_history_id)?;
        validate_timestamp("now", now_unix_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_account_sync_cursor(&tx, account_alias, expected_start_history_id)?;
        let current = sync_checkpoint_by_alias(&tx, account_alias)?.ok_or_else(|| {
            GmailAutomationError::NotFound(format!("sync checkpoint for {account_alias}"))
        })?;
        if current.start_history_id != expected_start_history_id {
            return Err(GmailAutomationError::Conflict(
                "sync checkpoint start cursor changed".to_string(),
            ));
        }
        if current.mode == GmailSyncMode::Recovery && current.target_history_id == target_history_id
        {
            tx.commit()?;
            return Ok(current);
        }
        tx.execute(
            "UPDATE gmail_sync_checkpoints
             SET mode = 'recovery', target_history_id = ?1, page_token = NULL,
                 pages_processed = 0, messages_processed = 0,
                 last_error_code = NULL, updated_at = ?2
             WHERE account_alias = ?3 AND start_history_id = ?4",
            params![
                target_history_id,
                now_unix_ms,
                account_alias.as_str(),
                expected_start_history_id
            ],
        )?;
        let checkpoint = sync_checkpoint_by_alias(&tx, account_alias)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("recovery checkpoint disappeared".to_string())
        })?;
        tx.commit()?;
        Ok(checkpoint)
    }

    /// Commit one fully processed page. Returning `None` means the final page
    /// atomically advanced the account cursor and removed the checkpoint.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_sync_page(
        &self,
        account_alias: &GmailAccountAlias,
        expected_mode: GmailSyncMode,
        expected_start_history_id: &str,
        expected_page_token: Option<&str>,
        next_page_token: Option<&str>,
        target_history_id: &str,
        messages_processed: usize,
        now_unix_ms: i64,
    ) -> Result<Option<GmailSyncCheckpointRecord>, GmailAutomationError> {
        validate_history_id(expected_start_history_id)?;
        validate_history_id(target_history_id)?;
        validate_page_token(expected_page_token)?;
        validate_page_token(next_page_token)?;
        validate_timestamp("now", now_unix_ms)?;
        let processed = i64::try_from(messages_processed).map_err(|_| {
            GmailAutomationError::InvalidInput("processed message count is too large".to_string())
        })?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_account_sync_cursor(&tx, account_alias, expected_start_history_id)?;
        let current = sync_checkpoint_by_alias(&tx, account_alias)?.ok_or_else(|| {
            GmailAutomationError::NotFound(format!("sync checkpoint for {account_alias}"))
        })?;
        if current.mode != expected_mode
            || current.start_history_id != expected_start_history_id
            || current.page_token.as_deref() != expected_page_token
        {
            return Err(GmailAutomationError::Conflict(
                "sync checkpoint changed while processing the page".to_string(),
            ));
        }
        if let Some(next_page_token) = next_page_token {
            tx.execute(
                "UPDATE gmail_sync_checkpoints
                 SET target_history_id = ?1, page_token = ?2,
                     pages_processed = pages_processed + 1,
                     messages_processed = messages_processed + ?3,
                     last_error_code = NULL, updated_at = ?4
                 WHERE account_alias = ?5",
                params![
                    target_history_id,
                    next_page_token,
                    processed,
                    now_unix_ms,
                    account_alias.as_str()
                ],
            )?;
            let updated = sync_checkpoint_by_alias(&tx, account_alias)?.ok_or_else(|| {
                GmailAutomationError::CorruptData("updated sync checkpoint disappeared".to_string())
            })?;
            tx.commit()?;
            Ok(Some(updated))
        } else {
            let changed = tx.execute(
                "UPDATE gmail_accounts
                 SET history_id = ?1, status = 'ready', enabled = 1,
                     last_sync_at = ?2, last_error_code = NULL, updated_at = ?2
                 WHERE alias = ?3",
                params![target_history_id, now_unix_ms, account_alias.as_str()],
            )?;
            if changed != 1 {
                return Err(GmailAutomationError::NotFound(format!(
                    "Gmail account {account_alias}"
                )));
            }
            tx.execute(
                "DELETE FROM gmail_sync_checkpoints WHERE account_alias = ?1",
                [account_alias.as_str()],
            )?;
            tx.commit()?;
            Ok(None)
        }
    }

    pub fn record_sync_failure(
        &self,
        account_alias: &GmailAccountAlias,
        error_code: &str,
        now_unix_ms: i64,
    ) -> Result<(), GmailAutomationError> {
        validate_error_code(error_code)?;
        validate_timestamp("now", now_unix_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE gmail_sync_checkpoints
             SET last_error_code = ?1, updated_at = ?2 WHERE account_alias = ?3",
            params![error_code, now_unix_ms, account_alias.as_str()],
        )?;
        let changed = tx.execute(
            "UPDATE gmail_accounts SET last_error_code = ?1, updated_at = ?2
             WHERE alias = ?3",
            params![error_code, now_unix_ms, account_alias.as_str()],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::NotFound(format!(
                "Gmail account {account_alias}"
            )));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn enqueue_match(
        &self,
        input: &NewGmailAutomationMatch,
    ) -> Result<GmailAutomationQueueOutcome, GmailAutomationError> {
        validate_match(input)?;
        let metadata: serde_json::Value = serde_json::from_str(&input.metadata_json)
            .map_err(|_| GmailAutomationError::InvalidInput("metadata_json is invalid".into()))?;
        if !metadata.is_object() {
            return Err(GmailAutomationError::InvalidInput(
                "metadata_json must be a JSON object".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = event_by_idempotency(&tx, &input.idempotency_key)? {
            if event_matches_input(&existing, input) {
                let outbox = outbox_by_event(&tx, &existing.id)?;
                tx.commit()?;
                return Ok(GmailAutomationQueueOutcome {
                    event: existing,
                    outbox,
                });
            }
            return Err(GmailAutomationError::Conflict(
                "event idempotency key was reused with different input".to_string(),
            ));
        }
        let rule = rule_by_id(&tx, &input.rule_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(input.rule_id.clone()))?;
        if rule.state_version != input.expected_rule_version {
            return Err(GmailAutomationError::Conflict(format!(
                "rule '{}' changed while evaluating the message",
                rule.id
            )));
        }
        if !rule.enabled {
            return Err(GmailAutomationError::Conflict(format!(
                "rule '{}' is disabled",
                rule.id
            )));
        }
        if rule.account_alias != input.account_alias {
            return Err(GmailAutomationError::Conflict(
                "rule and event account aliases differ".to_string(),
            ));
        }
        let queued_in_window: i64 = tx.query_row(
            "SELECT COUNT(*) FROM gmail_automation_events
             WHERE rule_id = ?1 AND decision = 'queued'
               AND created_at > ?2 AND created_at <= ?3",
            params![
                rule.id,
                input.occurred_at_unix_ms.saturating_sub(HOUR_MS),
                input.occurred_at_unix_ms
            ],
            |row| row.get(0),
        )?;
        let decision = if queued_in_window >= i64::from(rule.max_fires_per_hour) {
            GmailAutomationEventDecision::SuppressedRateLimit
        } else {
            GmailAutomationEventDecision::Queued
        };
        let rule_snapshot_json = encode_json(&rule, "rule snapshot")?;
        if rule_snapshot_json.len() > MAX_METADATA_BYTES {
            return Err(GmailAutomationError::InvalidInput(
                "rule snapshot exceeds the safety limit".to_string(),
            ));
        }
        let event_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO gmail_automation_events (
                 id, idempotency_key, rule_id, rule_version, rule_snapshot_json,
                 account_alias, message_id, history_id, metadata_json, decision, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                event_id,
                input.idempotency_key,
                rule.id,
                rule.state_version,
                rule_snapshot_json,
                input.account_alias.as_str(),
                input.message_id,
                input.history_id,
                input.metadata_json,
                decision.as_str(),
                input.occurred_at_unix_ms,
            ],
        )?;
        let event = event_by_id(&tx, &event_id)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("created event disappeared".to_string())
        })?;
        let outbox = if decision == GmailAutomationEventDecision::Queued {
            let payload = GmailAutomationDeliveryPayload {
                rule_id: rule.id,
                rule_version: rule.state_version,
                rule_name: rule.name,
                account_alias: input.account_alias.clone(),
                message_id: input.message_id.clone(),
                history_id: input.history_id.clone(),
                instruction: rule.action.instruction,
                include_body: rule.action.include_body,
                max_body_bytes: rule.action.max_body_bytes,
                metadata,
            };
            let payload_json = encode_json(&payload, "delivery payload")?;
            if payload_json.len() > MAX_PAYLOAD_BYTES {
                return Err(GmailAutomationError::InvalidInput(
                    "delivery payload exceeds the safety limit".to_string(),
                ));
            }
            let outbox_id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO gmail_automation_outbox (
                     id, idempotency_key, event_id, target_agent_id, payload_json,
                     status, attempt_count, max_attempts, run_after, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 0, ?6, ?7, ?7, ?7)",
                params![
                    outbox_id,
                    format!("gmail-delivery:{event_id}"),
                    event_id,
                    rule.action.target_agent_id.to_string(),
                    payload_json,
                    rule.action.max_delivery_attempts,
                    input.occurred_at_unix_ms,
                ],
            )?;
            Some(outbox_by_id(&tx, &outbox_id)?.ok_or_else(|| {
                GmailAutomationError::CorruptData("created outbox item disappeared".to_string())
            })?)
        } else {
            None
        };
        tx.commit()?;
        Ok(GmailAutomationQueueOutcome { event, outbox })
    }

    pub fn get_outbox(
        &self,
        outbox_id: &str,
    ) -> Result<Option<GmailAutomationOutboxRecord>, GmailAutomationError> {
        validate_token("outbox id", outbox_id, 96)?;
        let conn = self.lock_conn()?;
        outbox_by_id(&conn, outbox_id).map_err(Into::into)
    }

    pub fn list_outbox(
        &self,
        status: Option<GmailAutomationOutboxStatus>,
        limit: usize,
    ) -> Result<Vec<GmailAutomationOutboxRecord>, GmailAutomationError> {
        let conn = self.lock_conn()?;
        let limit = limit.clamp(1, MAX_OUTBOX_RESULTS) as i64;
        let mut records = Vec::new();
        if let Some(status) = status {
            let mut statement = conn.prepare(&format!(
                "{OUTBOX_SELECT} WHERE status = ?1 ORDER BY updated_at DESC, id DESC LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![status.as_str(), limit], outbox_from_row)?;
            records.extend(rows.collect::<Result<Vec<_>, _>>()?);
        } else {
            let mut statement = conn.prepare(&format!(
                "{OUTBOX_SELECT} ORDER BY updated_at DESC, id DESC LIMIT ?1"
            ))?;
            let rows = statement.query_map([limit], outbox_from_row)?;
            records.extend(rows.collect::<Result<Vec<_>, _>>()?);
        }
        Ok(records)
    }

    pub fn claim_due_outbox(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<Option<GmailAutomationOutboxRecord>, GmailAutomationError> {
        self.claim_due_outbox_excluding(worker, now_unix_ms, lease_duration_ms, &[])
    }

    /// Claim the oldest due delivery whose target is not already processing
    /// another Gmail automation. The delivery worker has two slots, so the
    /// exclusion list is deliberately bounded to two agent IDs.
    pub fn claim_due_outbox_excluding(
        &self,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
        excluded_target_agents: &[AgentId],
    ) -> Result<Option<GmailAutomationOutboxRecord>, GmailAutomationError> {
        validate_token("worker", worker, 96)?;
        validate_timestamp("now", now_unix_ms)?;
        validate_lease_duration(lease_duration_ms)?;
        if excluded_target_agents.len() > 2 {
            return Err(GmailAutomationError::InvalidInput(
                "at most two active Gmail delivery agents may be excluded".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reconcile_outbox_in_tx(&tx, now_unix_ms, false)?;
        let id: Option<String> = match excluded_target_agents {
            [] => tx
                .query_row(
                    "SELECT id FROM gmail_automation_outbox
                     WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
                     ORDER BY run_after, created_at, id LIMIT 1",
                    [now_unix_ms],
                    |row| row.get(0),
                )
                .optional()?,
            [agent] => tx
                .query_row(
                    "SELECT id FROM gmail_automation_outbox
                     WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
                       AND target_agent_id != ?2
                     ORDER BY run_after, created_at, id LIMIT 1",
                    params![now_unix_ms, agent.to_string()],
                    |row| row.get(0),
                )
                .optional()?,
            [first, second] => tx
                .query_row(
                    "SELECT id FROM gmail_automation_outbox
                     WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
                       AND target_agent_id NOT IN (?2, ?3)
                     ORDER BY run_after, created_at, id LIMIT 1",
                    params![now_unix_ms, first.to_string(), second.to_string()],
                    |row| row.get(0),
                )
                .optional()?,
            _ => unreachable!("exclusion count was validated"),
        };
        let Some(id) = id else {
            tx.commit()?;
            return Ok(None);
        };
        let changed = tx.execute(
            "UPDATE gmail_automation_outbox
             SET status = 'delivering', attempt_count = attempt_count + 1,
                 lease_owner = ?1, lease_expires_at = ?2, updated_at = ?3
             WHERE id = ?4 AND status IN ('pending', 'retry_wait') AND run_after <= ?3",
            params![worker, now_unix_ms + lease_duration_ms, now_unix_ms, id],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "outbox item changed while claiming".to_string(),
            ));
        }
        let claimed = outbox_by_id(&tx, &id)?.ok_or_else(|| {
            GmailAutomationError::CorruptData("claimed outbox item disappeared".to_string())
        })?;
        tx.commit()?;
        Ok(Some(claimed))
    }

    pub fn renew_outbox_lease(
        &self,
        outbox_id: &str,
        worker: &str,
        now_unix_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_token("outbox id", outbox_id, 96)?;
        validate_token("worker", worker, 96)?;
        validate_timestamp("now", now_unix_ms)?;
        validate_lease_duration(lease_duration_ms)?;
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE gmail_automation_outbox
             SET lease_expires_at = ?1, updated_at = ?2
             WHERE id = ?3 AND status = 'delivering' AND lease_owner = ?4
               AND lease_expires_at > ?2",
            params![
                now_unix_ms.saturating_add(lease_duration_ms),
                now_unix_ms,
                outbox_id,
                worker
            ],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "lease renewal requires the current live delivery lease".to_string(),
            ));
        }
        outbox_by_id(&conn, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))
    }

    pub fn complete_outbox(
        &self,
        outbox_id: &str,
        worker: &str,
        result_json: Option<&str>,
        completed_at_unix_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_token("outbox id", outbox_id, 96)?;
        validate_token("worker", worker, 96)?;
        validate_timestamp("completed_at", completed_at_unix_ms)?;
        if let Some(result) = result_json {
            validate_json("delivery result", result, 32 * 1024)?;
        }
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE gmail_automation_outbox
             SET status = 'delivered', delivery_result_json = ?1, delivered_at = ?2,
                 lease_owner = NULL, lease_expires_at = NULL, last_error = NULL,
                 updated_at = ?2
             WHERE id = ?3 AND status = 'delivering' AND lease_owner = ?4
               AND lease_expires_at > ?2",
            params![result_json, completed_at_unix_ms, outbox_id, worker],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "completion requires the current live delivery lease".to_string(),
            ));
        }
        outbox_by_id(&conn, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))
    }

    /// Retry only a failure known to have happened before the agent turn was accepted.
    pub fn retry_outbox(
        &self,
        outbox_id: &str,
        worker: &str,
        error: &str,
        retry_at_unix_ms: i64,
        failed_at_unix_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_outbox_failure_input(outbox_id, worker, error, failed_at_unix_ms)?;
        validate_timestamp("retry_at", retry_at_unix_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = require_live_lease(&tx, outbox_id, worker, failed_at_unix_ms)?;
        let (status, run_after) = if current.attempt_count < current.max_attempts {
            (GmailAutomationOutboxStatus::RetryWait, retry_at_unix_ms)
        } else {
            (GmailAutomationOutboxStatus::Dead, failed_at_unix_ms)
        };
        tx.execute(
            "UPDATE gmail_automation_outbox
             SET status = ?1, run_after = ?2, lease_owner = NULL,
                 lease_expires_at = NULL, last_error = ?3, updated_at = ?4
             WHERE id = ?5 AND status = 'delivering' AND lease_owner = ?6",
            params![
                status.as_str(),
                run_after,
                error,
                failed_at_unix_ms,
                outbox_id,
                worker
            ],
        )?;
        let result = outbox_by_id(&tx, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))?;
        tx.commit()?;
        Ok(result)
    }

    pub fn mark_outbox_uncertain(
        &self,
        outbox_id: &str,
        worker: &str,
        reason: &str,
        occurred_at_unix_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_outbox_failure_input(outbox_id, worker, reason, occurred_at_unix_ms)?;
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE gmail_automation_outbox
             SET status = 'uncertain', lease_owner = NULL, lease_expires_at = NULL,
                 last_error = ?1, updated_at = ?2
             WHERE id = ?3 AND status = 'delivering' AND lease_owner = ?4",
            params![reason, occurred_at_unix_ms, outbox_id, worker],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "uncertain transition requires the current delivery lease".to_string(),
            ));
        }
        outbox_by_id(&conn, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))
    }

    pub fn dead_letter_outbox(
        &self,
        outbox_id: &str,
        worker: &str,
        error: &str,
        failed_at_unix_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_outbox_failure_input(outbox_id, worker, error, failed_at_unix_ms)?;
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE gmail_automation_outbox
             SET status = 'dead', lease_owner = NULL, lease_expires_at = NULL,
                 last_error = ?1, updated_at = ?2
             WHERE id = ?3 AND status = 'delivering' AND lease_owner = ?4
               AND lease_expires_at > ?2",
            params![error, failed_at_unix_ms, outbox_id, worker],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "dead-letter requires the current live delivery lease".to_string(),
            ));
        }
        outbox_by_id(&conn, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))
    }

    pub fn requeue_uncertain(
        &self,
        outbox_id: &str,
        operator: &str,
        now_unix_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_token("outbox id", outbox_id, 96)?;
        validate_token("operator", operator, 96)?;
        validate_timestamp("now", now_unix_ms)?;
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE gmail_automation_outbox
             SET status = 'retry_wait', run_after = ?1,
                 last_error = ?2, updated_at = ?1
             WHERE id = ?3 AND status = 'uncertain' AND attempt_count < max_attempts",
            params![
                now_unix_ms,
                format!("operator '{operator}' explicitly requeued uncertain delivery"),
                outbox_id
            ],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(
                "only an uncertain item with attempts remaining can be requeued".to_string(),
            ));
        }
        outbox_by_id(&conn, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))
    }

    /// Explicit operator recovery after inspecting a terminal delivery state.
    /// Resetting attempts is safe only because the caller must confirm the
    /// exact state it reviewed; an uncertain item may already have executed.
    pub fn requeue_reviewed(
        &self,
        outbox_id: &str,
        operator: &str,
        expected_status: GmailAutomationOutboxStatus,
        now_unix_ms: i64,
    ) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
        validate_token("outbox id", outbox_id, 96)?;
        validate_token("operator", operator, 96)?;
        validate_timestamp("now", now_unix_ms)?;
        if !matches!(
            expected_status,
            GmailAutomationOutboxStatus::Dead | GmailAutomationOutboxStatus::Uncertain
        ) {
            return Err(GmailAutomationError::InvalidInput(
                "only a reviewed dead or uncertain delivery may be requeued".to_string(),
            ));
        }
        let conn = self.lock_conn()?;
        let changed = conn.execute(
            "UPDATE gmail_automation_outbox
             SET status = 'retry_wait', attempt_count = 0, run_after = ?1,
                 lease_owner = NULL, lease_expires_at = NULL,
                 delivery_result_json = NULL, delivered_at = NULL,
                 last_error = ?2, updated_at = ?1
             WHERE id = ?3 AND status = ?4",
            params![
                now_unix_ms,
                format!(
                    "operator '{operator}' explicitly requeued reviewed {} delivery",
                    expected_status.as_str()
                ),
                outbox_id,
                expected_status.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(GmailAutomationError::Conflict(format!(
                "delivery is no longer in reviewed state '{}'",
                expected_status.as_str()
            )));
        }
        outbox_by_id(&conn, outbox_id)?
            .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))
    }

    pub fn reconcile_expired_outbox(
        &self,
        now_unix_ms: i64,
    ) -> Result<GmailAutomationRecoverySummary, GmailAutomationError> {
        self.reconcile_outbox(now_unix_ms, false)
    }

    pub fn reconcile_outbox_after_restart(
        &self,
        now_unix_ms: i64,
    ) -> Result<GmailAutomationRecoverySummary, GmailAutomationError> {
        self.reconcile_outbox(now_unix_ms, true)
    }

    fn reconcile_outbox(
        &self,
        now_unix_ms: i64,
        include_unexpired: bool,
    ) -> Result<GmailAutomationRecoverySummary, GmailAutomationError> {
        validate_timestamp("now", now_unix_ms)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let summary = reconcile_outbox_in_tx(&tx, now_unix_ms, include_unexpired)?;
        tx.commit()?;
        Ok(summary)
    }

    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, GmailAutomationError> {
        self.conn.lock().map_err(|error| {
            GmailAutomationError::CorruptData(format!("database lock poisoned: {error}"))
        })
    }
}

fn validate_new_rule(
    mut input: NewGmailAutomationRule,
) -> Result<NewGmailAutomationRule, GmailAutomationError> {
    validate_token("rule id", &input.id, 96)?;
    input.name = validated_name(input.name)?;
    input.condition = input.condition.canonicalized()?;
    input.action = input.action.validated()?;
    validate_rate_limit(input.max_fires_per_hour)?;
    validate_timestamp("created_at", input.created_at_unix_ms)?;
    Ok(input)
}

fn validate_match(input: &NewGmailAutomationMatch) -> Result<(), GmailAutomationError> {
    validate_token("event idempotency key", &input.idempotency_key, 192)?;
    validate_token("rule id", &input.rule_id, 96)?;
    if input.expected_rule_version == 0 {
        return Err(GmailAutomationError::InvalidInput(
            "expected rule version must be positive".to_string(),
        ));
    }
    validate_identifier("message id", &input.message_id, 256)?;
    validate_history_id(&input.history_id)?;
    validate_json("metadata_json", &input.metadata_json, MAX_METADATA_BYTES)?;
    validate_timestamp("occurred_at", input.occurred_at_unix_ms)
}

fn validate_page_token(value: Option<&str>) -> Result<(), GmailAutomationError> {
    if value.is_some_and(|token| {
        token.is_empty() || token.len() > 2_048 || token.chars().any(char::is_control)
    }) {
        return Err(GmailAutomationError::InvalidInput(
            "page token is empty, oversized, or malformed".to_string(),
        ));
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), GmailAutomationError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
    {
        return Err(GmailAutomationError::InvalidInput(
            "error code must use 1 to 64 lowercase ASCII letters or underscores".to_string(),
        ));
    }
    Ok(())
}

fn require_readable_account(
    conn: &Connection,
    alias: &GmailAccountAlias,
) -> Result<(), GmailAutomationError> {
    let profile: Option<String> = conn
        .query_row(
            "SELECT access_profile FROM gmail_accounts
             WHERE alias = ?1 AND enabled = 1 AND status = 'ready'",
            [alias.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    let profile = profile.ok_or_else(|| {
        GmailAutomationError::InvalidInput(format!(
            "Gmail account '{alias}' is missing, disabled, or not ready"
        ))
    })?;
    let profile = GmailAccessProfile::from_str(&profile).map_err(|_| {
        GmailAutomationError::CorruptData(format!(
            "Gmail account '{alias}' has an invalid access profile"
        ))
    })?;
    if !profile.can_read() {
        return Err(GmailAutomationError::InvalidInput(format!(
            "Gmail account '{alias}' does not grant read access"
        )));
    }
    Ok(())
}

fn require_account_sync_cursor(
    conn: &Connection,
    alias: &GmailAccountAlias,
    expected_history_id: &str,
) -> Result<(), GmailAutomationError> {
    require_readable_account(conn, alias)?;
    let history_id: Option<String> = conn
        .query_row(
            "SELECT history_id FROM gmail_accounts WHERE alias = ?1",
            [alias.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if history_id.as_deref() != Some(expected_history_id) {
        return Err(GmailAutomationError::Conflict(format!(
            "Gmail account '{alias}' history cursor changed"
        )));
    }
    Ok(())
}

fn reconcile_outbox_in_tx(
    tx: &Transaction<'_>,
    now_unix_ms: i64,
    include_unexpired: bool,
) -> Result<GmailAutomationRecoverySummary, GmailAutomationError> {
    let reason = if include_unexpired {
        "runtime restarted during delivery; inspect the target agent session before explicit replay"
    } else {
        "delivery lease expired; inspect the target agent session before explicit replay"
    };
    let uncertain = tx.execute(
        "UPDATE gmail_automation_outbox
         SET status = 'uncertain', lease_owner = NULL, lease_expires_at = NULL,
             last_error = ?1, updated_at = ?2
         WHERE status = 'delivering' AND (?3 = 1 OR lease_expires_at <= ?2)",
        params![reason, now_unix_ms, i64::from(include_unexpired)],
    )?;
    Ok(GmailAutomationRecoverySummary { uncertain })
}

fn require_live_lease(
    conn: &Connection,
    outbox_id: &str,
    worker: &str,
    now_unix_ms: i64,
) -> Result<GmailAutomationOutboxRecord, GmailAutomationError> {
    let current = outbox_by_id(conn, outbox_id)?
        .ok_or_else(|| GmailAutomationError::NotFound(outbox_id.to_string()))?;
    if current.status != GmailAutomationOutboxStatus::Delivering
        || current.lease_owner.as_deref() != Some(worker)
        || current.lease_expires_at_unix_ms <= Some(now_unix_ms)
    {
        return Err(GmailAutomationError::Conflict(
            "operation requires the current live delivery lease".to_string(),
        ));
    }
    Ok(current)
}

const RULE_SELECT: &str = "SELECT id, account_alias, name, condition_json, action_json,
            enabled, max_fires_per_hour, state_version, created_at, updated_at
     FROM gmail_automation_rules";

fn rule_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<GmailAutomationRuleRecord>> {
    conn.query_row(&format!("{RULE_SELECT} WHERE id = ?1"), [id], rule_from_row)
        .optional()
}

fn rule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GmailAutomationRuleRecord> {
    let alias: String = row.get(1)?;
    let condition_json: String = row.get(3)?;
    let action_json: String = row.get(4)?;
    Ok(GmailAutomationRuleRecord {
        id: row.get(0)?,
        account_alias: parse_alias(alias, 1)?,
        name: row.get(2)?,
        condition: parse_json(&condition_json, 3, "rule condition")?,
        action: parse_json(&action_json, 4, "rule action")?,
        enabled: row.get::<_, i64>(5)? != 0,
        max_fires_per_hour: row.get::<_, i64>(6)?.max(0) as u16,
        state_version: row.get::<_, i64>(7)?.max(0) as u64,
        created_at_unix_ms: row.get(8)?,
        updated_at_unix_ms: row.get(9)?,
    })
}

const EVENT_SELECT: &str = "SELECT id, idempotency_key, rule_id, rule_version,
            rule_snapshot_json, account_alias, message_id, history_id,
            metadata_json, decision, created_at
     FROM gmail_automation_events";

fn event_by_id(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<GmailAutomationEventRecord>> {
    conn.query_row(
        &format!("{EVENT_SELECT} WHERE id = ?1"),
        [id],
        event_from_row,
    )
    .optional()
}

fn event_by_idempotency(
    conn: &Connection,
    key: &str,
) -> rusqlite::Result<Option<GmailAutomationEventRecord>> {
    conn.query_row(
        &format!("{EVENT_SELECT} WHERE idempotency_key = ?1"),
        [key],
        event_from_row,
    )
    .optional()
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GmailAutomationEventRecord> {
    let alias: String = row.get(5)?;
    let decision: String = row.get(9)?;
    Ok(GmailAutomationEventRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        rule_id: row.get(2)?,
        rule_version: row.get::<_, i64>(3)?.max(0) as u64,
        rule_snapshot_json: row.get(4)?,
        account_alias: parse_alias(alias, 5)?,
        message_id: row.get(6)?,
        history_id: row.get(7)?,
        metadata_json: row.get(8)?,
        decision: GmailAutomationEventDecision::parse(&decision).ok_or_else(|| {
            invalid_column(9, format!("unknown Gmail event decision '{decision}'"))
        })?,
        created_at_unix_ms: row.get(10)?,
    })
}

const OUTBOX_SELECT: &str = "SELECT id, idempotency_key, event_id, target_agent_id,
            payload_json, status, attempt_count, max_attempts, run_after,
            lease_owner, lease_expires_at, delivery_result_json, last_error,
            delivered_at, created_at, updated_at
     FROM gmail_automation_outbox";

fn outbox_by_id(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<GmailAutomationOutboxRecord>> {
    conn.query_row(
        &format!("{OUTBOX_SELECT} WHERE id = ?1"),
        [id],
        outbox_from_row,
    )
    .optional()
}

fn outbox_by_event(
    conn: &Connection,
    event_id: &str,
) -> rusqlite::Result<Option<GmailAutomationOutboxRecord>> {
    conn.query_row(
        &format!("{OUTBOX_SELECT} WHERE event_id = ?1"),
        [event_id],
        outbox_from_row,
    )
    .optional()
}

fn outbox_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GmailAutomationOutboxRecord> {
    let agent_id: String = row.get(3)?;
    let status: String = row.get(5)?;
    Ok(GmailAutomationOutboxRecord {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        event_id: row.get(2)?,
        target_agent_id: AgentId::from_str(&agent_id)
            .map_err(|error| invalid_column(3, format!("invalid target agent id: {error}")))?,
        payload_json: row.get(4)?,
        status: GmailAutomationOutboxStatus::parse(&status)
            .ok_or_else(|| invalid_column(5, format!("unknown outbox status '{status}'")))?,
        attempt_count: row.get::<_, i64>(6)?.max(0) as u32,
        max_attempts: row.get::<_, i64>(7)?.max(0) as u32,
        run_after_unix_ms: row.get(8)?,
        lease_owner: row.get(9)?,
        lease_expires_at_unix_ms: row.get(10)?,
        delivery_result_json: row.get(11)?,
        last_error: row.get(12)?,
        delivered_at_unix_ms: row.get(13)?,
        created_at_unix_ms: row.get(14)?,
        updated_at_unix_ms: row.get(15)?,
    })
}

const SYNC_CHECKPOINT_SELECT: &str = "SELECT account_alias, mode, start_history_id,
            target_history_id, page_token, pages_processed, messages_processed,
            last_error_code, started_at, updated_at
     FROM gmail_sync_checkpoints";

fn sync_checkpoint_by_alias(
    conn: &Connection,
    alias: &GmailAccountAlias,
) -> rusqlite::Result<Option<GmailSyncCheckpointRecord>> {
    conn.query_row(
        &format!("{SYNC_CHECKPOINT_SELECT} WHERE account_alias = ?1"),
        [alias.as_str()],
        sync_checkpoint_from_row,
    )
    .optional()
}

fn sync_checkpoint_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<GmailSyncCheckpointRecord> {
    let alias: String = row.get(0)?;
    let mode: String = row.get(1)?;
    let pages_processed = u32::try_from(row.get::<_, i64>(5)?)
        .map_err(|_| invalid_column(5, "pages_processed is outside the u32 range".to_string()))?;
    let messages_processed = u64::try_from(row.get::<_, i64>(6)?)
        .map_err(|_| invalid_column(6, "messages_processed cannot be negative".to_string()))?;
    Ok(GmailSyncCheckpointRecord {
        account_alias: parse_alias(alias, 0)?,
        mode: GmailSyncMode::parse(&mode)
            .ok_or_else(|| invalid_column(1, format!("unknown sync mode '{mode}'")))?,
        start_history_id: row.get(2)?,
        target_history_id: row.get(3)?,
        page_token: row.get(4)?,
        pages_processed,
        messages_processed,
        last_error_code: row.get(7)?,
        started_at_unix_ms: row.get(8)?,
        updated_at_unix_ms: row.get(9)?,
    })
}

fn event_matches_input(
    event: &GmailAutomationEventRecord,
    input: &NewGmailAutomationMatch,
) -> bool {
    event.rule_id == input.rule_id
        && event.account_alias == input.account_alias
        && event.message_id == input.message_id
}

fn contains_optional(haystack: Option<&str>, needle: Option<&str>) -> bool {
    needle.is_none_or(|needle| {
        haystack.is_some_and(|value| value.to_ascii_lowercase().contains(needle))
    })
}

fn contains_recipient(message: &GmailMessageSummary, needle: Option<&str>) -> bool {
    needle.is_none_or(|needle| {
        message
            .to
            .as_deref()
            .into_iter()
            .chain(message.cc.as_deref())
            .any(|value| value.to_ascii_lowercase().contains(needle))
    })
}

fn canonical_match(
    value: Option<String>,
    name: &str,
) -> Result<Option<String>, GmailAutomationError> {
    value
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            validate_text(name, &value, 1, MAX_MATCH_BYTES)?;
            Ok(value)
        })
        .transpose()
}

fn canonicalize_labels(labels: &mut Vec<String>) -> Result<(), GmailAutomationError> {
    if labels.len() > 100 {
        return Err(GmailAutomationError::InvalidInput(
            "a rule may reference at most 100 labels per condition".to_string(),
        ));
    }
    for label in labels.iter_mut() {
        *label = label.trim().to_string();
        validate_identifier("label id", label, 256)?;
    }
    labels.sort();
    labels.dedup();
    Ok(())
}

fn validated_name(value: String) -> Result<String, GmailAutomationError> {
    let value = value.trim().to_string();
    validate_text("rule name", &value, 1, MAX_RULE_NAME_BYTES)?;
    Ok(value)
}

fn validate_rate_limit(value: u16) -> Result<(), GmailAutomationError> {
    if !(1..=1_000).contains(&value) {
        return Err(GmailAutomationError::InvalidInput(
            "max_fires_per_hour must be between 1 and 1000".to_string(),
        ));
    }
    Ok(())
}

fn validate_lease_duration(value: i64) -> Result<(), GmailAutomationError> {
    if !(1_000..=3_600_000).contains(&value) {
        return Err(GmailAutomationError::InvalidInput(
            "lease duration must be between 1 second and 1 hour".to_string(),
        ));
    }
    Ok(())
}

fn validate_outbox_failure_input(
    outbox_id: &str,
    worker: &str,
    error: &str,
    occurred_at: i64,
) -> Result<(), GmailAutomationError> {
    validate_token("outbox id", outbox_id, 96)?;
    validate_token("worker", worker, 96)?;
    validate_text("delivery error", error, 1, 2_048)?;
    validate_timestamp("occurred_at", occurred_at)
}

fn validate_history_id(value: &str) -> Result<(), GmailAutomationError> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GmailAutomationError::InvalidInput(
            "history id must contain 1 to 128 digits".to_string(),
        ));
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str, max: usize) -> Result<(), GmailAutomationError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(GmailAutomationError::InvalidInput(format!(
            "{name} is empty, oversized, or malformed"
        )));
    }
    Ok(())
}

fn validate_token(name: &str, value: &str, max: usize) -> Result<(), GmailAutomationError> {
    if value.is_empty()
        || value.len() > max
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
    {
        return Err(GmailAutomationError::InvalidInput(format!(
            "{name} is empty, oversized, or malformed"
        )));
    }
    Ok(())
}

fn validate_text(
    name: &str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), GmailAutomationError> {
    if value.len() < min || value.len() > max || value.chars().any(|character| character == '\0') {
        return Err(GmailAutomationError::InvalidInput(format!(
            "{name} must contain {min} to {max} bytes and no NUL"
        )));
    }
    Ok(())
}

fn validate_timestamp(name: &str, value: i64) -> Result<(), GmailAutomationError> {
    if value < 0 {
        return Err(GmailAutomationError::InvalidInput(format!(
            "{name} cannot be negative"
        )));
    }
    Ok(())
}

fn validate_json(name: &str, value: &str, max: usize) -> Result<(), GmailAutomationError> {
    validate_text(name, value, 2, max)?;
    serde_json::from_str::<serde_json::Value>(value)
        .map(|_| ())
        .map_err(|_| GmailAutomationError::InvalidInput(format!("{name} is invalid JSON")))
}

fn encode_json<T: Serialize>(value: &T, name: &str) -> Result<String, GmailAutomationError> {
    serde_json::to_string(value).map_err(|error| {
        GmailAutomationError::CorruptData(format!("could not encode {name}: {error}"))
    })
}

fn parse_json<T: for<'de> Deserialize<'de>>(
    value: &str,
    column: usize,
    name: &str,
) -> rusqlite::Result<T> {
    serde_json::from_str(value)
        .map_err(|error| invalid_column(column, format!("invalid {name}: {error}")))
}

fn parse_alias(value: String, column: usize) -> rusqlite::Result<GmailAccountAlias> {
    GmailAccountAlias::parse(&value)
        .map_err(|error| invalid_column(column, format!("invalid account alias: {error}")))
}

fn invalid_column(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}
