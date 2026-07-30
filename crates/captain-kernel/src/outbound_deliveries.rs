//! Crash-safe generic outbound channel delivery ledger.

use crate::error::{KernelError, KernelResult};
use crate::{shared_memory_agent_id, CaptainKernel};
use captain_channels::outbound_delivery::{
    OutboundDeliveryClaim, OutboundDeliveryIntent, OutboundDeliveryPreparation,
    OutboundDeliverySnapshot,
};
use captain_types::error::CaptainError;
use serde::{Deserialize, Serialize};

const STATE_KEY: &str = "__captain_outbound_deliveries_v1";
const STATE_SCHEMA_VERSION: u16 = 1;
const MAX_ATTEMPTS: u32 = 3;
const MAX_RECORDS: usize = 500;
const MAX_INTENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_LIVE_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
const TERMINAL_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const LEASE_DURATION_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DeliveryState {
    Pending,
    Attempting,
    Delivered,
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryRecord {
    id: String,
    intent: OutboundDeliveryIntent,
    state: DeliveryState,
    attempt_count: u32,
    max_attempts: u32,
    run_after_unix_ms: i64,
    lease_owner: Option<String>,
    lease_token: Option<String>,
    lease_expires_at_unix_ms: Option<i64>,
    possible_duplicate: bool,
    last_error: Option<String>,
    external_message_id: Option<String>,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    delivered_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct DeliveryLedger {
    schema_version: u16,
    records: Vec<DeliveryRecord>,
}

impl Default for DeliveryLedger {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            records: Vec::new(),
        }
    }
}

impl CaptainKernel {
    pub fn prepare_outbound_delivery(
        &self,
        intent: OutboundDeliveryIntent,
        lease_owner: &str,
    ) -> Result<OutboundDeliveryPreparation, String> {
        validate_intent(&intent)?;
        let now = now_unix_ms();
        self.mutate_outbound_delivery_ledger(|ledger| {
            prune_ledger(ledger, now);
            if let Some(existing) = ledger
                .records
                .iter_mut()
                .find(|record| record.intent.idempotency_key == intent.idempotency_key)
            {
                if existing.state == DeliveryState::Pending && existing.run_after_unix_ms <= now {
                    return Ok(OutboundDeliveryPreparation::Claimed(claim_record(
                        existing,
                        lease_owner,
                        now,
                    )));
                }
                if existing.state == DeliveryState::Attempting
                    && (existing.lease_owner.as_deref() != Some(lease_owner)
                        || existing.lease_expires_at_unix_ms.unwrap_or(0) <= now)
                {
                    existing.possible_duplicate = true;
                    return Ok(OutboundDeliveryPreparation::Claimed(claim_record(
                        existing,
                        lease_owner,
                        now,
                    )));
                }
                return Ok(OutboundDeliveryPreparation::AlreadyHandled);
            }

            let live = ledger
                .records
                .iter()
                .filter(|record| {
                    matches!(
                        record.state,
                        DeliveryState::Pending | DeliveryState::Attempting
                    )
                })
                .count();
            if live >= MAX_RECORDS {
                return Err(
                    "outbound delivery ledger is full; operator action required".to_string()
                );
            }
            let live_payload_bytes = ledger
                .records
                .iter()
                .filter(|record| {
                    matches!(
                        record.state,
                        DeliveryState::Pending | DeliveryState::Attempting
                    )
                })
                .map(|record| serialized_intent_len(&record.intent))
                .sum::<usize>();
            let intent_bytes = serialized_intent_len(&intent);
            if live_payload_bytes.saturating_add(intent_bytes) > MAX_LIVE_PAYLOAD_BYTES {
                return Err(
                    "outbound delivery payload budget is full; operator action required"
                        .to_string(),
                );
            }

            let mut record = DeliveryRecord {
                id: uuid::Uuid::new_v4().to_string(),
                intent,
                state: DeliveryState::Pending,
                attempt_count: 0,
                max_attempts: MAX_ATTEMPTS,
                run_after_unix_ms: now,
                lease_owner: None,
                lease_token: None,
                lease_expires_at_unix_ms: None,
                possible_duplicate: false,
                last_error: None,
                external_message_id: None,
                created_at_unix_ms: now,
                updated_at_unix_ms: now,
                delivered_at_unix_ms: None,
            };
            let claim = claim_record(&mut record, lease_owner, now);
            ledger.records.push(record);
            Ok(OutboundDeliveryPreparation::Claimed(claim))
        })
        .map_err(|error| error.to_string())?
    }

