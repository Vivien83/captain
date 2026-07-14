//! Visible progress, typing and lifecycle helpers for long channel turns.

use super::send_response;
use crate::types::{
    default_phase_emoji, AgentPhase, ChannelAdapter, ChannelType, ChannelUser, LifecycleReaction,
};
use captain_types::config::OutputFormat;
use std::sync::Arc;
use std::time::{Duration, Instant};

const VISIBLE_PROGRESS_INITIAL_DELAY_SECS: u64 = 45;
const VISIBLE_PROGRESS_INTERVAL_SECS: u64 = 120;

fn visible_progress_text(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs().div_ceil(60).max(1);
    if minutes <= 1 {
        "⏳ Toujours en cours. La session est active, je continue.".to_string()
    } else {
        format!("⏳ Toujours en cours depuis environ {minutes} min. Je continue.")
    }
}

/// Send a sparse visible heartbeat for long channel turns.
///
/// Typing indicators are useful but ephemeral; this message is intentionally
/// delayed and rate-limited so short turns stay clean while long Telegram/TUI
/// sessions reassure the user that Captain is still working.
fn spawn_visible_progress_loop(
    adapter: Arc<dyn ChannelAdapter>,
    sender: ChannelUser,
    thread_id: Option<String>,
    output_format: OutputFormat,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let started = Instant::now();
        tokio::time::sleep(Duration::from_secs(VISIBLE_PROGRESS_INITIAL_DELAY_SECS)).await;
        loop {
            send_response(
                adapter.as_ref(),
                &sender,
                visible_progress_text(started.elapsed()),
                thread_id.as_deref(),
                output_format,
            )
            .await;
            tokio::time::sleep(Duration::from_secs(VISIBLE_PROGRESS_INTERVAL_SECS)).await;
        }
    })
}

pub(super) fn spawn_visible_progress_loop_for_channel(
    channel: &ChannelType,
    adapter: Arc<dyn ChannelAdapter>,
    sender: ChannelUser,
    thread_id: Option<String>,
    output_format: OutputFormat,
) -> Option<tokio::task::JoinHandle<()>> {
    // Telegram streaming owns its visible-progress lifecycle inside the API
    // bridge so the task can be aborted before a fallback final response is
    // posted. The generic channel heartbeat can race and arrive after the
    // answer when Telegram rejects a live edit late in the turn.
    if matches!(channel, ChannelType::Telegram) {
        None
    } else {
        Some(spawn_visible_progress_loop(
            adapter,
            sender,
            thread_id,
            output_format,
        ))
    }
}

pub(super) fn abort_visible_progress_task(task: Option<tokio::task::JoinHandle<()>>) {
    if let Some(task) = task {
        task.abort();
    }
}

/// Send a lifecycle reaction (best-effort, non-blocking for supported adapters).
///
/// Silently ignores errors; reactions are non-critical UX polish. For Telegram,
/// the underlying HTTP call is already fire-and-forget, so this await returns
/// almost immediately.
pub(super) async fn send_lifecycle_reaction(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    message_id: &str,
    phase: AgentPhase,
) {
    let reaction = LifecycleReaction {
        emoji: default_phase_emoji(&phase).to_string(),
        phase,
        remove_previous: true,
    };
    let _ = adapter.send_reaction(user, message_id, &reaction).await;
}

/// Spawn a background task that refreshes the typing indicator every 4 seconds.
///
/// Returns a `JoinHandle` that should be aborted once the LLM call completes.
/// Telegram and similar platforms expire typing indicators after about 5
/// seconds, so refreshing at 4-second intervals keeps the indicator alive for
/// long LLM calls.
pub(super) fn spawn_typing_loop(
    adapter: Arc<dyn ChannelAdapter>,
    sender: ChannelUser,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let _ = adapter.send_typing(&sender).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_progress_text_is_sparse_and_human_readable() {
        assert_eq!(
            visible_progress_text(Duration::from_secs(45)),
            "⏳ Toujours en cours. La session est active, je continue."
        );
        assert_eq!(
            visible_progress_text(Duration::from_secs(181)),
            "⏳ Toujours en cours depuis environ 4 min. Je continue."
        );
    }
}
