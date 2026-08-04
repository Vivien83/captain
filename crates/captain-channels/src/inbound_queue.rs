//! Inbound per-session queue for channel follow-ups.

use crate::inbound_queue_snapshot::{InboundQueueChannelSnapshot, InboundQueueSnapshot};
use crate::inbound_queue_state::SessionState;
use crate::inbound_queue_store::{DeadInboundRecord, InboundQueueStore, PendingInboundRecord};
use crate::inbound_queue_types::{
    AcceptedInboundId, InboundStart, PendingInboundMessage, PendingInboundSummary,
    PendingMergeKind, DURABLE_INGRESS_ID_METADATA_KEY, INBOUND_ACCEPTED_ID_RETENTION_SECS,
    MAX_RECOVERED_INBOUND_ATTEMPTS,
};
use crate::types::{
    ChannelContent, ChannelMessage, ChannelType, INTERNAL_ADAPTER_NAME_METADATA_KEY,
};
use captain_types::agent::AgentId;
use chrono::Utc;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tracing::warn;

const QUEUED_ACK_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_DURABLE_PENDING_SESSIONS: usize = 128;
const MAX_ACCEPTED_INGRESS_IDS_PER_SESSION: usize = 256;

#[derive(Debug, Clone, Default)]
pub(crate) struct InboundSessionQueue {
    inner: Arc<Mutex<HashMap<String, SessionState>>>,
    store: Option<Arc<InboundQueueStore>>,
}

impl InboundSessionQueue {
    pub(crate) fn with_persistence(path: PathBuf) -> Self {
        let store = Arc::new(InboundQueueStore::new(path, MAX_DURABLE_PENDING_SESSIONS));
        let sessions = records_to_sessions(store.load_records());
        Self {
            inner: Arc::new(Mutex::new(sessions)),
            store: Some(store),
        }
    }

    pub(crate) fn session_key(&self, message: &ChannelMessage, sender_user_id: &str) -> String {
        let thread_id = message.thread_id.as_deref().unwrap_or("-");
        let captain_user = message.sender.captain_user.as_deref().unwrap_or("-");
        let account_scope = message
            .metadata
            .get("account_id")
            .and_then(serde_json::Value::as_str)
            .filter(|account| !account.is_empty())
            .map(|account| format!("|account:{account}"))
            .unwrap_or_default();
        format!(
            "{}{}|chat:{}|user:{}|captain:{}|thread:{}",
            channel_key(&message.channel),
            account_scope,
            message.sender.platform_id,
            sender_user_id,
            captain_user,
            thread_id
        )
    }

    pub(crate) fn start_or_queue(&self, key: String, message: ChannelMessage) -> InboundStart {
        self.start_or_queue_at(key, message, Instant::now())
    }

    pub(crate) fn start_or_queue_at(
        &self,
        key: String,
        message: ChannelMessage,
        now: Instant,
    ) -> InboundStart {
        let durable_id = durable_ingress_id(&message).map(str::to_string);
        if durable_id.is_some() && self.store.is_none() {
            return InboundStart::Rejected {
                reason: "durable inbound queue is unavailable".to_string(),
            };
        }

        let mut sessions = self.lock_sessions();
        let previous = self.store.as_ref().map(|_| sessions.clone());
        prune_accepted_ingress(&mut sessions);

        if let Some(durable_id) = durable_id.as_deref() {
            if sessions.values().any(|state| {
                state
                    .accepted_ingress
                    .iter()
                    .any(|accepted| accepted.id == durable_id)
            }) {
                return InboundStart::Duplicate { key };
            }
        }

        let start = match sessions.get_mut(&key) {
            Some(state) if is_dead_letter_only(state) || is_idle_tombstone(state) => {
                let accepted_ingress = std::mem::take(&mut state.accepted_ingress);
                *state = new_active_state(message, durable_id);
                state.accepted_ingress.extend(accepted_ingress);
                trim_accepted_ingress(&mut state.accepted_ingress);
                InboundStart::Started { key }
            }
            Some(state) => {
                remember_accepted_ingress(state, durable_id);
                InboundStart::Queued(queue_pending(state, message, now))
            }
            None => {
                sessions.insert(key.clone(), new_active_state(message, durable_id));
                InboundStart::Started { key }
            }
        };
        let records = self.pending_records(&sessions);
        if let Err(reason) = self.persist_pending_result(records) {
            if let Some(previous) = previous {
                *sessions = previous;
            }
            return InboundStart::Rejected { reason };
        }
        start
    }