    pub fn claim_outbound_delivery(
        &self,
        channel: &str,
        lease_owner: &str,
    ) -> Result<Option<OutboundDeliveryClaim>, String> {
        let now = now_unix_ms();
        self.mutate_outbound_delivery_ledger(|ledger| {
            prune_ledger(ledger, now);
            let record = ledger.records.iter_mut().find(|record| {
                if record.intent.channel != channel {
                    return false;
                }
                match record.state {
                    DeliveryState::Pending => record.run_after_unix_ms <= now,
                    DeliveryState::Attempting => {
                        record.lease_owner.as_deref() != Some(lease_owner)
                            || record.lease_expires_at_unix_ms.unwrap_or(0) <= now
                    }
                    DeliveryState::Delivered | DeliveryState::Dead => false,
                }
            });
            Ok(record.map(|record| {
                if record.state == DeliveryState::Attempting {
                    record.possible_duplicate = true;
                }
                claim_record(record, lease_owner, now)
            }))
        })
        .map_err(|error| error.to_string())?
    }

    pub fn complete_outbound_delivery(
        &self,
        delivery_id: &str,
        lease_token: &str,
        external_message_id: Option<&str>,
    ) -> Result<(), String> {
        let now = now_unix_ms();
        self.mutate_outbound_delivery_ledger(|ledger| {
            let record = leased_record_mut(ledger, delivery_id, lease_token)?;
            record.state = DeliveryState::Delivered;
            record.external_message_id = external_message_id.map(bound_external_id);
            record.delivered_at_unix_ms = Some(now);
            record.updated_at_unix_ms = now;
            record.last_error = None;
            clear_lease(record);
            scrub_terminal_payload(record);
            Ok(())
        })
        .map_err(|error| error.to_string())?
    }

