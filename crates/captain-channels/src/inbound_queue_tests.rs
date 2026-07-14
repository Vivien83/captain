use crate::inbound_queue::InboundSessionQueue;
use crate::inbound_queue_types::{
    InboundStart, PendingInboundSummary, PendingMergeKind, INBOUND_DEAD_LETTER_RETENTION_SECS,
    MAX_RECOVERED_INBOUND_ATTEMPTS,
};
use crate::types::{ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use captain_types::agent::AgentId;
use chrono::{Duration as ChronoDuration, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn message(platform_message_id: &str, text: &str) -> ChannelMessage {
    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: platform_message_id.to_string(),
        sender: ChannelUser {
            platform_id: "chat-1".to_string(),
            display_name: "Alex".to_string(),
            captain_user: None,
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: Some("topic-1".to_string()),
        metadata: HashMap::new(),
    }
}

fn temp_queue_path() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "captain-inbound-queue-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("channel_inbound_queue.json")
}

#[test]
fn starts_once_then_queues_followups() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "hello");
    let key = queue.session_key(&first, "user-1");

    assert_eq!(
        queue.start_or_queue(key.clone(), first),
        InboundStart::Started { key: key.clone() }
    );

    let queued = queue.start_or_queue(key, message("2", "again"));
    assert_eq!(
        queued,
        InboundStart::Queued(PendingInboundSummary {
            queued_count: 1,
            merge_kind: PendingMergeKind::Inserted,
            ack_recommended: true,
        })
    );
    assert_eq!(queue.active_len(), 1);
}

#[test]
fn appends_text_followups_into_one_pending_turn() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "start");
    let key = queue.session_key(&first, "user-1");
    assert!(matches!(
        queue.start_or_queue(key.clone(), first),
        InboundStart::Started { .. }
    ));

    queue.start_or_queue(key.clone(), message("2", "second"));
    let queued = queue.start_or_queue(key.clone(), message("3", "third"));
    assert_eq!(
        queued,
        InboundStart::Queued(PendingInboundSummary {
            queued_count: 2,
            merge_kind: PendingMergeKind::AppendedText,
            ack_recommended: false,
        })
    );

    let pending = queue.next_or_finish(&key).expect("pending message");
    assert_eq!(pending.queued_count, 2);
    assert!(
        matches!(pending.message.content, ChannelContent::Text(ref text) if text == "second\nthird")
    );
    assert_eq!(queue.active_len(), 1, "session remains active for drain");
    assert!(queue.next_or_finish(&key).is_none());
    assert_eq!(queue.active_len(), 0);
}

#[test]
fn clear_removes_active_session_and_pending_message() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "start");
    let key = queue.session_key(&first, "user-1");
    assert!(matches!(
        queue.start_or_queue(key.clone(), first),
        InboundStart::Started { .. }
    ));
    queue.start_or_queue(key.clone(), message("2", "queued"));

    queue.clear(&key);

    assert_eq!(queue.active_len(), 0);
    assert_eq!(
        queue.start_or_queue(key.clone(), message("3", "fresh")),
        InboundStart::Started { key }
    );
}

#[test]
fn tracks_active_agent_for_running_session() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "start");
    let key = queue.session_key(&first, "user-1");
    let agent_id = AgentId::new();

    assert!(matches!(
        queue.start_or_queue(key.clone(), first),
        InboundStart::Started { .. }
    ));
    assert_eq!(queue.active_agent(&key), None);

    queue.set_active_agent(&key, agent_id);

    assert_eq!(queue.active_agent(&key), Some(agent_id));
    assert!(queue.should_ack_active_interjection(&key));
    assert!(!queue.should_ack_active_interjection(&key));
    queue.clear(&key);
    assert_eq!(queue.active_agent(&key), None);
}

