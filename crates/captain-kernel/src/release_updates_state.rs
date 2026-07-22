//! Pure durable state transitions for runtime update discovery and delivery.

use std::collections::BTreeSet;

use captain_types::release_update::{
    is_prerelease_version, release_lookup_token, ReleaseDescriptor, RuntimeUpdateCard,
    RuntimeUpdateInstallMode, RuntimeUpdateNoticeKind,
};
use captain_types::version::canonical_version;

use super::{
    RuntimeUpdateCandidate, RuntimeUpdateDecisionRecord, RuntimeUpdateOutbox,
    RuntimeUpdateOutboxStatus, RuntimeUpdateState, CHECK_INTERVAL_MS, FIRST_FAILURE_RETRY_MS,
    MAX_HISTORY, OUTBOX_LEASE_MS, OUTBOX_MAX_ATTEMPTS, UPDATE_ATTEMPT_STALE_MS,
};

pub(super) fn reconcile_scan_success(
    state: &mut RuntimeUpdateState,
    current: &str,
    release: Option<&ReleaseDescriptor>,
    install_mode: RuntimeUpdateInstallMode,
    now: i64,
) {
    state.last_checked_at_unix_ms = Some(now);
    state.last_success_at_unix_ms = Some(now);
    state.next_check_at_unix_ms = now.saturating_add(CHECK_INTERVAL_MS);
    state.last_error = None;
    state.consecutive_failures = 0;

    let Some(release) = release else {
        if state.active_attempt.is_none() {
            state.pending = None;
            suppress_actionable_outbox(state);
        }
        return;
    };
    let version = canonical_version(&release.tag_name).to_string();
    if decision_suppresses_version(state, &version) {
        if state
            .pending
            .as_ref()
            .is_some_and(|candidate| candidate.available_version == version)
        {
            state.pending = None;
            suppress_actionable_outbox(state);
        }
        return;
    }
    if state
        .pending
        .as_ref()
        .is_some_and(|candidate| candidate.available_version == version)
    {
        let should_requeue = {
            let candidate = state.pending.as_mut().expect("candidate checked above");
            candidate.current_version = current.to_string();
            candidate.release_tag = release.tag_name.clone();
            candidate.release_url = release.html_url.clone();
            candidate.published_at = release.published_at.clone();
            candidate.install_mode = install_mode;
            state.active_attempt.is_none()
                && candidate
                    .deferred_until_unix_ms
                    .is_none_or(|deferred_until| deferred_until <= now)
                && !candidate_has_live_or_delivered_notice(&state.outbox, candidate)
        };
        if should_requeue {
            let decision_version = allocate_decision_version(state);
            let candidate = state.pending.as_mut().expect("candidate checked above");
            candidate.decision_version = decision_version;
            let candidate = candidate.clone();
            enqueue_notice(
                state,
                &candidate,
                RuntimeUpdateNoticeKind::Available,
                None,
                now,
            );
        }
        return;
    }
    if state.active_attempt.is_some() {
        return;
    }

    suppress_actionable_outbox(state);
    let decision_version = allocate_decision_version(state);
    let candidate = RuntimeUpdateCandidate {
        token: release_lookup_token(&version),
        decision_version,
        current_version: current.to_string(),
        available_version: version,
        release_tag: release.tag_name.clone(),
        release_url: release.html_url.clone(),
        published_at: release.published_at.clone(),
        prerelease: release.prerelease || is_prerelease_version(&release.tag_name).unwrap_or(false),
        install_mode,
        discovered_at_unix_ms: now,
        deferred_until_unix_ms: None,
    };
    state.pending = Some(candidate.clone());
    enqueue_notice(
        state,
        &candidate,
        RuntimeUpdateNoticeKind::Available,
        None,
        now,
    );
}

fn candidate_has_live_or_delivered_notice(
    outbox: &[RuntimeUpdateOutbox],
    candidate: &RuntimeUpdateCandidate,
) -> bool {
    outbox.iter().any(|item| {
        item.card.token == candidate.token
            && item.card.decision_version == candidate.decision_version
            && matches!(
                item.status,
                RuntimeUpdateOutboxStatus::Pending
                    | RuntimeUpdateOutboxStatus::Delivering
                    | RuntimeUpdateOutboxStatus::Delivered
            )
    })
}

pub(super) fn reconcile_scan_failure(state: &mut RuntimeUpdateState, error: &str, now: i64) {
    state.last_checked_at_unix_ms = Some(now);
    state.last_error = Some(bound(error, 2_048));
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    let exponent = state.consecutive_failures.saturating_sub(1).min(5);
    let retry = FIRST_FAILURE_RETRY_MS
        .saturating_mul(1_i64 << exponent)
        .min(CHECK_INTERVAL_MS);
    state.next_check_at_unix_ms = now.saturating_add(retry);
}