    pub fn retry_outbound_delivery(
        &self,
        delivery_id: &str,
        lease_token: &str,
        error: &str,
    ) -> Result<(), String> {
        let now = now_unix_ms();
        self.mutate_outbound_delivery_ledger(|ledger| {
            let record = leased_record_mut(ledger, delivery_id, lease_token)?;
            record.last_error = Some(bound_error(error));
            record.possible_duplicate = true;
            record.updated_at_unix_ms = now;
            clear_lease(record);
            if record.attempt_count >= record.max_attempts {
                record.state = DeliveryState::Dead;
                scrub_terminal_payload(record);
            } else {
                record.state = DeliveryState::Pending;
                record.run_after_unix_ms = now.saturating_add(retry_delay_ms(record.attempt_count));
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?
    }

    pub fn outbound_delivery_snapshot(&self) -> Result<OutboundDeliverySnapshot, String> {
        let ledger = self
            .load_outbound_delivery_ledger()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        let now = now_unix_ms();
        let mut snapshot = OutboundDeliverySnapshot::default();
        for record in &ledger.records {
            match record.state {
                DeliveryState::Pending => snapshot.pending += 1,
                DeliveryState::Attempting => snapshot.attempting += 1,
                DeliveryState::Delivered => snapshot.delivered += 1,
                DeliveryState::Dead => snapshot.dead += 1,
            }
            if record.possible_duplicate
                && matches!(
                    record.state,
                    DeliveryState::Pending | DeliveryState::Attempting
                )
            {
                snapshot.possible_duplicates += 1;
            }
            if matches!(
                record.state,
                DeliveryState::Pending | DeliveryState::Attempting
            ) {
                let age = now.saturating_sub(record.created_at_unix_ms) / 1_000;
                snapshot.oldest_pending_age_secs = Some(
                    snapshot
                        .oldest_pending_age_secs
                        .map_or(age, |current| current.max(age)),
                );
            }
            if record.last_error.is_some() {
                snapshot.last_error = record.last_error.clone();
            }
        }
        Ok(snapshot)
    }

    fn load_outbound_delivery_ledger(&self) -> KernelResult<Option<DeliveryLedger>> {
        let value = self
            .memory
            .structured_get(shared_memory_agent_id(), STATE_KEY)
            .map_err(KernelError::Captain)?;
        value.map(decode_ledger).transpose()
    }

    fn mutate_outbound_delivery_ledger<T>(
        &self,
        mutate: impl FnOnce(&mut DeliveryLedger) -> T,
    ) -> KernelResult<T> {
        let _guard = self
            .outbound_delivery_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut ledger = self.load_outbound_delivery_ledger()?.unwrap_or_default();
        let output = mutate(&mut ledger);
        self.memory
            .structured_set(
                shared_memory_agent_id(),
                STATE_KEY,
                serde_json::to_value(ledger).map_err(|error| {
                    KernelError::Captain(CaptainError::Internal(format!(
                        "Failed to serialize outbound delivery ledger: {error}"
                    )))
                })?,
            )
            .map_err(KernelError::Captain)?;
        Ok(output)
    }
}

fn decode_ledger(value: serde_json::Value) -> KernelResult<DeliveryLedger> {
    let ledger = serde_json::from_value::<DeliveryLedger>(value).map_err(|error| {
        KernelError::Captain(CaptainError::Internal(format!(
            "Invalid persisted outbound delivery ledger: {error}"
        )))
    })?;
    if ledger.schema_version != STATE_SCHEMA_VERSION {
        return Err(KernelError::Captain(CaptainError::Internal(format!(
            "Unsupported outbound delivery schema {} (runtime supports {})",
            ledger.schema_version, STATE_SCHEMA_VERSION
        ))));
    }
    Ok(ledger)
}

fn claim_record(record: &mut DeliveryRecord, lease_owner: &str, now: i64) -> OutboundDeliveryClaim {
    record.state = DeliveryState::Attempting;
    record.attempt_count = record.attempt_count.saturating_add(1);
    record.lease_owner = Some(lease_owner.to_string());
    let lease_token = uuid::Uuid::new_v4().to_string();
    record.lease_token = Some(lease_token.clone());
    record.lease_expires_at_unix_ms = Some(now.saturating_add(LEASE_DURATION_MS));
    record.updated_at_unix_ms = now;
    OutboundDeliveryClaim {
        delivery_id: record.id.clone(),
        lease_token,
        intent: record.intent.clone(),
        attempt_count: record.attempt_count,
        possible_duplicate: record.possible_duplicate,
    }
}

fn leased_record_mut<'a>(
    ledger: &'a mut DeliveryLedger,
    delivery_id: &str,
    lease_token: &str,
) -> Result<&'a mut DeliveryRecord, String> {
    let record = ledger
        .records
        .iter_mut()
        .find(|record| record.id == delivery_id)
        .ok_or_else(|| "outbound delivery no longer exists".to_string())?;
    if record.state != DeliveryState::Attempting
        || record.lease_token.as_deref() != Some(lease_token)
    {
        return Err("outbound delivery lease is stale".to_string());
    }
    Ok(record)
}

fn clear_lease(record: &mut DeliveryRecord) {
    record.lease_owner = None;
    record.lease_token = None;
    record.lease_expires_at_unix_ms = None;
}

fn scrub_terminal_payload(record: &mut DeliveryRecord) {
    record.intent.content = captain_channels::types::ChannelContent::Text(String::new());
    record.intent.recipient.display_name.clear();
    record.intent.recipient.captain_user = None;
}

fn prune_ledger(ledger: &mut DeliveryLedger, now: i64) {
    ledger.records.retain(|record| {
        !matches!(record.state, DeliveryState::Delivered | DeliveryState::Dead)
            || now.saturating_sub(record.updated_at_unix_ms) < TERMINAL_RETENTION_MS
    });
    if ledger.records.len() <= MAX_RECORDS {
        return;
    }
    let to_remove = ledger.records.len() - MAX_RECORDS;
    let mut removed = 0;
    ledger.records.retain(|record| {
        if removed < to_remove
            && matches!(record.state, DeliveryState::Delivered | DeliveryState::Dead)
        {
            removed += 1;
            false
        } else {
            true
        }
    });
}

fn validate_intent(intent: &OutboundDeliveryIntent) -> Result<(), String> {
    if intent.idempotency_key.trim().is_empty() || intent.idempotency_key.len() > 256 {
        return Err("outbound idempotency key must contain 1..=256 bytes".to_string());
    }
    if intent.channel.trim().is_empty() || intent.channel.len() > 64 {
        return Err("outbound channel must contain 1..=64 bytes".to_string());
    }
    if intent.recipient.platform_id.trim().is_empty() {
        return Err("outbound recipient is empty".to_string());
    }
    if serialized_intent_len(intent) > MAX_INTENT_BYTES {
        return Err(format!(
            "outbound delivery exceeds the {} byte durable payload limit",
            MAX_INTENT_BYTES
        ));
    }
    Ok(())
}

fn serialized_intent_len(intent: &OutboundDeliveryIntent) -> usize {
    serde_json::to_vec(intent)
        .map(|payload| payload.len())
        .unwrap_or(MAX_INTENT_BYTES.saturating_add(1))
}

fn retry_delay_ms(attempt_count: u32) -> i64 {
    let exponent = attempt_count.saturating_sub(1).min(6);
    5_000_i64.saturating_mul(1_i64 << exponent)
}

fn bound_error(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(2_048)
        .collect()
}

fn bound_external_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_channels::outbound_delivery::OutboundDeliveryTransport;
    use captain_channels::types::{ChannelContent, ChannelUser};