#[test]
fn queued_ack_is_debounced_per_session() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "start");
    let key = queue.session_key(&first, "user-1");
    let now = Instant::now();
    assert!(matches!(
        queue.start_or_queue_at(key.clone(), first, now),
        InboundStart::Started { .. }
    ));

    let first_queued = queue.start_or_queue_at(
        key.clone(),
        message("2", "queued"),
        now + Duration::from_secs(1),
    );
    assert!(matches!(
        first_queued,
        InboundStart::Queued(PendingInboundSummary {
            ack_recommended: true,
            ..
        })
    ));

    let burst = queue.start_or_queue_at(
        key.clone(),
        message("3", "burst"),
        now + Duration::from_secs(2),
    );
    assert!(matches!(
        burst,
        InboundStart::Queued(PendingInboundSummary {
            ack_recommended: false,
            ..
        })
    ));

    let later = queue.start_or_queue_at(key, message("4", "later"), now + Duration::from_secs(32));
    assert!(matches!(
        later,
        InboundStart::Queued(PendingInboundSummary {
            ack_recommended: true,
            ..
        })
    ));
}

#[test]
fn snapshot_reports_counts_without_session_keys() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "start");
    let key = queue.session_key(&first, "user-1");
    queue.start_or_queue(key.clone(), first);

    queue.start_or_queue(key.clone(), message("2", "queued"));
    queue.start_or_queue(key, message("3", "queued again"));

    let snapshot = queue.snapshot();
    assert_eq!(snapshot.active_sessions, 1);
    assert_eq!(snapshot.pending_sessions, 1);
    assert_eq!(snapshot.pending_messages, 2);
    assert_eq!(snapshot.channels.len(), 1);
    assert_eq!(snapshot.channels[0].channel, "telegram");
    assert_eq!(snapshot.channels[0].pending_messages, 2);
}

#[test]
fn snapshot_reports_interjections_without_session_keys() {
    let queue = InboundSessionQueue::default();
    let first = message("1", "start");
    let key = queue.session_key(&first, "user-1");
    queue.start_or_queue(key.clone(), first);

    queue.record_interjection(&key);
    queue.record_interjection(&key);

    let snapshot = queue.snapshot();
    assert_eq!(snapshot.active_sessions, 1);
    assert_eq!(snapshot.pending_sessions, 0);
    assert_eq!(snapshot.pending_messages, 0);
    assert_eq!(snapshot.interjected_sessions, 1);
    assert_eq!(snapshot.interjected_messages, 2);
    assert_eq!(snapshot.channels.len(), 1);
    assert_eq!(snapshot.channels[0].channel, "telegram");
    assert_eq!(snapshot.channels[0].interjected_sessions, 1);
    assert_eq!(snapshot.channels[0].interjected_messages, 2);
}