    pub(crate) fn active_agent(&self, key: &str) -> Option<AgentId> {
        self.lock_sessions()
            .get(key)
            .and_then(|state| state.active_agent_id)
    }

    pub(crate) fn set_active_agent(&self, key: &str, agent_id: AgentId) {
        if let Some(state) = self.lock_sessions().get_mut(key) {
            state.active_agent_id = Some(agent_id);
        }
    }

    pub(crate) fn should_ack_active_interjection(&self, key: &str) -> bool {
        let mut sessions = self.lock_sessions();
        let Some(state) = sessions.get_mut(key) else {
            return false;
        };
        should_send_queued_ack(state, Instant::now())
    }

    pub(crate) fn record_interjection(&self, key: &str) {
        if let Some(state) = self.lock_sessions().get_mut(key) {
            state.interjected_count = state.interjected_count.saturating_add(1);
        }
    }

    pub(crate) fn next_or_finish(&self, key: &str) -> Option<PendingInboundMessage> {
        let mut sessions = self.lock_sessions();
        let previous = self.store.as_ref().map(|_| sessions.clone());
        let state = sessions.get_mut(key)?;
        state.inflight = None;
        state.active_agent_id = None;
        state.recovery_attempts = 0;
        let next = if let Some(pending) = state.pending.take() {
            state.inflight = Some(pending.clone());
            Some(pending)
        } else {
            if state.accepted_ingress.is_empty() && state.dead_letter.is_none() {
                sessions.remove(key);
            }
            None
        };
        let records = self.pending_records(&sessions);
        if let Err(reason) = self.persist_pending_result(records) {
            if let Some(previous) = previous {
                *sessions = previous;
            }
            warn!(key, %reason, "Failed to commit inbound channel completion");
            return None;
        }
        next
    }