pub(super) fn claim_outbox(
    state: &mut RuntimeUpdateState,
    lease_owner: &str,
    now: i64,
) -> Option<RuntimeUpdateOutbox> {
    for item in &mut state.outbox {
        if item.status == RuntimeUpdateOutboxStatus::Delivering
            && item
                .lease_expires_at_unix_ms
                .is_some_and(|expiry| expiry <= now)
        {
            item.status = RuntimeUpdateOutboxStatus::Pending;
            item.lease_owner = None;
            item.lease_expires_at_unix_ms = None;
        }
    }
    let mut indices = (0..state.outbox.len()).collect::<Vec<_>>();
    indices.sort_by_key(|index| state.outbox[*index].run_after_unix_ms);
    for index in indices {
        let item = &state.outbox[index];
        if item.status != RuntimeUpdateOutboxStatus::Pending || item.run_after_unix_ms > now {
            continue;
        }
        if !outbox_is_current(state, item, now) {
            state.outbox[index].status = RuntimeUpdateOutboxStatus::Suppressed;
            continue;
        }
        let item = &mut state.outbox[index];
        if item.attempt_count >= item.max_attempts {
            item.status = RuntimeUpdateOutboxStatus::Dead;
            continue;
        }
        item.status = RuntimeUpdateOutboxStatus::Delivering;
        item.attempt_count = item.attempt_count.saturating_add(1);
        item.lease_owner = Some(lease_owner.to_string());
        item.lease_expires_at_unix_ms = Some(now.saturating_add(OUTBOX_LEASE_MS));
        return Some(item.clone());
    }
    None
}

fn outbox_is_current(state: &RuntimeUpdateState, item: &RuntimeUpdateOutbox, now: i64) -> bool {
    if item.card.notice == RuntimeUpdateNoticeKind::Installed {
        return true;
    }
    if state.active_attempt.is_some() {
        return false;
    }
    state.pending.as_ref().is_some_and(|candidate| {
        candidate.token == item.card.token
            && candidate.decision_version == item.card.decision_version
            && candidate.available_version == item.card.available_version
            && candidate
                .deferred_until_unix_ms
                .is_none_or(|deferred_until| deferred_until <= now)
    })
}

pub(super) fn enqueue_notice(
    state: &mut RuntimeUpdateState,
    candidate: &RuntimeUpdateCandidate,
    notice: RuntimeUpdateNoticeKind,
    detail: Option<String>,
    run_after: i64,
) {
    let card = card_for_candidate(state, candidate, notice, detail);
    enqueue_card(state, card, run_after);
}

pub(super) fn enqueue_card(
    state: &mut RuntimeUpdateState,
    card: RuntimeUpdateCard,
    run_after: i64,
) {
    let id = format!(
        "{}:{}:{}",
        notice_key(card.notice),
        card.token,
        card.decision_version
    );
    if state.outbox.iter().any(|item| item.id == id) {
        return;
    }
    state.outbox.push(RuntimeUpdateOutbox {
        id,
        card,
        status: RuntimeUpdateOutboxStatus::Pending,
        attempt_count: 0,
        max_attempts: OUTBOX_MAX_ATTEMPTS,
        run_after_unix_ms: run_after,
        lease_owner: None,
        lease_expires_at_unix_ms: None,
        external_message_id: None,
        last_error: None,
        delivered_at_unix_ms: None,
    });
}

pub(super) fn card_for_candidate(
    state: &RuntimeUpdateState,
    candidate: &RuntimeUpdateCandidate,
    notice: RuntimeUpdateNoticeKind,
    detail: Option<String>,
) -> RuntimeUpdateCard {
    RuntimeUpdateCard {
        notice,
        token: candidate.token.clone(),
        decision_version: candidate.decision_version,
        current_version: candidate.current_version.clone(),
        available_version: candidate.available_version.clone(),
        release_url: candidate.release_url.clone(),
        published_at: candidate.published_at.clone(),
        prerelease: candidate.prerelease,
        install_mode: candidate.install_mode,
        checked_at: format_unix_ms(
            state
                .last_checked_at_unix_ms
                .unwrap_or(candidate.discovered_at_unix_ms),
        ),
        next_check_at: format_unix_ms(state.next_check_at_unix_ms),
        detail,
    }
}