    fn intent(key: &str) -> OutboundDeliveryIntent {
        OutboundDeliveryIntent {
            idempotency_key: key.to_string(),
            agent_id: None,
            channel: "telegram".to_string(),
            recipient: ChannelUser {
                platform_id: "42".to_string(),
                display_name: "Test".to_string(),
                captain_user: None,
            },
            content: ChannelContent::Text("hello".to_string()),
            transport: OutboundDeliveryTransport::Standard,
            source_message_id: "source-1".to_string(),
            purpose: "agent_final".to_string(),
        }
    }

    #[test]
    fn claim_record_increments_attempt_and_sets_a_lease() {
        let now = 1_000;
        let mut record = DeliveryRecord {
            id: "delivery-1".to_string(),
            intent: intent("key-1"),
            state: DeliveryState::Pending,
            attempt_count: 0,
            max_attempts: MAX_ATTEMPTS,
            run_after_unix_ms: now,
            lease_owner: None,
            lease_token: None,
            lease_expires_at_unix_ms: None,
            possible_duplicate: false,
            last_error: None,
            external_message_id: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            delivered_at_unix_ms: None,
        };

        let claim = claim_record(&mut record, "boot-a", now);
        assert_eq!(claim.attempt_count, 1);
        assert_eq!(record.state, DeliveryState::Attempting);
        assert_eq!(record.lease_owner.as_deref(), Some("boot-a"));
        assert!(!claim.lease_token.is_empty());
    }

    #[test]
    fn prune_never_discards_live_delivery_records() {
        let now = TERMINAL_RETENTION_MS + 1;
        let base = DeliveryRecord {
            id: "delivery-1".to_string(),
            intent: intent("key-1"),
            state: DeliveryState::Pending,
            attempt_count: 0,
            max_attempts: MAX_ATTEMPTS,
            run_after_unix_ms: 0,
            lease_owner: None,
            lease_token: None,
            lease_expires_at_unix_ms: None,
            possible_duplicate: false,
            last_error: None,
            external_message_id: None,
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            delivered_at_unix_ms: None,
        };
        let mut ledger = DeliveryLedger {
            records: vec![
                base.clone(),
                DeliveryRecord {
                    id: "terminal".to_string(),
                    state: DeliveryState::Delivered,
                    ..base
                },
            ],
            ..Default::default()
        };

        prune_ledger(&mut ledger, now);
        assert_eq!(ledger.records.len(), 1);
        assert_eq!(ledger.records[0].state, DeliveryState::Pending);
    }

    #[test]
    fn terminal_records_drop_response_content_and_display_identity() {
        let mut record = DeliveryRecord {
            id: "delivery-1".to_string(),
            intent: intent("key-1"),
            state: DeliveryState::Delivered,
            attempt_count: 1,
            max_attempts: MAX_ATTEMPTS,
            run_after_unix_ms: 0,
            lease_owner: None,
            lease_token: None,
            lease_expires_at_unix_ms: None,
            possible_duplicate: false,
            last_error: None,
            external_message_id: None,
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            delivered_at_unix_ms: Some(0),
        };

        scrub_terminal_payload(&mut record);

        assert!(matches!(
            record.intent.content,
            ChannelContent::Text(ref text) if text.is_empty()
        ));
        assert!(record.intent.recipient.display_name.is_empty());
        assert!(record.intent.recipient.captain_user.is_none());
        assert_eq!(record.intent.recipient.platform_id, "42");
    }
}