    pub(crate) fn abandon_for_recovery(&self, key: &str) {
        if let Some(state) = self.lock_sessions().get_mut(key) {
            state.active_agent_id = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn clear(&self, key: &str) {
        let records = {
            let mut sessions = self.lock_sessions();
            sessions.remove(key);
            self.pending_records(&sessions)
        };
        self.persist_pending(records);
    }

    pub(crate) fn clear_dead_letters(&self, channel: Option<&str>) -> (usize, usize) {
        let ((sessions_cleared, messages_cleared), records) = {
            let mut sessions = self.lock_sessions();
            let mut sessions_cleared = 0usize;
            let mut messages_cleared = 0usize;
            sessions.retain(|_, state| {
                if channel
                    .map(|wanted| state.channel != wanted)
                    .unwrap_or(false)
                {
                    return true;
                }
                let Some(dead_letter) = state.dead_letter.take() else {
                    return true;
                };
                sessions_cleared += 1;
                messages_cleared += dead_letter.message.queued_count;
                state.pending.is_some()
                    || state.inflight.is_some()
                    || !state.accepted_ingress.is_empty()
            });
            (
                (sessions_cleared, messages_cleared),
                self.pending_records(&sessions),
            )
        };
        self.persist_pending(records);
        (sessions_cleared, messages_cleared)
    }

    pub(crate) fn recover_pending_for_channel(
        &self,
        channel: &str,
    ) -> Vec<(String, PendingInboundMessage)> {
        let (recovered, records) = {
            let mut sessions = self.lock_sessions();
            let mut keys: Vec<String> = sessions
                .iter()
                .filter(|(_, state)| {
                    state.channel == channel
                        && (state.pending.is_some() || state.inflight.is_some())
                })
                .map(|(key, _)| key.clone())
                .collect();
            keys.sort();

            let mut recovered = Vec::new();
            for key in keys {
                if let Some(state) = sessions.get_mut(&key) {
                    let recovered_message = match state.inflight.clone() {
                        Some(inflight) => Some(inflight),
                        None => {
                            let Some(pending) = state.pending.take() else {
                                continue;
                            };
                            state.inflight = Some(pending.clone());
                            Some(pending)
                        }
                    };
                    if let Some(pending) = recovered_message {
                        state.recovery_attempts = state.recovery_attempts.saturating_add(1);
                        if state.recovery_attempts > MAX_RECOVERED_INBOUND_ATTEMPTS {
                            state.dead_letter = Some(DeadInboundRecord {
                                message: pending,
                                recovery_attempts: state.recovery_attempts,
                                reason: "recovered_inbound_dispatch_not_completed".to_string(),
                                dead_lettered_at: Utc::now(),
                            });
                            state.pending = None;
                            state.inflight = None;
                            state.active_agent_id = None;
                            warn!(
                                channel,
                                attempts = state.recovery_attempts,
                                "Moved recovered inbound follow-up to dead letter"
                            );
                        } else {
                            recovered.push((key, pending));
                        }
                    };
                }
            }
            (recovered, self.pending_records(&sessions))
        };
        self.persist_pending(records);
        recovered
    }

    pub(crate) fn snapshot(&self) -> InboundQueueSnapshot {
        let sessions = self.lock_sessions();
        let mut snapshot = InboundQueueSnapshot::default();
        let mut channels: BTreeMap<String, InboundQueueChannelSnapshot> = BTreeMap::new();

        for state in sessions.values() {
            let active = usize::from(state.inflight.is_some() || state.pending.is_some());
            snapshot.active_sessions += active;
            let pending_messages = state
                .pending
                .as_ref()
                .map(|pending| pending.queued_count)
                .unwrap_or(0);
            if pending_messages > 0 {
                snapshot.pending_sessions += 1;
                snapshot.pending_messages += pending_messages;
            }
            let inflight_messages = state
                .inflight
                .as_ref()
                .map(|pending| pending.queued_count)
                .unwrap_or(0);
            if inflight_messages > 0 {
                snapshot.inflight_sessions += 1;
                snapshot.inflight_messages += inflight_messages;
            }
            let dead_letter_messages = state
                .dead_letter
                .as_ref()
                .map(|dead| dead.message.queued_count)
                .unwrap_or(0);
            if dead_letter_messages > 0 {
                snapshot.dead_letter_sessions += 1;
                snapshot.dead_letter_messages += dead_letter_messages;
                update_oldest_age(
                    &mut snapshot.dead_letter_oldest_age_secs,
                    state.dead_letter.as_ref(),
                );
            }
            if state.interjected_count > 0 {
                snapshot.interjected_sessions += 1;
                snapshot.interjected_messages += state.interjected_count;
            }

            let channel = channels.entry(state.channel.clone()).or_insert_with(|| {
                InboundQueueChannelSnapshot {
                    channel: state.channel.clone(),
                    ..Default::default()
                }
            });
            channel.active_sessions += active;
            if pending_messages > 0 {
                channel.pending_sessions += 1;
                channel.pending_messages += pending_messages;
            }
            if inflight_messages > 0 {
                channel.inflight_sessions += 1;
                channel.inflight_messages += inflight_messages;
            }
            if dead_letter_messages > 0 {
                channel.dead_letter_sessions += 1;
                channel.dead_letter_messages += dead_letter_messages;
                update_oldest_age(
                    &mut channel.dead_letter_oldest_age_secs,
                    state.dead_letter.as_ref(),
                );
            }
            if state.interjected_count > 0 {
                channel.interjected_sessions += 1;
                channel.interjected_messages += state.interjected_count;
            }
        }

        snapshot.channels = channels.into_values().collect();
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn active_len(&self) -> usize {
        self.snapshot().active_sessions
    }

    fn lock_sessions(&self) -> MutexGuard<'_, HashMap<String, SessionState>> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn pending_records(
        &self,
        sessions: &HashMap<String, SessionState>,
    ) -> Option<Vec<PendingInboundRecord>> {
        self.store.as_ref()?;
        let mut records: Vec<PendingInboundRecord> = sessions
            .iter()
            .filter_map(|(key, state)| {
                if state.pending.is_none()
                    && state.inflight.is_none()
                    && state.dead_letter.is_none()
                    && state.accepted_ingress.is_empty()
                {
                    return None;
                }
                Some(PendingInboundRecord {
                    key: key.clone(),
                    channel: state.channel.clone(),
                    recovery_attempts: state.recovery_attempts,
                    pending: state.pending.clone(),
                    inflight: state.inflight.clone(),
                    dead_letter: state.dead_letter.clone(),
                    accepted_ingress: state.accepted_ingress.clone(),
                })
            })
            .collect();
        records.sort_by(|a, b| {
            pending_record_priority(a)
                .cmp(&pending_record_priority(b))
                .then_with(|| a.key.cmp(&b.key))
        });
        Some(records)
    }

    fn persist_pending(&self, records: Option<Vec<PendingInboundRecord>>) {
        if let Err(err) = self.persist_pending_result(records) {
            warn!("Failed to persist inbound channel queue: {err}");
        }
    }

    fn persist_pending_result(
        &self,
        records: Option<Vec<PendingInboundRecord>>,
    ) -> Result<(), String> {
        let (Some(store), Some(records)) = (self.store.as_ref(), records) else {
            return Ok(());
        };
        store
            .save_records(&records)
            .map_err(|err| format!("failed to persist inbound channel queue: {err}"))
    }
}

fn pending_record_priority(record: &PendingInboundRecord) -> u8 {
    if record.inflight.is_some() || record.pending.is_some() {
        0
    } else if record.dead_letter.is_some() {
        1
    } else {
        2
    }
}

fn records_to_sessions(records: Vec<PendingInboundRecord>) -> HashMap<String, SessionState> {
    records
        .into_iter()
        .filter_map(|record| {
            if record.pending.is_none()
                && record.inflight.is_none()
                && record.dead_letter.is_none()
                && record.accepted_ingress.is_empty()
            {
                return None;
            }
            let message = record
                .pending
                .as_ref()
                .or(record.inflight.as_ref())
                .or(record.dead_letter.as_ref().map(|dead| &dead.message));
            let channel = if record.channel.is_empty() {
                channel_key(&message?.message.channel)
            } else {
                record.channel
            };
            Some((
                record.key,
                SessionState {
                    channel,
                    active_agent_id: None,
                    pending: record.pending,
                    inflight: record.inflight,
                    dead_letter: record.dead_letter,
                    accepted_ingress: record.accepted_ingress,
                    recovery_attempts: record.recovery_attempts,
                    interjected_count: 0,
                    last_ack_at: None,
                },
            ))
        })
        .collect()
}

fn queue_pending(
    state: &mut SessionState,
    message: ChannelMessage,
    now: Instant,
) -> PendingInboundSummary {
    let ack_recommended = should_send_queued_ack(state, now);
    match state.pending.as_mut() {
        Some(existing) => {
            let merge_kind = merge_pending(existing, message);
            PendingInboundSummary {
                queued_count: existing.queued_count,
                merge_kind,
                ack_recommended,
            }
        }
        None => {
            state.pending = Some(PendingInboundMessage {
                message,
                queued_count: 1,
            });
            state.dead_letter = None;
            PendingInboundSummary {
                queued_count: 1,
                merge_kind: PendingMergeKind::Inserted,
                ack_recommended,
            }
        }
    }
}

fn should_send_queued_ack(state: &mut SessionState, now: Instant) -> bool {
    let should_ack = state
        .last_ack_at
        .map(|last_ack| now.saturating_duration_since(last_ack) >= QUEUED_ACK_COOLDOWN)
        .unwrap_or(true);
    if should_ack {
        state.last_ack_at = Some(now);
    }
    should_ack
}

fn merge_pending(
    existing: &mut PendingInboundMessage,
    incoming: ChannelMessage,
) -> PendingMergeKind {
    existing.queued_count += 1;

    if append_text(&mut existing.message, &incoming) {
        refresh_envelope(&mut existing.message, incoming);
        return PendingMergeKind::AppendedText;
    }

    existing.message = incoming;
    PendingMergeKind::Replaced
}

fn append_text(existing: &mut ChannelMessage, incoming: &ChannelMessage) -> bool {
    let (ChannelContent::Text(current), ChannelContent::Text(next)) =
        (&mut existing.content, &incoming.content)
    else {
        return false;
    };

    if next.trim().is_empty() {
        return true;
    }

    if current.trim().is_empty() {
        *current = next.clone();
    } else {
        current.push('\n');
        current.push_str(next);
    }
    true
}

fn refresh_envelope(existing: &mut ChannelMessage, incoming: ChannelMessage) {
    existing.platform_message_id = incoming.platform_message_id;
    existing.target_agent = incoming.target_agent;
    existing.timestamp = incoming.timestamp;
    existing.is_group = incoming.is_group;
    existing.thread_id = incoming.thread_id;
    existing.metadata = incoming.metadata;
}

fn new_active_state(message: ChannelMessage, durable_id: Option<String>) -> SessionState {
    let channel = inbound_adapter_key(&message);
    SessionState {
        channel,
        active_agent_id: None,
        pending: None,
        inflight: Some(PendingInboundMessage {
            message,
            queued_count: 1,
        }),
        dead_letter: None,
        accepted_ingress: durable_id
            .map(|id| {
                vec![AcceptedInboundId {
                    id,
                    accepted_at: Utc::now(),
                }]
            })
            .unwrap_or_default(),
        recovery_attempts: 0,
        interjected_count: 0,
        last_ack_at: None,
    }
}

fn inbound_adapter_key(message: &ChannelMessage) -> String {
    message
        .metadata
        .get(INTERNAL_ADAPTER_NAME_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| channel_key(&message.channel))
}

fn is_dead_letter_only(state: &SessionState) -> bool {
    state.dead_letter.is_some() && state.pending.is_none() && state.inflight.is_none()
}

fn is_idle_tombstone(state: &SessionState) -> bool {
    state.dead_letter.is_none() && state.pending.is_none() && state.inflight.is_none()
}

fn durable_ingress_id(message: &ChannelMessage) -> Option<&str> {
    message
        .metadata
        .get(DURABLE_INGRESS_ID_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
}

fn remember_accepted_ingress(state: &mut SessionState, durable_id: Option<String>) {
    let Some(id) = durable_id else {
        return;
    };
    state.accepted_ingress.push(AcceptedInboundId {
        id,
        accepted_at: Utc::now(),
    });
    trim_accepted_ingress(&mut state.accepted_ingress);
}

fn trim_accepted_ingress(accepted: &mut Vec<AcceptedInboundId>) {
    accepted.sort_by_key(|entry| entry.accepted_at);
    if accepted.len() > MAX_ACCEPTED_INGRESS_IDS_PER_SESSION {
        accepted.drain(..accepted.len() - MAX_ACCEPTED_INGRESS_IDS_PER_SESSION);
    }
}

fn prune_accepted_ingress(sessions: &mut HashMap<String, SessionState>) {
    let now = Utc::now();
    sessions.retain(|_, state| {
        state.accepted_ingress.retain(|accepted| {
            now.signed_duration_since(accepted.accepted_at)
                .num_seconds()
                < INBOUND_ACCEPTED_ID_RETENTION_SECS
        });
        state.pending.is_some()
            || state.inflight.is_some()
            || state.dead_letter.is_some()
            || !state.accepted_ingress.is_empty()
    });
}

fn update_oldest_age(target: &mut Option<u64>, dead_letter: Option<&DeadInboundRecord>) {
    let Some(dead_letter) = dead_letter else {
        return;
    };
    let age = Utc::now()
        .signed_duration_since(dead_letter.dead_lettered_at)
        .num_seconds()
        .max(0) as u64;
    *target = Some(target.map_or(age, |current| current.max(age)));
}

fn channel_key(channel: &ChannelType) -> String {
    match channel {
        ChannelType::Custom(name) => format!("custom:{name}"),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}