#[test]
fn persisted_pending_survives_restart_until_dispatch_finishes() {
    let path = temp_queue_path();
    let first = message("1", "start");
    let key = InboundSessionQueue::default().session_key(&first, "user-1");

    let queue = InboundSessionQueue::with_persistence(path.clone());
    queue.start_or_queue(key.clone(), first);
    queue.start_or_queue(key.clone(), message("2", "queued"));
    drop(queue);

    let recovered_queue = InboundSessionQueue::with_persistence(path.clone());
    let snapshot = recovered_queue.snapshot();
    assert_eq!(snapshot.pending_sessions, 1);
    assert_eq!(snapshot.pending_messages, 1);
    assert_eq!(snapshot.inflight_sessions, 0);
    assert_eq!(snapshot.inflight_messages, 0);

    let recovered = recovered_queue.recover_pending_for_channel("telegram");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].0, key);
    assert_eq!(recovered[0].1.queued_count, 1);
    assert!(
        matches!(recovered[0].1.message.content, ChannelContent::Text(ref text) if text == "queued")
    );
    assert_eq!(recovered_queue.active_len(), 1);
    let recovered_snapshot = recovered_queue.snapshot();
    assert_eq!(recovered_snapshot.pending_messages, 0);
    assert_eq!(recovered_snapshot.inflight_sessions, 1);
    assert_eq!(recovered_snapshot.inflight_messages, 1);
    assert_eq!(recovered_snapshot.channels[0].inflight_messages, 1);

    let retry_after_crash = InboundSessionQueue::with_persistence(path.clone());
    let recovered_again = retry_after_crash.recover_pending_for_channel("telegram");
    assert_eq!(recovered_again.len(), 1);
    assert_eq!(recovered_again[0].0, key);

    assert!(recovered_queue.next_or_finish(&key).is_none());
    let empty_after_finish = InboundSessionQueue::with_persistence(path.clone());
    assert_eq!(
        empty_after_finish
            .recover_pending_for_channel("telegram")
            .len(),
        0
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn recovered_pending_dead_letters_after_repeated_crashes() {
    let path = temp_queue_path();
    let first = message("1", "start");
    let key = InboundSessionQueue::default().session_key(&first, "user-1");

    let queue = InboundSessionQueue::with_persistence(path.clone());
    queue.start_or_queue(key.clone(), first);
    queue.start_or_queue(key.clone(), message("2", "queued"));
    drop(queue);

    for _ in 0..MAX_RECOVERED_INBOUND_ATTEMPTS {
        let retry_queue = InboundSessionQueue::with_persistence(path.clone());
        let recovered = retry_queue.recover_pending_for_channel("telegram");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].0, key);
        drop(retry_queue);
    }

    let exhausted_queue = InboundSessionQueue::with_persistence(path.clone());
    assert_eq!(
        exhausted_queue
            .recover_pending_for_channel("telegram")
            .len(),
        0
    );
    let snapshot = exhausted_queue.snapshot();
    assert_eq!(snapshot.active_sessions, 0);
    assert_eq!(snapshot.inflight_messages, 0);
    assert_eq!(snapshot.dead_letter_sessions, 1);
    assert_eq!(snapshot.dead_letter_messages, 1);
    assert!(snapshot.dead_letter_oldest_age_secs.unwrap_or(99) <= 1);
    assert_eq!(snapshot.channels[0].dead_letter_messages, 1);
    assert!(
        snapshot.channels[0]
            .dead_letter_oldest_age_secs
            .unwrap_or(99)
            <= 1
    );

    let persisted_queue = InboundSessionQueue::with_persistence(path.clone());
    assert_eq!(persisted_queue.snapshot().dead_letter_messages, 1);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn stale_dead_letters_are_pruned_on_restart() {
    let path = temp_queue_path();
    let first = message("1", "start");
    let key = InboundSessionQueue::default().session_key(&first, "user-1");

    let queue = InboundSessionQueue::with_persistence(path.clone());
    queue.start_or_queue(key.clone(), first);
    queue.start_or_queue(key, message("2", "queued"));
    drop(queue);

    for _ in 0..=MAX_RECOVERED_INBOUND_ATTEMPTS {
        let retry_queue = InboundSessionQueue::with_persistence(path.clone());
        let _ = retry_queue.recover_pending_for_channel("telegram");
        drop(retry_queue);
    }

    let raw = std::fs::read_to_string(&path).unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let stale_at = Utc::now() - ChronoDuration::seconds(INBOUND_DEAD_LETTER_RETENTION_SECS + 60);
    payload["pending"][0]["dead_letter"]["dead_lettered_at"] =
        serde_json::json!(stale_at.to_rfc3339());
    std::fs::write(&path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();

    let pruned_queue = InboundSessionQueue::with_persistence(path.clone());
    assert_eq!(pruned_queue.snapshot().dead_letter_messages, 0);
    assert_eq!(pruned_queue.active_len(), 0);
    assert!(
        std::fs::read_to_string(&path).is_err(),
        "empty durable queue should remove the store file"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn dead_letters_can_be_cleared_without_exposing_content() {
    let path = temp_queue_path();
    let first = message("1", "start");
    let key = InboundSessionQueue::default().session_key(&first, "user-1");

    let queue = InboundSessionQueue::with_persistence(path.clone());
    queue.start_or_queue(key.clone(), first);
    queue.start_or_queue(key, message("2", "queued"));
    drop(queue);

    for _ in 0..=MAX_RECOVERED_INBOUND_ATTEMPTS {
        let retry_queue = InboundSessionQueue::with_persistence(path.clone());
        let _ = retry_queue.recover_pending_for_channel("telegram");
        drop(retry_queue);
    }

    let dead_queue = InboundSessionQueue::with_persistence(path.clone());
    assert_eq!(dead_queue.snapshot().dead_letter_messages, 1);

    let (sessions, messages) = dead_queue.clear_dead_letters(None);

    assert_eq!(sessions, 1);
    assert_eq!(messages, 1);
    assert_eq!(dead_queue.snapshot().dead_letter_messages, 0);
    assert_eq!(dead_queue.active_len(), 0);
    assert!(
        std::fs::read_to_string(&path).is_err(),
        "cleared durable dead letters should remove the empty store"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