pub(super) fn suppress_actionable_outbox(state: &mut RuntimeUpdateState) {
    for item in &mut state.outbox {
        if item.card.notice != RuntimeUpdateNoticeKind::Installed
            && matches!(
                item.status,
                RuntimeUpdateOutboxStatus::Pending | RuntimeUpdateOutboxStatus::Delivering
            )
        {
            item.status = RuntimeUpdateOutboxStatus::Suppressed;
            item.lease_owner = None;
            item.lease_expires_at_unix_ms = None;
        }
    }
}

pub(super) fn decision_suppresses_version(state: &RuntimeUpdateState, version: &str) -> bool {
    state
        .decisions
        .iter()
        .rev()
        .any(|decision| decision.available_version == version && decision.decision == "refused")
}

pub(super) fn record_decision(
    state: &mut RuntimeUpdateState,
    candidate: &RuntimeUpdateCandidate,
    decision: &str,
    actor: &str,
    now: i64,
) {
    state.decisions.push(RuntimeUpdateDecisionRecord {
        available_version: candidate.available_version.clone(),
        decision: decision.to_string(),
        actor: actor.to_string(),
        decided_at_unix_ms: now,
    });
}

pub(super) fn allocate_decision_version(state: &mut RuntimeUpdateState) -> u64 {
    let version = state.next_decision_version.max(1);
    state.next_decision_version = version.saturating_add(1);
    version
}

pub(super) fn remember_consumed_attempt(state: &mut RuntimeUpdateState, attempt_id: &str) {
    state.consumed_attempt_ids.push(attempt_id.to_string());
}

pub(super) fn requeue_candidate_after_failure(
    state: &mut RuntimeUpdateState,
    error: &str,
    now: i64,
) {
    suppress_actionable_outbox(state);
    let decision_version = allocate_decision_version(state);
    let Some(candidate) = state.pending.as_mut() else {
        return;
    };
    candidate.decision_version = decision_version;
    candidate.deferred_until_unix_ms = None;
    let candidate = candidate.clone();
    enqueue_notice(
        state,
        &candidate,
        RuntimeUpdateNoticeKind::InstallFailed,
        Some(bound(error, 1_000)),
        now,
    );
}

pub(super) fn expire_stale_update_attempt(state: &mut RuntimeUpdateState, now: i64) -> bool {
    let Some(attempt) = state.active_attempt.as_ref() else {
        return false;
    };
    if now.saturating_sub(attempt.started_at_unix_ms) < UPDATE_ATTEMPT_STALE_MS {
        return false;
    }
    let detail = format!(
        "Detached updater {} produced no durable result within 30 minutes.",
        attempt.attempt_id
    );
    state.active_attempt = None;
    state.last_error = Some(detail.clone());
    requeue_candidate_after_failure(state, &detail, now);
    true
}

pub(super) fn trim_state(state: &mut RuntimeUpdateState) {
    if state.decisions.len() > MAX_HISTORY {
        state
            .decisions
            .drain(0..state.decisions.len() - MAX_HISTORY);
    }
    if state.consumed_attempt_ids.len() > MAX_HISTORY {
        state
            .consumed_attempt_ids
            .drain(0..state.consumed_attempt_ids.len() - MAX_HISTORY);
    }
    let terminal = state
        .outbox
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(
                item.status,
                RuntimeUpdateOutboxStatus::Delivered
                    | RuntimeUpdateOutboxStatus::Suppressed
                    | RuntimeUpdateOutboxStatus::Dead
            )
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if terminal.len() > MAX_HISTORY {
        let overflow = terminal.len() - MAX_HISTORY;
        let remove = terminal.into_iter().take(overflow).collect::<BTreeSet<_>>();
        state.outbox = state
            .outbox
            .drain(..)
            .enumerate()
            .filter_map(|(index, item)| (!remove.contains(&index)).then_some(item))
            .collect();
    }
}

pub(super) fn outbox_retry_delay_ms(attempt: u32) -> i64 {
    30_000_i64
        .saturating_mul(1_i64 << attempt.saturating_sub(1).min(7))
        .min(60 * 60 * 1_000)
}

fn notice_key(notice: RuntimeUpdateNoticeKind) -> &'static str {
    match notice {
        RuntimeUpdateNoticeKind::Available => "available",
        RuntimeUpdateNoticeKind::Reminder => "reminder",
        RuntimeUpdateNoticeKind::InstallFailed => "failed",
        RuntimeUpdateNoticeKind::Installed => "installed",
    }
}

pub(super) fn format_unix_ms(value: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value.max(0))
        .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

pub(super) fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(super) fn bound(value: &str, max_bytes: usize) -> String {
    captain_types::truncate_str(value, max_bytes).to_string()
}
