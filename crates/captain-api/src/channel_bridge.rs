//! Channel bridge wiring — connects the Captain kernel to channel adapters.
//!
//! Implements `ChannelBridgeHandle` on `CaptainKernel` and provides the
//! `start_channel_bridge()` entry point called by the daemon.

use captain_channels::bridge::{BridgeManager, ChannelBridgeHandle};
use captain_channels::discord::DiscordAdapter;
use captain_channels::email::EmailAdapter;
use captain_channels::google_chat::GoogleChatAdapter;
use captain_channels::irc::IrcAdapter;
use captain_channels::matrix::MatrixAdapter;
use captain_channels::mattermost::MattermostAdapter;
use captain_channels::rocketchat::RocketChatAdapter;
use captain_channels::router::AgentRouter;
use captain_channels::signal::SignalAdapter;
use captain_channels::slack::SlackAdapter;
use captain_channels::teams::TeamsAdapter;
use captain_channels::telegram::TelegramAdapter;
use captain_channels::twitch::TwitchAdapter;
use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_channels::whatsapp::WhatsAppAdapter;
use captain_channels::xmpp::XmppAdapter;
use captain_channels::zulip::ZulipAdapter;
use captain_channels::{
    render_telegram_ask_user_answer, render_telegram_ask_user_expired,
    render_telegram_ask_user_prompt, TelegramProgressDraft,
};
// Wave 3
use captain_channels::bluesky::BlueskyAdapter;
use captain_channels::feishu::FeishuAdapter;
use captain_channels::line::LineAdapter;
use captain_channels::mastodon::MastodonAdapter;
use captain_channels::messenger::MessengerAdapter;
use captain_channels::reddit::RedditAdapter;
use captain_channels::revolt::RevoltAdapter;
use captain_channels::viber::ViberAdapter;
// Wave 4
use captain_channels::flock::FlockAdapter;
use captain_channels::guilded::GuildedAdapter;
use captain_channels::keybase::KeybaseAdapter;
use captain_channels::nextcloud::NextcloudAdapter;
use captain_channels::nostr::NostrAdapter;
use captain_channels::pumble::PumbleAdapter;
use captain_channels::threema::ThreemaAdapter;
use captain_channels::twist::TwistAdapter;
use captain_channels::webex::WebexAdapter;
// Wave 5
use async_trait::async_trait;
use captain_channels::dingtalk::DingTalkAdapter;
use captain_channels::dingtalk_stream::DingTalkStreamAdapter;
use captain_channels::discourse::DiscourseAdapter;
use captain_channels::gitter::GitterAdapter;
use captain_channels::gotify::GotifyAdapter;
use captain_channels::linkedin::LinkedInAdapter;
use captain_channels::mumble::MumbleAdapter;
use captain_channels::ntfy::NtfyAdapter;
use captain_channels::webhook::WebhookAdapter;
use captain_channels::wecom::WeComAdapter;
use captain_kernel::error::KernelResult;
use captain_kernel::model_switch::ModelSwitchSessionStrategy;
use captain_kernel::CaptainKernel;
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::kernel_handle::{skill_proposal_approval_decider, KernelHandle};
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::project_ask_answering::record_project_ask_answer_runtime;
use captain_runtime::str_utils::safe_truncate_str;

/// Wraps `CaptainKernel` to implement `ChannelBridgeHandle`.
pub struct KernelBridgeAdapter {
    kernel: Arc<CaptainKernel>,
    started_at: Instant,
    /// IJ.2/IJ.3 — per-agent state of an in-flight streaming loop.
    /// `sender` forwards interjections into the agent's
    /// `user_input_rx`; `reply_handle` updates the
    /// `TelegramStreamTarget`'s `pending_reply_to` so the next
    /// bubble cites the freshest user message rather than the one
    /// that started the turn. Both are removed when the loop ends.
    active_streams: Arc<tokio::sync::Mutex<ActiveStreamMap>>,
    /// Pending Telegram `ask_user` controls keyed by a short-lived id minted
    /// when each question is shown (never derived from `StreamEvent`, which
    /// has no `id` field). Button clicks resolve one entry; an accepted
    /// freeform answer resolves the waiting session's entries; stream end
    /// visibly expires anything left unanswered.
    pending_ask_users: Arc<std::sync::Mutex<HashMap<String, PendingTelegramAsk>>>,
}

/// Everything needed to resolve a button click or freeform answer back into
/// the waiting agent-loop turn. Only the option index round-trips through
/// Telegram callback data; the stored session identity avoids duplicating
/// the channel bridge's user/session resolution rules.
struct PendingTelegramAsk {
    agent_id: AgentId,
    session_key: String,
    question: String,
    options: Vec<String>,
    telegram: Arc<TelegramAdapter>,
    chat_id: i64,
    message_id: Option<i64>,
}

/// IJ.3 — what we keep alive while a streaming agent loop runs, to
/// allow follow-up user messages to interject AND rewrite the
/// reply-to quote target.
type ActiveStreamMap = HashMap<String, ActiveStream>;

struct ActiveStream {
    agent_id: AgentId,
    sender: tokio::sync::mpsc::Sender<String>,
    reply_handle: Arc<std::sync::Mutex<Option<i64>>>,
    progress_draft: Option<TelegramProgressDraft>,
}

type TelegramStreamParts = (
    tokio::sync::mpsc::Receiver<StreamEvent>,
    TelegramStreamJoin,
    tokio::sync::mpsc::Sender<String>,
);
type TelegramStreamJoin = tokio::task::JoinHandle<KernelResult<AgentLoopResult>>;
type ChannelAdapterStartup = (Arc<dyn ChannelAdapter>, Option<String>);

const TELEGRAM_STREAM_PROGRESS_INITIAL_DELAY_SECS: u64 = 20;
const TELEGRAM_STREAM_PROGRESS_INTERVAL_SECS: u64 = 20;
const SKILL_PROPOSAL_APPROVAL_USAGE: &str =
    "Usage: /skill_approve <id-prefix> schema diff tests human";

fn telegram_stream_progress_text(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs().div_ceil(60).max(1);
    format!(
        "<tg-thinking>Captain travaille…</tg-thinking>\n\n<blockquote>Tour actif · environ {minutes} min\nTu peux envoyer un complément, ou Stop pour interrompre.</blockquote>"
    )
}

fn telegram_streaming_enabled(config: &captain_types::config::ChannelsConfig) -> bool {
    config
        .telegram
        .as_ref()
        .map(|c| c.streaming)
        .unwrap_or(false)
}

fn telegram_channel_user(chat_id: i64) -> ChannelUser {
    ChannelUser {
        platform_id: chat_id.to_string(),
        display_name: "Telegram".to_string(),
        captain_user: None,
    }
}

fn telegram_thread_metadata(thread_id: Option<i64>) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    if let Some(tid) = thread_id {
        metadata.insert("thread_id".to_string(), serde_json::json!(tid));
    }
    metadata
}

async fn publish_telegram_channel_message(
    event_bus: &captain_kernel::event_bus::EventBus,
    agent_id: AgentId,
    sender: &'static str,
    content: String,
    response: Option<String>,
) {
    use captain_types::event::ChatStreamEvent;
    crate::chat_broadcast_publish(
        event_bus,
        agent_id,
        ChatStreamEvent::ChannelMessage {
            agent_id,
            channel: "telegram".to_string(),
            sender: sender.to_string(),
            content,
            response,
        },
    )
    .await;
}

fn start_telegram_agent_stream(
    kernel: &Arc<CaptainKernel>,
    agent_id: AgentId,
    message: &str,
) -> Result<TelegramStreamParts, String> {
    let kernel_handle: Arc<dyn KernelHandle> = kernel.clone() as Arc<dyn KernelHandle>;
    kernel
        .send_message_streaming(
            agent_id,
            message,
            Some(kernel_handle),
            None,
            None,
            None,
            Some("telegram".to_string()),
        )
        .map_err(|e| format!("send_message_streaming failed: {e}"))
}

#[allow(clippy::too_many_arguments)]
async fn pump_telegram_live_stream(
    raw_rx: tokio::sync::mpsc::Receiver<StreamEvent>,
    telegram: Arc<TelegramAdapter>,
    active_streams: &Arc<tokio::sync::Mutex<ActiveStreamMap>>,
    pending_ask_users: &Arc<std::sync::Mutex<HashMap<String, PendingTelegramAsk>>>,
    agent_id: AgentId,
    session_key: Option<&str>,
    chat_id: i64,
    thread_id: Option<i64>,
    user_message_id: Option<i64>,
    user_input_tx: tokio::sync::mpsc::Sender<String>,
) -> Option<String> {
    let target = captain_channels::telegram::TelegramStreamTarget::new(
        Arc::clone(&telegram),
        chat_id,
        thread_id,
    )
    .with_reply_to(user_message_id);
    let progress_draft = target.progress_draft();
    let progress_task = progress_draft
        .clone()
        .map(spawn_telegram_stream_progress_loop);
    let reply_handle = target.reply_to_handle();
    if let Some(key) = session_key {
        active_streams.lock().await.insert(
            key.to_string(),
            ActiveStream {
                agent_id,
                sender: user_input_tx,
                reply_handle,
                progress_draft: progress_draft.clone(),
            },
        );
    }

    let (rx, ask_user_ids) = tee_ask_user_events_to_telegram(
        raw_rx,
        Arc::clone(&telegram),
        Arc::clone(pending_ask_users),
        progress_draft,
        agent_id,
        session_key.map(str::to_string),
        chat_id,
        thread_id,
    );

    let mut consumer = captain_channels::stream_consumer::StreamConsumer::new(
        target,
        captain_channels::stream_consumer::StreamConsumerConfig::default(),
    );
    let stream_metric =
        crate::stream_metrics::StreamMetricHandle::start(agent_id.to_string(), "telegram");
    let stream_error = match crate::streaming_channels::pump_stream_to_channel_with_observer(
        rx,
        &mut consumer,
        |event| stream_metric.observe(event),
    )
    .await
    {
        Ok(()) => None,
        Err(e) => {
            warn!(error = %e, "telegram streaming pump failed");
            Some(e.to_string())
        }
    };
    stream_metric.finish();
    if let Some(progress_task) = progress_task {
        progress_task.abort();
    }
    if let Some(key) = session_key {
        active_streams.lock().await.remove(key);
    }
    expire_unanswered_telegram_asks(&ask_user_ids, pending_ask_users).await;
    stream_error
}

/// TG1 — forward every `StreamEvent` unchanged except `AskUser`, which is
/// intercepted here instead of reaching `map_runtime_event()` (which drops
/// it — see the plan's "découverte critique"). On `AskUser`, post the
/// question to Telegram as a Rich card and register inline-keyboard options.
/// Returns the receiving half of a fresh channel plus every short-lived id
/// created during the stream so unanswered controls can be expired visibly.
#[allow(clippy::too_many_arguments)]
fn tee_ask_user_events_to_telegram(
    mut raw_rx: tokio::sync::mpsc::Receiver<StreamEvent>,
    telegram: Arc<TelegramAdapter>,
    pending_ask_users: Arc<std::sync::Mutex<HashMap<String, PendingTelegramAsk>>>,
    progress_draft: Option<TelegramProgressDraft>,
    agent_id: AgentId,
    session_key: Option<String>,
    chat_id: i64,
    thread_id: Option<i64>,
) -> (
    tokio::sync::mpsc::Receiver<StreamEvent>,
    Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (forward_tx, forward_rx) = tokio::sync::mpsc::channel(32);
    let ask_user_ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ask_user_ids_task = Arc::clone(&ask_user_ids);

    tokio::spawn(async move {
        while let Some(event) = raw_rx.recv().await {
            if let StreamEvent::AskUser { question, options } = &event {
                let options = options.clone().unwrap_or_default();
                let user = telegram_channel_user(chat_id);
                let mut metadata = telegram_thread_metadata(thread_id);
                let has_buttons = !options.is_empty() && session_key.is_some();
                let short_id = session_key
                    .as_ref()
                    .map(|_| uuid::Uuid::new_v4().to_string());
                if has_buttons {
                    let short_id = short_id
                        .as_ref()
                        .expect("button-backed ask_user requires a session key");
                    metadata.insert(
                        "reply_markup".to_string(),
                        captain_channels::telegram::build_ask_user_keyboard(short_id, &options),
                    );
                }
                let content =
                    ChannelContent::Text(render_telegram_ask_user_prompt(question, has_buttons));
                match telegram.send_rich(&user, content, &metadata).await {
                    Ok(sent_id) => {
                        if let Some(short_id) = short_id {
                            let message_id = sent_id.and_then(|id| id.parse::<i64>().ok());
                            if let (Some(key), Ok(mut guard)) =
                                (session_key.as_ref(), pending_ask_users.lock())
                            {
                                guard.insert(
                                    short_id.clone(),
                                    PendingTelegramAsk {
                                        agent_id,
                                        session_key: key.clone(),
                                        question: question.clone(),
                                        options: options.clone(),
                                        telegram: Arc::clone(&telegram),
                                        chat_id,
                                        message_id,
                                    },
                                );
                            }
                            if let Ok(mut guard) = ask_user_ids_task.lock() {
                                guard.push(short_id);
                            }
                        }
                        if let Some(progress) = &progress_draft {
                            progress.set_waiting_for_user(true);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to post ask_user question to Telegram");
                    }
                }
                continue;
            }
            if forward_tx.send(event).await.is_err() {
                break;
            }
        }
    });

    (forward_rx, ask_user_ids)
}

async fn edit_pending_telegram_ask(pending: &PendingTelegramAsk, content: String) {
    let Some(message_id) = pending.message_id else {
        return;
    };
    let mut metadata = HashMap::new();
    metadata.insert("chat_id".to_string(), serde_json::json!(pending.chat_id));
    if let Err(e) = pending
        .telegram
        .edit_rich(&message_id.to_string(), &content, &metadata)
        .await
    {
        warn!(error = %e, "failed to resolve Telegram ask_user card");
    }
}

async fn expire_unanswered_telegram_asks(
    ask_user_ids: &Arc<std::sync::Mutex<Vec<String>>>,
    pending_ask_users: &Arc<std::sync::Mutex<HashMap<String, PendingTelegramAsk>>>,
) {
    let ids = ask_user_ids
        .lock()
        .map(|mut ids| ids.drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    let stale = pending_ask_users
        .lock()
        .map(|mut pending| {
            ids.into_iter()
                .filter_map(|id| pending.remove(&id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for pending in stale {
        edit_pending_telegram_ask(
            &pending,
            render_telegram_ask_user_expired(&pending.question),
        )
        .await;
    }
}

async fn await_telegram_stream_result(
    join: TelegramStreamJoin,
    agent_id: AgentId,
) -> Result<Option<AgentLoopResult>, String> {
    match join.await {
        Ok(Ok(result)) => Ok(Some(result)),
        Ok(Err(e)) => Err(format!("agent loop error: {e}")),
        Err(e) if e.is_cancelled() => {
            info!(agent_id = %agent_id, "telegram streaming run cancelled");
            Ok(None)
        }
        Err(e) => Err(format!("agent loop join error: {e}")),
    }
}

async fn send_telegram_stream_fallback(
    telegram: Arc<TelegramAdapter>,
    chat_id: i64,
    thread_id: Option<i64>,
    stream_error: &str,
    response: &str,
) -> Result<(), String> {
    let user = telegram_channel_user(chat_id);
    let metadata = telegram_thread_metadata(thread_id);
    telegram
        .send_rich(&user, ChannelContent::Text(response.to_string()), &metadata)
        .await
        .map(|_| ())
        .map_err(|e| format!("telegram stream fallback send failed after {stream_error}: {e}"))
}

async fn finalize_telegram_stream_response(
    event_bus: &captain_kernel::event_bus::EventBus,
    fallback_telegram: Arc<TelegramAdapter>,
    agent_id: AgentId,
    chat_id: i64,
    thread_id: Option<i64>,
    stream_error: Option<String>,
    result: AgentLoopResult,
) -> Result<String, String> {
    if result.silent {
        return Ok(String::new());
    }

    if let Some(error) = stream_error {
        send_telegram_stream_fallback(
            fallback_telegram,
            chat_id,
            thread_id,
            &error,
            &result.response,
        )
        .await?;
        warn!(
            agent_id = %agent_id,
            stream_error = %error,
            "telegram streaming fallback sent full final response"
        );
    }

    publish_telegram_channel_message(
        event_bus,
        agent_id,
        "agent",
        String::new(),
        Some(result.response.clone()),
    )
    .await;

    Ok(result.response)
}

fn skill_proposal_channel_decided_by(
    approve: bool,
    external_validation: bool,
) -> Result<String, &'static str> {
    if approve && !external_validation {
        return Err(SKILL_PROPOSAL_APPROVAL_USAGE);
    }
    if approve {
        Ok(skill_proposal_approval_decider("channel"))
    } else {
        Ok("channel".to_string())
    }
}

fn cron_action_message(action: &CronAction) -> String {
    match action {
        CronAction::AgentTurn { message, .. } => message.clone(),
        CronAction::SystemEvent { text } => text.clone(),
        CronAction::WorkflowRun {
            workflow_id, input, ..
        } => {
            format!(
                "Run workflow {workflow_id}{}",
                input
                    .as_deref()
                    .map(|i| format!(" with input: {i}"))
                    .unwrap_or_default()
            )
        }
        CronAction::InlineWorkflow { steps } => format!("Inline workflow ({} steps)", steps.len()),
    }
}

fn add_schedule_text(adapter: &KernelBridgeAdapter, args: &[String]) -> String {
    if args.len() < 7 {
        return "Usage: /schedule add <agent> <min> <hour> <dom> <month> <dow> <message>"
            .to_string();
    }
    let agent_name = &args[0];
    let agent = match adapter.kernel.registry.find_by_name(agent_name) {
        Some(e) => e,
        None => return format!("Agent '{agent_name}' not found."),
    };
    let cron_expr = args[1..6].join(" ");
    let message = args[6..].join(" ");
    let job = CronJob {
        id: CronJobId::new(),
        agent_id: agent.id,
        name: format!("chat-{}", agent.name),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: cron_expr.clone(),
            tz: None,
        },
        action: CronAction::AgentTurn {
            message: message.clone(),
            model_override: None,
            timeout_secs: None,
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    match adapter.kernel.cron_scheduler.add_job(job, false) {
        Ok(id) => {
            let id_str = id.0.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            format!("Job [{id_short}] created: '{cron_expr}' -> {agent_name}: \"{message}\"")
        }
        Err(e) => format!("Failed to create job: {e}"),
    }
}

fn delete_schedule_text(adapter: &KernelBridgeAdapter, args: &[String]) -> String {
    if args.is_empty() {
        return "Usage: /schedule del <id-prefix>".to_string();
    }
    let prefix = &args[0];
    let jobs = adapter.kernel.cron_scheduler.list_all_jobs();
    let matched: Vec<_> = jobs
        .iter()
        .filter(|j| j.id.0.to_string().starts_with(prefix.as_str()))
        .collect();
    match matched.len() {
        0 => format!("No job found matching '{prefix}'."),
        1 => {
            let job = matched[0];
            match adapter.kernel.cron_scheduler.remove_job(job.id) {
                Ok(_) => {
                    let id_str = job.id.0.to_string();
                    format!(
                        "Job [{}] '{}' removed.",
                        safe_truncate_str(&id_str, 8),
                        job.name
                    )
                }
                Err(e) => format!("Failed to remove job: {e}"),
            }
        }
        n => format!("{n} jobs match '{prefix}'. Be more specific."),
    }
}

async fn run_schedule_text(adapter: &KernelBridgeAdapter, args: &[String]) -> String {
    if args.is_empty() {
        return "Usage: /schedule run <id-prefix>".to_string();
    }
    let prefix = &args[0];
    let jobs = adapter.kernel.cron_scheduler.list_all_jobs();
    let matched: Vec<_> = jobs
        .iter()
        .filter(|j| j.id.0.to_string().starts_with(prefix.as_str()))
        .collect();
    match matched.len() {
        0 => format!("No job found matching '{prefix}'."),
        1 => {
            let job = matched[0];
            let message = cron_action_message(&job.action);
            match adapter.kernel.send_message(job.agent_id, &message).await {
                Ok(result) => {
                    let id_str = job.id.0.to_string();
                    let id_short = safe_truncate_str(&id_str, 8);
                    format!("Job [{id_short}] ran:\n{}", result.response)
                }
                Err(e) => format!("Failed to run job: {e}"),
            }
        }
        n => format!("{n} jobs match '{prefix}'. Be more specific."),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveInterjectionSend {
    Accepted,
    QueueNextTurn,
    Closed,
}

fn try_send_active_interjection(
    sender: &tokio::sync::mpsc::Sender<String>,
    message: &str,
) -> ActiveInterjectionSend {
    match sender.try_send(message.to_string()) {
        Ok(()) => ActiveInterjectionSend::Accepted,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            ActiveInterjectionSend::QueueNextTurn
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => ActiveInterjectionSend::Closed,
    }
}

#[allow(clippy::type_complexity)]
fn clone_active_stream_for_session(
    streams: &ActiveStreamMap,
    session_key: Option<&str>,
    agent_id: AgentId,
) -> Option<(
    tokio::sync::mpsc::Sender<String>,
    Arc<std::sync::Mutex<Option<i64>>>,
    Option<TelegramProgressDraft>,
)> {
    let key = session_key?;
    let active = streams.get(key)?;
    if active.agent_id != agent_id {
        return None;
    }
    Some((
        active.sender.clone(),
        active.reply_handle.clone(),
        active.progress_draft.clone(),
    ))
}

fn take_pending_telegram_ask(
    pending_ask_users: &std::sync::Mutex<HashMap<String, PendingTelegramAsk>>,
    short_id: &str,
    idx: usize,
) -> Result<(PendingTelegramAsk, String), String> {
    let mut pending = pending_ask_users
        .lock()
        .map_err(|_| "ask_user registry poisoned".to_string())?;
    let Some(chosen) = pending
        .get(short_id)
        .and_then(|entry| entry.options.get(idx))
        .cloned()
    else {
        return if pending.contains_key(short_id) {
            Err("Ce choix n'est pas valide. Utilise un des boutons proposés.".to_string())
        } else {
            Err("Cette question n'est plus active.".to_string())
        };
    };
    let Some(entry) = pending.remove(short_id) else {
        return Err("Cette question n'est plus active.".to_string());
    };
    Ok((entry, chosen))
}

fn take_pending_telegram_asks_for_session(
    pending_ask_users: &std::sync::Mutex<HashMap<String, PendingTelegramAsk>>,
    session_key: &str,
) -> Result<Vec<PendingTelegramAsk>, String> {
    let mut pending = pending_ask_users
        .lock()
        .map_err(|_| "ask_user registry poisoned".to_string())?;
    let ids = pending
        .iter()
        .filter(|(_, entry)| entry.session_key == session_key)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    Ok(ids
        .into_iter()
        .filter_map(|id| pending.remove(&id))
        .collect())
}

fn spawn_telegram_stream_progress_loop(
    progress: TelegramProgressDraft,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let started = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(TELEGRAM_STREAM_PROGRESS_INTERVAL_SECS)).await;
            if progress.is_waiting_for_user()
                || progress.idle_for()
                    < Duration::from_secs(TELEGRAM_STREAM_PROGRESS_INITIAL_DELAY_SECS)
            {
                continue;
            }
            match progress
                .refresh(&telegram_stream_progress_text(started.elapsed()))
                .await
            {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => warn!(error = %e, "telegram stream progress draft refresh failed"),
            }
        }
    })
}

fn classify_diagnostic_error(content: &str) -> String {
    let lower = content.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
        "timeout".to_string()
    } else if lower.contains("rate limit") || lower.contains("429") {
        "rate_limit".to_string()
    } else if lower.contains("unauthorized")
        || lower.contains("authentication")
        || lower.contains("api key")
        || lower.contains("billing")
        || lower.contains("quota")
    {
        "auth_or_billing".to_string()
    } else if lower.contains("permission") || lower.contains("approval") || lower.contains("denied")
    {
        "permission_or_approval".to_string()
    } else if lower.contains("context")
        || lower.contains("too many tokens")
        || lower.contains("token limit")
    {
        "context_limit".to_string()
    } else if lower.contains("max iterations") {
        "max_iterations".to_string()
    } else if lower.contains("not found") {
        "not_found".to_string()
    } else {
        let first_line = content
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let mut preview = safe_truncate_str(first_line.trim(), 180);
        if preview.contains('{') || preview.contains('}') {
            preview = "raw_error_hidden_json_payload";
        }
        format!("other:{preview}")
    }
}

fn recent_session_diagnostics(session: &captain_memory::session::Session) -> String {
    use captain_types::message::{ContentBlock, MessageContent};

    let recent_window = 28;
    let start = session.messages.len().saturating_sub(recent_window);
    let mut failures: Vec<String> = Vec::new();
    let mut successes: Vec<String> = Vec::new();
    let mut tool_uses: Vec<String> = Vec::new();

    for msg in &session.messages[start..] {
        let MessageContent::Blocks(blocks) = &msg.content else {
            continue;
        };
        for block in blocks {
            match block {
                ContentBlock::ToolUse { name, .. } => tool_uses.push(name.clone()),
                ContentBlock::ToolResult {
                    tool_name,
                    content,
                    is_error,
                    ..
                } => {
                    let name = if tool_name.trim().is_empty() {
                        "unknown_tool"
                    } else {
                        tool_name.as_str()
                    };
                    if *is_error {
                        failures.push(format!("{name}: {}", classify_diagnostic_error(content)));
                    } else {
                        successes.push(name.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    successes.dedup();
    tool_uses.dedup();

    let mut lines = vec![format!(
        "- Messages récents inspectés: {} sur {}.",
        session.messages.len().saturating_sub(start),
        session.messages.len()
    )];

    if failures.is_empty() {
        lines.push("- Échecs outil récents visibles: aucun.".to_string());
    } else {
        lines.push(format!(
            "- Échecs outil récents visibles: {}.",
            failures.join("; ")
        ));
    }

    if !successes.is_empty() {
        let tail: Vec<&str> = successes.iter().rev().take(8).map(String::as_str).collect();
        lines.push(format!(
            "- Derniers outils terminés avec succès: {}.",
            tail.into_iter().rev().collect::<Vec<_>>().join(", ")
        ));
    } else if !tool_uses.is_empty() {
        let tail: Vec<&str> = tool_uses.iter().rev().take(8).map(String::as_str).collect();
        lines.push(format!(
            "- Derniers outils demandés: {}.",
            tail.into_iter().rev().collect::<Vec<_>>().join(", ")
        ));
    }

    lines.join("\n")
}

const SKILL_REFINEMENTS_KEY: &str = "__captain_skill_refinement_registry";

fn load_skill_refinement_registry(
    kernel: &CaptainKernel,
) -> Result<Vec<serde_json::Value>, String> {
    match kernel.memory_recall(SKILL_REFINEMENTS_KEY)? {
        Some(serde_json::Value::Array(items)) => Ok(items),
        Some(_) => Err("Le registre des raffinements de skills est corrompu.".to_string()),
        None => Ok(Vec::new()),
    }
}

fn store_skill_refinement_registry(
    kernel: &CaptainKernel,
    items: Vec<serde_json::Value>,
) -> Result<(), String> {
    kernel.memory_store(SKILL_REFINEMENTS_KEY, serde_json::Value::Array(items))
}

fn resolve_skill_refinement_index(
    items: &[serde_json::Value],
    id_prefix: &str,
) -> Result<usize, String> {
    let matches: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            item["id"]
                .as_str()
                .filter(|id| id.starts_with(id_prefix))
                .map(|_| idx)
        })
        .collect();
    match matches.len() {
        0 => Err(format!(
            "Aucun raffinement de skill correspondant à « {id_prefix} »."
        )),
        1 => Ok(matches[0]),
        n => Err(format!(
            "{n} raffinements de skill correspondent à « {id_prefix} ». Sois plus précis."
        )),
    }
}

impl KernelBridgeAdapter {
    pub fn new(kernel: Arc<CaptainKernel>) -> Self {
        Self {
            kernel,
            started_at: Instant::now(),
            active_streams: Arc::new(tokio::sync::Mutex::new(ActiveStreamMap::new())),
            pending_ask_users: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    async fn try_forward_active_stream_interjection(
        &self,
        agent_id: AgentId,
        session_key: Option<&str>,
        message: &str,
        telegram_reply_to: Option<i64>,
    ) -> Result<bool, String> {
        let active = {
            let map = self.active_streams.lock().await;
            clone_active_stream_for_session(&map, session_key, agent_id)
        };
        let Some((sender, reply_handle, progress_draft)) = active else {
            return Ok(false);
        };

        match try_send_active_interjection(&sender, message) {
            ActiveInterjectionSend::Accepted => {
                if let Some(new_id) = telegram_reply_to {
                    if let Ok(mut g) = reply_handle.lock() {
                        *g = Some(new_id);
                    }
                }
                let was_waiting_for_user = progress_draft
                    .as_ref()
                    .is_some_and(TelegramProgressDraft::is_waiting_for_user);
                if let Some(progress) = &progress_draft {
                    progress.set_waiting_for_user(false);
                }
                if was_waiting_for_user {
                    if let Some(session_key) = session_key {
                        match take_pending_telegram_asks_for_session(
                            &self.pending_ask_users,
                            session_key,
                        ) {
                            Ok(pending) => {
                                for pending in pending {
                                    edit_pending_telegram_ask(
                                        &pending,
                                        render_telegram_ask_user_answer(&pending.question, message),
                                    )
                                    .await;
                                }
                            }
                            Err(error) => {
                                warn!(%error, "failed to resolve Telegram ask_user card after accepted freeform answer");
                            }
                        }
                    }
                }
                info!(
                    agent_id = %agent_id,
                    new_reply_to = ?telegram_reply_to,
                    "user interjection forwarded into active stream"
                );
                Ok(true)
            }
            ActiveInterjectionSend::QueueNextTurn => {
                warn!(
                    agent_id = %agent_id,
                    "telegram interjection queue full; falling back to queued next turn"
                );
                Ok(false)
            }
            ActiveInterjectionSend::Closed => {
                if let Some(key) = session_key {
                    self.active_streams.lock().await.remove(key);
                }
                warn!(
                    agent_id = %agent_id,
                    "telegram interjection channel closed; stale active stream removed, continuing as a new turn"
                );
                Ok(false)
            }
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for KernelBridgeAdapter {
    async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
        channel_type: Option<&str>,
    ) -> Result<String, String> {
        let channel_name = channel_type.unwrap_or("unknown").to_string();

        // Broadcast incoming message immediately so web clients see it in real time
        {
            use captain_types::event::ChatStreamEvent;
            crate::chat_broadcast_publish(
                &self.kernel.event_bus,
                agent_id,
                ChatStreamEvent::ChannelMessage {
                    agent_id,
                    channel: channel_name.clone(),
                    sender: "user".to_string(),
                    content: message.to_string(),
                    response: None,
                },
            )
            .await;
        }

        let result = self
            .kernel
            .send_message_full(
                agent_id,
                message,
                Some(self.kernel.clone()),
                None,
                None,
                None,
                channel_type.map(String::from),
            )
            .await
            .map_err(|e| format!("{e}"))?;
        if result.silent {
            return Ok(String::new());
        }

        // Broadcast agent response
        {
            use captain_types::event::ChatStreamEvent;
            crate::chat_broadcast_publish(
                &self.kernel.event_bus,
                agent_id,
                ChatStreamEvent::ChannelMessage {
                    agent_id,
                    channel: channel_name,
                    sender: "agent".to_string(),
                    content: String::new(),
                    response: Some(result.response.clone()),
                },
            )
            .await;
        }

        Ok(result.response)
    }

    async fn transcribe_channel_audio(
        &self,
        path: &str,
        language: Option<&str>,
    ) -> Result<Option<String>, String> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("audio metadata failed: {e}"))?;
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ogg")
            .to_ascii_lowercase();
        let mime_type = match ext.as_str() {
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "ogg" | "oga" => "audio/ogg",
            "flac" => "audio/flac",
            "m4a" => "audio/mp4",
            "webm" => "audio/webm",
            _ => "audio/ogg",
        };
        let attachment = captain_types::media::MediaAttachment {
            media_type: captain_types::media::MediaType::Audio,
            mime_type: mime_type.to_string(),
            source: captain_types::media::MediaSource::FilePath {
                path: path.to_string(),
            },
            size_bytes: metadata.len(),
            context_hint: language
                .filter(|s| !s.trim().is_empty())
                .map(|s| format!("language:{}", s.trim())),
            batch_size_hint: None,
        };

        let understanding = self
            .kernel
            .media_engine
            .transcribe_audio(&attachment)
            .await?;
        Ok(Some(understanding.description))
    }

    async fn describe_channel_image(
        &self,
        path: &str,
        prompt: Option<&str>,
    ) -> Result<Option<String>, String> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("image metadata failed: {e}"))?;
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => {
                return Err(format!(
                    "unsupported channel image extension for description: .{ext}"
                ))
            }
        };
        let context_hint = prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("Demande ou légende utilisateur: {s}"));
        let attachment = captain_types::media::MediaAttachment {
            media_type: captain_types::media::MediaType::Image,
            mime_type: mime_type.to_string(),
            source: captain_types::media::MediaSource::FilePath {
                path: path.to_string(),
            },
            size_bytes: metadata.len(),
            context_hint,
            batch_size_hint: None,
        };

        let understanding = self.kernel.media_engine.describe_image(&attachment).await?;
        Ok(Some(understanding.description))
    }

    async fn try_interject_active_agent(
        &self,
        agent_id: AgentId,
        channel: &str,
        session_key: &str,
        message: &str,
        platform_message_id: Option<&str>,
    ) -> Result<bool, String> {
        if !channel.eq_ignore_ascii_case("telegram") {
            return Ok(false);
        }
        let telegram_reply_to = platform_message_id.and_then(|id| id.parse::<i64>().ok());
        self.try_forward_active_stream_interjection(
            agent_id,
            Some(session_key),
            message,
            telegram_reply_to,
        )
        .await
    }

    async fn try_answer_ask_user(&self, short_id: &str, idx: usize) -> Result<String, String> {
        let (pending, chosen) = take_pending_telegram_ask(&self.pending_ask_users, short_id, idx)?;

        let delivered = self
            .try_forward_active_stream_interjection(
                pending.agent_id,
                Some(&pending.session_key),
                &chosen,
                None,
            )
            .await;

        match delivered {
            Ok(true) => {
                edit_pending_telegram_ask(
                    &pending,
                    render_telegram_ask_user_answer(&pending.question, &chosen),
                )
                .await;
                Ok(chosen)
            }
            Ok(false) => {
                edit_pending_telegram_ask(
                    &pending,
                    render_telegram_ask_user_expired(&pending.question),
                )
                .await;
                Err("Cette question n'est plus active.".to_string())
            }
            Err(error) => {
                warn!(error = %error, "failed to deliver Telegram ask_user answer");
                edit_pending_telegram_ask(
                    &pending,
                    render_telegram_ask_user_expired(&pending.question),
                )
                .await;
                Err("Cette question n'est plus active.".to_string())
            }
        }
    }

    async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        blocks: Vec<captain_types::message::ContentBlock>,
    ) -> Result<String, String> {
        let text: String = blocks
            .iter()
            .filter_map(|b| match b {
                captain_types::message::ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = if text.is_empty() {
            "[Image]".to_string()
        } else {
            text
        };

        // Broadcast incoming message immediately
        {
            use captain_types::event::ChatStreamEvent;
            crate::chat_broadcast_publish(
                &self.kernel.event_bus,
                agent_id,
                ChatStreamEvent::ChannelMessage {
                    agent_id,
                    channel: "telegram".to_string(),
                    sender: "user".to_string(),
                    content: text.clone(),
                    response: None,
                },
            )
            .await;
        }

        // Pre-flight: if model can't handle the content (e.g., vision for images),
        // delegate to a specialist and inject the analysis back into Captain's context.
        let (enriched_text, cleaned_blocks) = self
            .kernel
            .resolve_capability_gap(agent_id, &text, Some(blocks))
            .await
            .map_err(|e| format!("{e}"))?;

        let final_blocks = cleaned_blocks.unwrap_or_default();
        let content_blocks = if final_blocks.is_empty() {
            None
        } else {
            Some(final_blocks)
        };
        let result = self
            .kernel
            .send_message_full(
                agent_id,
                &enriched_text,
                Some(self.kernel.clone()),
                content_blocks,
                None,
                None,
                Some("telegram".to_string()),
            )
            .await;

        let result = result.map_err(|e| format!("{e}"))?;

        // Broadcast agent response
        {
            use captain_types::event::ChatStreamEvent;
            crate::chat_broadcast_publish(
                &self.kernel.event_bus,
                agent_id,
                ChatStreamEvent::ChannelMessage {
                    agent_id,
                    channel: "telegram".to_string(),
                    sender: "agent".to_string(),
                    content: String::new(),
                    response: Some(result.response.clone()),
                },
            )
            .await;
        }

        Ok(result.response)
    }

    /// HS.3b — Live-stream the agent's reply into Telegram instead of
    /// posting one final block. Gated by `[channels.telegram] streaming
    /// = true` in the active config so this is opt-in per deployment.
    ///
    /// Side-effect parity with `send_message`:
    ///   - chat_broadcast: incoming "user" event published before
    ///     dispatch, "agent" event published after with the captured
    ///     full response (so the web/SSE clients still see the final
    ///     text, not the interleaved deltas).
    ///   - audit/persistence: relies on `send_message_streaming`'s
    ///     join handle resolving to the same `AgentLoopResult` shape;
    ///     `result.silent` is preserved.
    async fn try_stream_telegram_response(
        &self,
        telegram: Arc<TelegramAdapter>,
        chat_id: i64,
        thread_id: Option<i64>,
        user_message_id: Option<i64>,
        agent_id: AgentId,
        session_key: Option<&str>,
        message: &str,
    ) -> Option<Result<String, String>> {
        if !telegram_streaming_enabled(&self.kernel.config.channels) {
            return None;
        }

        // IJ.2/IJ.3 — if a streaming loop is already running, forward the
        // message into `user_input_rx` and update the Telegram reply target.
        match self
            .try_forward_active_stream_interjection(agent_id, session_key, message, user_message_id)
            .await
        {
            Ok(true) => return Some(Ok(String::new())),
            Ok(false) => {}
            Err(err) => {
                warn!(
                    agent_id = %agent_id,
                    "telegram interjection forwarding failed, continuing as a new turn: {err}"
                );
            }
        }

        publish_telegram_channel_message(
            &self.kernel.event_bus,
            agent_id,
            "user",
            message.to_string(),
            None,
        )
        .await;

        let (rx, join, user_input_tx) =
            match start_telegram_agent_stream(&self.kernel, agent_id, message) {
                Ok(triple) => triple,
                Err(e) => return Some(Err(e)),
            };

        let fallback_telegram = Arc::clone(&telegram);
        let stream_error = pump_telegram_live_stream(
            rx,
            telegram,
            &self.active_streams,
            &self.pending_ask_users,
            agent_id,
            session_key,
            chat_id,
            thread_id,
            user_message_id,
            user_input_tx,
        )
        .await;

        let result = match await_telegram_stream_result(join, agent_id).await {
            Ok(Some(result)) => result,
            Ok(None) => return Some(Ok(String::new())),
            Err(e) => return Some(Err(e)),
        };

        let response = finalize_telegram_stream_response(
            &self.kernel.event_bus,
            fallback_telegram,
            agent_id,
            chat_id,
            thread_id,
            stream_error,
            result,
        )
        .await;

        Some(response)
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        Ok(self.kernel.registry.find_by_name(name).map(|e| e.id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self
            .kernel
            .registry
            .list()
            .iter()
            .map(|e| (e.id, e.name.clone()))
            .collect())
    }

    async fn spawn_agent_by_name(&self, manifest_name: &str) -> Result<AgentId, String> {
        // Look for manifest at ~/.captain/agents/{name}/agent.toml
        let manifest_path = self
            .kernel
            .config
            .home_dir
            .join("agents")
            .join(manifest_name)
            .join("agent.toml");

        if !manifest_path.exists() {
            return Err(format!("Manifest not found: {}", manifest_path.display()));
        }

        let contents = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read manifest: {e}"))?;

        let manifest: captain_types::agent::AgentManifest =
            toml::from_str(&contents).map_err(|e| format!("Invalid manifest TOML: {e}"))?;

        let agent_id = self
            .kernel
            .spawn_agent(manifest)
            .map_err(|e| format!("Failed to spawn agent: {e}"))?;

        Ok(agent_id)
    }

    async fn uptime_info(&self) -> String {
        let uptime = self.started_at.elapsed();
        let agents = self.list_agents().await.unwrap_or_default();
        let secs = uptime.as_secs();
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if hours > 0 {
            format!(
                "Captain status: {}h {}m uptime, {} agent(s)",
                hours,
                mins,
                agents.len()
            )
        } else {
            format!(
                "Captain status: {}m uptime, {} agent(s)",
                mins,
                agents.len()
            )
        }
    }

    async fn daemon_command_text(
        &self,
        command: &str,
        args: &[String],
        channel_type: &str,
        sender_platform_id: &str,
        sender_user_id: &str,
        thread_id: Option<&str>,
        source_message_id: Option<&str>,
    ) -> String {
        crate::daemon_commands::handle_daemon_command(
            self.kernel.clone(),
            Some(self.started_at),
            None,
            command,
            args,
            crate::daemon_commands::DaemonCommandOrigin::new(
                channel_type,
                sender_user_id,
                Some(sender_platform_id.to_string()),
                thread_id.map(ToString::to_string),
            )
            .with_source_message_id(source_message_id.map(ToString::to_string)),
        )
        .await
    }

    async fn list_models_text(&self) -> String {
        let catalog = self
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let available = catalog.available_models();
        if available.is_empty() {
            return "No models available. Configure API keys to enable providers.".to_string();
        }
        let mut msg = format!("Available models ({}):\n", available.len());
        // Group by provider
        let mut by_provider: std::collections::HashMap<
            &str,
            Vec<&captain_types::model_catalog::ModelCatalogEntry>,
        > = std::collections::HashMap::new();
        for m in &available {
            by_provider.entry(m.provider.as_str()).or_default().push(m);
        }
        let mut providers: Vec<&&str> = by_provider.keys().collect();
        providers.sort();
        for provider in providers {
            let provider_name = catalog
                .get_provider(provider)
                .map(|p| p.display_name.as_str())
                .unwrap_or(provider);
            msg.push_str(&format!("\n{}:\n", provider_name));
            for m in &by_provider[provider] {
                let cost = if m.input_cost_per_m > 0.0 {
                    format!(
                        " (${:.2}/${:.2} per M)",
                        m.input_cost_per_m, m.output_cost_per_m
                    )
                } else {
                    " (free/local)".to_string()
                };
                msg.push_str(&format!("  {} — {}{}\n", m.id, m.display_name, cost));
            }
        }
        msg
    }

    async fn list_providers_text(&self) -> String {
        let catalog = self
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let mut msg = "Providers:\n".to_string();
        for p in catalog.list_providers() {
            let status = match p.auth_status {
                captain_types::model_catalog::AuthStatus::Configured => "configured",
                captain_types::model_catalog::AuthStatus::Missing => "not configured",
                captain_types::model_catalog::AuthStatus::NotRequired => "local (no key needed)",
            };
            msg.push_str(&format!(
                "  {} — {} [{}, {} model(s)]\n",
                p.id, p.display_name, status, p.model_count
            ));
        }
        msg
    }

    async fn list_skills_text(&self) -> String {
        let skills = self
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let skills = skills.list();
        if skills.is_empty() {
            return "No skills installed. Place skills in ~/.captain/skills/ or install from the marketplace.".to_string();
        }
        let mut msg = format!("Installed skills ({}):\n", skills.len());
        for skill in &skills {
            let runtime = format!("{:?}", skill.manifest.runtime.runtime_type);
            let tools_count = skill.manifest.tools.provided.len();
            let enabled = if skill.enabled { "" } else { " [disabled]" };
            msg.push_str(&format!(
                "  {} — {} ({}, {} tool(s)){}\n",
                skill.manifest.skill.name,
                skill.manifest.skill.description,
                runtime,
                tools_count,
                enabled,
            ));
        }
        msg
    }

    async fn list_hands_text(&self) -> String {
        let defs = self.kernel.hand_registry.list_definitions();
        if defs.is_empty() {
            return "No hands available.".to_string();
        }
        let instances = self.kernel.hand_registry.list_instances();
        let mut msg = format!("Available hands ({}):\n", defs.len());
        for d in &defs {
            let reqs_met = self
                .kernel
                .hand_registry
                .check_requirements(&d.id)
                .map(|r| r.iter().all(|(_, ok)| *ok))
                .unwrap_or(false);
            let badge = if reqs_met { "Ready" } else { "Setup needed" };
            msg.push_str(&format!(
                "  {} {} — {} [{}]\n",
                d.icon, d.name, d.description, badge
            ));
        }
        if !instances.is_empty() {
            msg.push_str(&format!("\nActive ({}):\n", instances.len()));
            for i in &instances {
                msg.push_str(&format!(
                    "  {} — {} ({})\n",
                    i.agent_name, i.hand_id, i.status
                ));
            }
        }
        msg
    }

    // ── Automation: workflows, triggers, schedules, approvals ──

    async fn list_workflows_text(&self) -> String {
        let workflows = self.kernel.workflows.list_workflows().await;
        if workflows.is_empty() {
            return "No workflows defined.".to_string();
        }
        let mut msg = format!("Workflows ({}):\n", workflows.len());
        for wf in &workflows {
            let steps = wf.steps.len();
            let desc = if wf.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", wf.description)
            };
            msg.push_str(&format!("  {} ({} step(s)){}\n", wf.name, steps, desc));
        }
        msg
    }

    async fn run_workflow_text(&self, name: &str, input: &str) -> String {
        let workflows = self.kernel.workflows.list_workflows().await;
        let wf = match workflows.iter().find(|w| w.name.eq_ignore_ascii_case(name)) {
            Some(w) => w.clone(),
            None => return format!("Workflow '{name}' not found. Use /workflows to list."),
        };

        let run_id = match self
            .kernel
            .workflows
            .create_run(wf.id, input.to_string())
            .await
        {
            Some(id) => id,
            None => return "Failed to create workflow run.".to_string(),
        };

        let kernel = self.kernel.clone();
        let registry_ref = &self.kernel.registry;
        let result = self
            .kernel
            .workflows
            .execute_run(
                run_id,
                |step_agent| match step_agent {
                    captain_kernel::workflow::StepAgent::ById { id } => {
                        let aid: AgentId = id.parse().ok()?;
                        let entry = registry_ref.get(aid)?;
                        Some((aid, entry.name.clone()))
                    }
                    captain_kernel::workflow::StepAgent::ByName { name } => {
                        let entry = registry_ref.find_by_name(name)?;
                        Some((entry.id, entry.name.clone()))
                    }
                },
                |agent_id, message| {
                    let k = kernel.clone();
                    async move {
                        let result = k
                            .send_message(agent_id, &message)
                            .await
                            .map_err(|e| format!("{e}"))?;
                        Ok((
                            result.response,
                            result.total_usage.input_tokens,
                            result.total_usage.output_tokens,
                        ))
                    }
                },
            )
            .await;

        match result {
            Ok(output) => format!("Workflow '{}' completed:\n{}", wf.name, output),
            Err(e) => format!("Workflow '{}' failed: {}", wf.name, e),
        }
    }

    async fn list_triggers_text(&self) -> String {
        let triggers = self.kernel.triggers.list_all();
        if triggers.is_empty() {
            return "No triggers configured.".to_string();
        }
        let mut msg = format!("Triggers ({}):\n", triggers.len());
        for t in &triggers {
            let agent_name = self
                .kernel
                .registry
                .get(t.agent_id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| t.agent_id.to_string());
            let status = if t.enabled { "on" } else { "off" };
            let id_str = t.id.0.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            msg.push_str(&format!(
                "  [{}] {} -> {} ({:?}) fires:{} [{}]\n",
                id_short,
                agent_name,
                t.prompt_template.chars().take(40).collect::<String>(),
                t.pattern,
                t.fire_count,
                status,
            ));
        }
        msg
    }

    async fn create_trigger_text(
        &self,
        agent_name: &str,
        pattern_str: &str,
        prompt: &str,
    ) -> String {
        let agent = match self.kernel.registry.find_by_name(agent_name) {
            Some(e) => e,
            None => return format!("Agent '{agent_name}' not found."),
        };

        let pattern = match parse_trigger_pattern(pattern_str) {
            Some(p) => p,
            None => {
                return format!(
                    "Unknown pattern '{pattern_str}'. Valid: lifecycle, spawned:<name>, terminated, \
                 system, system:<keyword>, memory, memory:<key>, match:<text>, all"
                );
            }
        };

        let trigger_id = self
            .kernel
            .triggers
            .register(agent.id, pattern, prompt.to_string(), 0);
        let id_str = trigger_id.0.to_string();
        let id_short = safe_truncate_str(&id_str, 8);
        format!("Trigger created [{id_short}] for agent '{agent_name}'.")
    }

    async fn delete_trigger_text(&self, id_prefix: &str) -> String {
        let triggers = self.kernel.triggers.list_all();
        let matched: Vec<_> = triggers
            .iter()
            .filter(|t| t.id.0.to_string().starts_with(id_prefix))
            .collect();
        match matched.len() {
            0 => format!("No trigger found matching '{id_prefix}'."),
            1 => {
                let t = matched[0];
                if self.kernel.triggers.remove(t.id) {
                    let id_str = t.id.0.to_string();
                    format!("Trigger [{}] removed.", safe_truncate_str(&id_str, 8))
                } else {
                    "Failed to remove trigger.".to_string()
                }
            }
            n => format!("{n} triggers match '{id_prefix}'. Be more specific."),
        }
    }

    async fn list_schedules_text(&self) -> String {
        let jobs = self.kernel.cron_scheduler.list_all_jobs();
        if jobs.is_empty() {
            return "No scheduled jobs.".to_string();
        }
        let mut msg = format!("Cron jobs ({}):\n", jobs.len());
        for job in &jobs {
            let agent_name = self
                .kernel
                .registry
                .get(job.agent_id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| job.agent_id.to_string());
            let status = if job.enabled { "on" } else { "off" };
            let id_str = job.id.0.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            let sched = match &job.schedule {
                captain_types::scheduler::CronSchedule::Cron { expr, .. } => expr.clone(),
                captain_types::scheduler::CronSchedule::Every { every_secs } => {
                    format!("every {every_secs}s")
                }
                captain_types::scheduler::CronSchedule::At { at } => {
                    format!("at {}", at.format("%Y-%m-%d %H:%M"))
                }
            };
            let last = job
                .last_run
                .map(|t| t.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".to_string());
            msg.push_str(&format!(
                "  [{}] {} — {} ({}) last:{} [{}]\n",
                id_short, job.name, sched, agent_name, last, status,
            ));
        }
        msg
    }

    #[allow(dead_code)]
    async fn manage_schedule_text(&self, action: &str, args: &[String]) -> String {
        match action {
            "add" => add_schedule_text(self, args),
            "del" => delete_schedule_text(self, args),
            "run" => run_schedule_text(self, args).await,
            _ => "Unknown schedule action. Use: add, del, run".to_string(),
        }
    }

    async fn list_approvals_text(&self) -> String {
        let pending = self.kernel.approval_manager.list_pending();
        if pending.is_empty() {
            return "Aucune approbation en attente.".to_string();
        }
        let mut msg = format!("🛡️ Approbations en attente ({}) :\n\n", pending.len());
        for req in &pending {
            let id_str = req.id.to_string();
            let id_short = safe_truncate_str(&id_str, 8);
            let age_secs = (chrono::Utc::now() - req.requested_at).num_seconds();
            let age = if age_secs >= 60 {
                format!("{}m", age_secs / 60)
            } else {
                format!("{age_secs}s")
            };
            let risk_emoji = match req.risk_level {
                captain_types::approval::RiskLevel::Critical => "☠️",
                captain_types::approval::RiskLevel::High => "🚨",
                captain_types::approval::RiskLevel::Medium => "⚠️",
                captain_types::approval::RiskLevel::Low => "ℹ️",
            };
            msg.push_str(&format!(
                "{} [{}] {} — {} ({})\n",
                risk_emoji, id_short, req.tool_name, req.description, age,
            ));
            if !req.action_summary.is_empty() {
                msg.push_str(&format!("  → {}\n", req.action_summary));
            }
        }
        msg.push_str(
            "\n/approve <id> · /approve_session <id> · /approve_always <id> · /reject <id>",
        );
        msg
    }

    async fn resolve_approval_text_with(
        &self,
        id_prefix: &str,
        decision: captain_types::approval::ApprovalDecision,
    ) -> String {
        use captain_types::approval::ApprovalDecision;
        let pending = self.kernel.approval_manager.list_pending();
        let matched: Vec<_> = pending
            .iter()
            .filter(|r| r.id.to_string().starts_with(id_prefix))
            .collect();
        match matched.len() {
            0 => format!("Aucune approbation en attente correspondant à « {id_prefix} »."),
            1 => {
                let req = matched[0];
                match self.kernel.approval_manager.resolve(
                    req.id,
                    decision,
                    Some("channel".to_string()),
                ) {
                    Ok(_) => {
                        let verb = match decision {
                            ApprovalDecision::Approved => "✅ Approuvé (une fois)",
                            ApprovalDecision::ApprovedSession => "🕒 Approuvé (session)",
                            ApprovalDecision::ApprovedAlways => "🔒 Approuvé (toujours)",
                            ApprovalDecision::Denied => "❌ Rejeté",
                            ApprovalDecision::TimedOut => "⏱️ Expiré",
                        };
                        let id_str = req.id.to_string();
                        format!(
                            "{} — [{}] {}",
                            verb,
                            safe_truncate_str(&id_str, 8),
                            req.tool_name,
                        )
                    }
                    Err(e) => format!("Échec de la résolution : {e}"),
                }
            }
            n => format!("{n} approvals match '{id_prefix}'. Be more specific."),
        }
    }

    async fn list_learning_review_text(&self) -> String {
        let items = match self.kernel.learning_review_list(20) {
            Ok(v) => v,
            Err(e) => return format!("Impossible de lire les apprentissages en attente : {e}"),
        };
        let Some(rows) = items.as_array() else {
            return "Aucun apprentissage en attente.".to_string();
        };
        if rows.is_empty() {
            return "Aucun apprentissage en attente.".to_string();
        }
        let mut msg = format!("💭 Apprentissages en attente ({}) :\n\n", rows.len());
        for item in rows {
            let id = item["id"].as_str().unwrap_or("");
            let subject = item["subject"].as_str().unwrap_or("");
            let predicate = item["predicate"].as_str().unwrap_or("");
            let object = item["object"].as_str().unwrap_or("");
            let confidence = item["confidence"].as_f64().unwrap_or(0.0);
            msg.push_str(&format!(
                "[{}] {} {} {} ({:.0}%)\n",
                safe_truncate_str(id, 8),
                subject,
                predicate,
                object,
                confidence * 100.0,
            ));
        }
        msg.push_str("\n/learn_approve <id> · /learn_reject <id>");
        msg
    }

    async fn resolve_learning_review_text(&self, id_prefix: &str, approve: bool) -> String {
        let items = match self.kernel.learning_review_list(10_000) {
            Ok(v) => v,
            Err(e) => return format!("Impossible de lire les apprentissages en attente : {e}"),
        };
        let rows = items.as_array().cloned().unwrap_or_default();
        let matched: Vec<_> = rows
            .iter()
            .filter(|item| {
                item["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(id_prefix))
            })
            .collect();
        match matched.len() {
            0 => format!("Aucun apprentissage en attente correspondant à « {id_prefix} »."),
            1 => {
                let id = matched[0]["id"].as_str().unwrap_or(id_prefix);
                match self
                    .kernel
                    .learning_review_decide(id, approve, Some("channel"))
                    .await
                {
                    Ok(_) if approve => {
                        format!("✅ Apprentissage approuvé — [{}]", safe_truncate_str(id, 8))
                    }
                    Ok(_) => format!("❌ Apprentissage rejeté — [{}]", safe_truncate_str(id, 8)),
                    Err(e) => format!("Échec de la décision learning : {e}"),
                }
            }
            n => format!("{n} apprentissages correspondent à « {id_prefix} ». Sois plus précis."),
        }
    }

    async fn list_skill_proposals_text(&self) -> String {
        let items = match self.kernel.skill_proposal_list(20) {
            Ok(v) => v,
            Err(e) => return format!("Impossible de lire les skills proposés : {e}"),
        };
        let Some(rows) = items.as_array() else {
            return "Aucun skill proposé en attente.".to_string();
        };
        if rows.is_empty() {
            return "Aucun skill proposé en attente.".to_string();
        }
        let mut msg = format!("🛠️ Skills proposés en attente ({}) :\n\n", rows.len());
        for item in rows {
            let id = item["id"].as_str().unwrap_or("");
            let name = item["name"].as_str().unwrap_or("");
            let description = item["description"].as_str().unwrap_or("");
            let family = item["family"].as_str().unwrap_or("general-automation");
            let confidence = item["confidence"].as_f64().unwrap_or(0.0);
            msg.push_str(&format!(
                "[{}] {} — {} · famille: {} ({:.0}%)\n",
                safe_truncate_str(id, 8),
                name,
                safe_truncate_str(description, 90),
                family,
                confidence * 100.0,
            ));
        }
        msg.push_str("\n/skill_approve <id> schema diff tests human · /skill_reject <id>");
        msg
    }

    async fn resolve_skill_proposal_text(
        &self,
        id_prefix: &str,
        approve: bool,
        external_validation: bool,
    ) -> String {
        let decided_by = match skill_proposal_channel_decided_by(approve, external_validation) {
            Ok(value) => value,
            Err(usage) => return usage.to_string(),
        };
        let items = match self.kernel.skill_proposal_list(10_000) {
            Ok(v) => v,
            Err(e) => return format!("Impossible de lire les skills proposés : {e}"),
        };
        let rows = items.as_array().cloned().unwrap_or_default();
        let matched: Vec<_> = rows
            .iter()
            .filter(|item| {
                item["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(id_prefix))
            })
            .collect();
        match matched.len() {
            0 => format!("Aucun skill proposé correspondant à « {id_prefix} »."),
            1 => {
                let id = matched[0]["id"].as_str().unwrap_or(id_prefix);
                match self
                    .kernel
                    .skill_proposal_decide(id, approve, Some(decided_by.as_str()))
                    .await
                {
                    Ok(_) if approve => format!(
                        "✅ Skill approuvé et mis en quarantaine — [{}]",
                        safe_truncate_str(id, 8)
                    ),
                    Ok(_) => format!("❌ Skill proposé rejeté — [{}]", safe_truncate_str(id, 8)),
                    Err(e) => format!("Échec de la décision skill : {e}"),
                }
            }
            n => format!("{n} skills proposés correspondent à « {id_prefix} ». Sois plus précis."),
        }
    }

    async fn list_skill_refinements_text(&self) -> String {
        let items = match load_skill_refinement_registry(self.kernel.as_ref()) {
            Ok(v) => v,
            Err(e) => return format!("Impossible de lire les raffinements de skills : {e}"),
        };
        let pending: Vec<_> = items
            .iter()
            .rev()
            .filter(|item| item["status"].as_str() == Some("pending"))
            .take(20)
            .collect();
        if pending.is_empty() {
            return "Aucun raffinement de skill en attente.".to_string();
        }
        let mut msg = format!(
            "🛠️ Raffinements de skills en attente ({}) :\n\n",
            pending.len()
        );
        for item in pending {
            let id = item["id"].as_str().unwrap_or("");
            let skill = item["skill"].as_str().unwrap_or("");
            let finding = item["finding"].as_str().unwrap_or("");
            let risk = item["risk"].as_str().unwrap_or("medium");
            msg.push_str(&format!(
                "[{}] {} — {} ({risk})\n",
                safe_truncate_str(id, 8),
                skill,
                safe_truncate_str(finding, 90),
            ));
        }
        msg.push_str("\n/skill_refine_approve <id> · /skill_refine_reject <id>");
        msg
    }

    async fn resolve_skill_refinement_text(&self, id_prefix: &str, approve: bool) -> String {
        let mut items = match load_skill_refinement_registry(self.kernel.as_ref()) {
            Ok(v) => v,
            Err(e) => return format!("Impossible de lire les raffinements de skills : {e}"),
        };
        let index = match resolve_skill_refinement_index(&items, id_prefix) {
            Ok(index) => index,
            Err(e) => return e,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let Some(refinement) = items.get_mut(index).and_then(|v| v.as_object_mut()) else {
            return "Registre des raffinements corrompu : item invalide.".to_string();
        };
        let id = refinement
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(id_prefix)
            .to_string();
        let status = if approve { "approved" } else { "denied" };
        refinement.insert(
            "status".to_string(),
            serde_json::Value::String(status.to_string()),
        );
        refinement.insert(
            "updated_at".to_string(),
            serde_json::Value::String(now.clone()),
        );
        refinement.insert(
            "decided_at".to_string(),
            serde_json::Value::String(now.clone()),
        );
        let note = serde_json::json!({
            "at": now,
            "note": "Decision depuis un canal interactif"
        });
        match refinement.get_mut("notes").and_then(|v| v.as_array_mut()) {
            Some(notes) => notes.push(note),
            None => {
                refinement.insert("notes".to_string(), serde_json::json!([note]));
            }
        }
        if let Err(e) = store_skill_refinement_registry(self.kernel.as_ref(), items) {
            return format!("Échec de la décision raffinement : {e}");
        }
        if approve {
            format!(
                "✅ Raffinement de skill approuvé — [{}]\nSnapshot prêt si le skill est file-backed.",
                safe_truncate_str(&id, 8)
            )
        } else {
            format!(
                "❌ Raffinement de skill rejeté — [{}]",
                safe_truncate_str(&id, 8)
            )
        }
    }

    async fn resolve_project_ask_text(&self, ask_id_prefix: &str, answer: &str) -> String {
        match crate::project_ask::answer_project_ask_with_receipt(ask_id_prefix, answer).await {
            Ok(receipt) => {
                let runtime_note = record_project_ask_answer_runtime(
                    self.kernel.memory.as_ref(),
                    Some(&receipt.meta.project_id),
                    &receipt.ask_id,
                    &receipt.answer,
                    "delivered_to_active_worker",
                )
                .map(|_| String::new())
                .unwrap_or_else(|e| {
                    format!(
                        "\n⚠️ Réponse transmise, mais l'état runtime n'a pas été mis à jour : {e}"
                    )
                });
                format!(
                    "✅ Réponse envoyée au projet « {} » [{}] : {}{}",
                    receipt.meta.project_name,
                    safe_truncate_str(&receipt.ask_id, 8),
                    receipt.answer,
                    runtime_note
                )
            }
            Err(active_error) => match record_project_ask_answer_runtime(
                self.kernel.memory.as_ref(),
                None,
                ask_id_prefix,
                answer,
                "recorded_for_resume",
            ) {
                Ok((project, receipt)) => format!(
                    "✅ Réponse enregistrée pour le projet « {} » [{}] : {}\nLe runtime est marqué pour reprise : le prochain Start/Resume reprendra cette phase sans réinitialiser les phases déjà terminées.",
                    project.name,
                    safe_truncate_str(&receipt.ask_id, 8),
                    receipt.answer
                ),
                Err(runtime_error) => format!(
                    "Impossible de répondre à la question projet : {active_error}. Reprise persistée impossible : {runtime_error}"
                ),
            },
        }
    }

    async fn reset_session(&self, agent_id: AgentId) -> Result<String, String> {
        self.kernel
            .reset_session(agent_id)
            .map_err(|e| format!("{e}"))?;
        Ok("New session started. The previous session remains available in history.".to_string())
    }

    async fn compact_session(&self, agent_id: AgentId) -> Result<String, String> {
        self.kernel
            .compact_agent_session(agent_id)
            .await
            .map_err(|e| format!("{e}"))
    }

    async fn set_model(&self, agent_id: AgentId, model: &str) -> Result<String, String> {
        if model.is_empty() {
            // Show current model
            let entry = self
                .kernel
                .registry
                .get(agent_id)
                .ok_or_else(|| "Agent not found".to_string())?;
            return Ok(format!(
                "Current model: {} (provider: {})",
                entry.manifest.model.model, entry.manifest.model.provider
            ));
        }
        self.kernel
            .set_agent_model(agent_id, model, None)
            .map_err(|e| format!("{e}"))?;
        // Read back resolved model+provider from registry
        let entry = self
            .kernel
            .registry
            .get(agent_id)
            .ok_or_else(|| "Agent not found after model switch".to_string())?;
        Ok(format!(
            "Model switched to: {} (provider: {})",
            entry.manifest.model.model, entry.manifest.model.provider
        ))
    }

    async fn model_switch_plan(
        &self,
        agent_id: AgentId,
        target_model: &str,
    ) -> Result<serde_json::Value, String> {
        self.kernel
            .plan_model_switch(agent_id, target_model.trim(), None)
            .map(|plan| serde_json::json!(plan))
            .map_err(|e| format!("{e}"))
    }

    async fn model_switch_apply(
        &self,
        agent_id: AgentId,
        target_model: &str,
        target_provider: Option<&str>,
        session_strategy: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let target_model = target_model.trim();
        let strategy = match session_strategy {
            Some(strategy) => strategy.parse::<ModelSwitchSessionStrategy>()?,
            None => {
                self.kernel
                    .plan_model_switch(agent_id, target_model, target_provider)
                    .map_err(|e| format!("{e}"))?
                    .recommended_session_strategy
            }
        };
        self.kernel
            .apply_model_switch(agent_id, target_model, target_provider, strategy)
            .map(|result| serde_json::json!(result))
            .map_err(|e| format!("{e}"))
    }

    async fn stop_run(&self, agent_id: AgentId) -> Result<String, String> {
        let cancelled = self
            .kernel
            .stop_agent_run(agent_id)
            .map_err(|e| format!("{e}"))?;
        if cancelled {
            if self
                .kernel
                .config
                .language
                .to_ascii_lowercase()
                .starts_with("fr")
            {
                Ok("Run annulé.".to_string())
            } else {
                Ok("Run cancelled.".to_string())
            }
        } else if self
            .kernel
            .config
            .language
            .to_ascii_lowercase()
            .starts_with("fr")
        {
            Ok("Aucun run actif à annuler.".to_string())
        } else {
            Ok("No active run to cancel.".to_string())
        }
    }

    async fn session_usage(&self, agent_id: AgentId) -> Result<String, String> {
        let (input, output, cost) = self
            .kernel
            .session_usage_cost(agent_id)
            .map_err(|e| format!("{e}"))?;
        let total = input + output;
        let mut msg = format!(
            "Session usage:\n  Input: ~{input} tokens\n  Output: ~{output} tokens\n  Total: ~{total} tokens"
        );
        if cost > 0.0 {
            msg.push_str(&format!("\n  Estimated cost: ${cost:.4}"));
        }
        Ok(msg)
    }

    async fn set_thinking(&self, _agent_id: AgentId, on: bool) -> Result<String, String> {
        // Future-ready: stores preference but doesn't affect model behavior yet
        let state = if on { "enabled" } else { "disabled" };
        Ok(format!(
            "Extended thinking {state}. (This will take effect when supported by the model.)"
        ))
    }

    async fn channel_overrides(
        &self,
        channel_type: &str,
    ) -> Option<captain_types::config::ChannelOverrides> {
        let channels = &self.kernel.config.channels;
        match channel_type {
            "telegram" => channels.telegram.as_ref().map(|c| c.overrides.clone()),
            "discord" => channels.discord.as_ref().map(|c| c.overrides.clone()),
            "slack" => channels.slack.as_ref().map(|c| c.overrides.clone()),
            "whatsapp" => channels.whatsapp.as_ref().map(|c| c.overrides.clone()),
            "signal" => channels.signal.as_ref().map(|c| c.overrides.clone()),
            "matrix" => channels.matrix.as_ref().map(|c| c.overrides.clone()),
            "email" => channels.email.as_ref().map(|c| c.overrides.clone()),
            "teams" => channels.teams.as_ref().map(|c| c.overrides.clone()),
            "mattermost" => channels.mattermost.as_ref().map(|c| c.overrides.clone()),
            "irc" => channels.irc.as_ref().map(|c| c.overrides.clone()),
            "google_chat" => channels.google_chat.as_ref().map(|c| c.overrides.clone()),
            "twitch" => channels.twitch.as_ref().map(|c| c.overrides.clone()),
            "rocketchat" => channels.rocketchat.as_ref().map(|c| c.overrides.clone()),
            "zulip" => channels.zulip.as_ref().map(|c| c.overrides.clone()),
            "xmpp" => channels.xmpp.as_ref().map(|c| c.overrides.clone()),
            // Wave 3
            "line" => channels.line.as_ref().map(|c| c.overrides.clone()),
            "viber" => channels.viber.as_ref().map(|c| c.overrides.clone()),
            "messenger" => channels.messenger.as_ref().map(|c| c.overrides.clone()),
            "reddit" => channels.reddit.as_ref().map(|c| c.overrides.clone()),
            "mastodon" => channels.mastodon.as_ref().map(|c| c.overrides.clone()),
            "bluesky" => channels.bluesky.as_ref().map(|c| c.overrides.clone()),
            "feishu" => channels.feishu.as_ref().map(|c| c.overrides.clone()),
            "revolt" => channels.revolt.as_ref().map(|c| c.overrides.clone()),
            // Wave 4
            "nextcloud" => channels.nextcloud.as_ref().map(|c| c.overrides.clone()),
            "guilded" => channels.guilded.as_ref().map(|c| c.overrides.clone()),
            "keybase" => channels.keybase.as_ref().map(|c| c.overrides.clone()),
            "threema" => channels.threema.as_ref().map(|c| c.overrides.clone()),
            "nostr" => channels.nostr.as_ref().map(|c| c.overrides.clone()),
            "webex" => channels.webex.as_ref().map(|c| c.overrides.clone()),
            "pumble" => channels.pumble.as_ref().map(|c| c.overrides.clone()),
            "flock" => channels.flock.as_ref().map(|c| c.overrides.clone()),
            "twist" => channels.twist.as_ref().map(|c| c.overrides.clone()),
            // Wave 5
            "mumble" => channels.mumble.as_ref().map(|c| c.overrides.clone()),
            "dingtalk" => channels.dingtalk.as_ref().map(|c| c.overrides.clone()),
            "dingtalk_stream" => channels
                .dingtalk_stream
                .as_ref()
                .map(|c| c.overrides.clone()),
            "discourse" => channels.discourse.as_ref().map(|c| c.overrides.clone()),
            "gitter" => channels.gitter.as_ref().map(|c| c.overrides.clone()),
            "ntfy" => channels.ntfy.as_ref().map(|c| c.overrides.clone()),
            "gotify" => channels.gotify.as_ref().map(|c| c.overrides.clone()),
            "webhook" => channels.webhook.as_ref().map(|c| c.overrides.clone()),
            "linkedin" => channels.linkedin.as_ref().map(|c| c.overrides.clone()),
            "wecom" => channels.wecom.as_ref().map(|c| c.overrides.clone()),
            _ => None,
        }
    }

    async fn authorize_channel_user(
        &self,
        channel_type: &str,
        platform_id: &str,
        action: &str,
    ) -> Result<(), String> {
        if !self.kernel.auth.is_enabled() {
            return Ok(()); // RBAC not configured — allow all
        }

        let user_id = self
            .kernel
            .auth
            .identify(channel_type, platform_id)
            .ok_or_else(|| "Unrecognized user. Contact an admin to get access.".to_string())?;

        let auth_action = match action {
            "chat" => captain_kernel::auth::Action::ChatWithAgent,
            "spawn" => captain_kernel::auth::Action::SpawnAgent,
            "kill" => captain_kernel::auth::Action::KillAgent,
            "install_skill" => captain_kernel::auth::Action::InstallSkill,
            _ => captain_kernel::auth::Action::ChatWithAgent,
        };

        self.kernel
            .auth
            .authorize(user_id, &auth_action)
            .map_err(|e| e.to_string())
    }

    async fn record_delivery(
        &self,
        agent_id: AgentId,
        channel: &str,
        recipient: &str,
        success: bool,
        error: Option<&str>,
        thread_id: Option<&str>,
    ) {
        let receipt = if success {
            captain_kernel::DeliveryTracker::sent_receipt(channel, recipient)
        } else {
            captain_kernel::DeliveryTracker::failed_receipt(
                channel,
                recipient,
                error.unwrap_or("Unknown error"),
            )
        };
        self.kernel.delivery_tracker.record(agent_id, receipt);

        // Persist last channel for cron CronDelivery::LastChannel.
        // Include thread_id when present so forum-topic context survives restarts.
        if success {
            let mut kv_val = serde_json::json!({"channel": channel, "recipient": recipient});
            if let Some(tid) = thread_id {
                kv_val["thread_id"] = serde_json::json!(tid);
            }
            let _ = self
                .kernel
                .memory
                .structured_set(agent_id, "delivery.last_channel", kv_val);
        }
    }

    async fn get_agent_for_topic(&self, thread_id: &str) -> Option<AgentId> {
        // Look up topic→agent mapping from structured memory
        let key = format!("topic_agent:{}", thread_id);
        if let Ok(Some(serde_json::Value::String(agent_id_str))) = self
            .kernel
            .memory
            .structured_get(AgentId(uuid::Uuid::nil()), &key)
        {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id_str) {
                let aid = AgentId(uuid);
                if self.kernel.registry.get(aid).is_some() {
                    return Some(aid);
                }
            }
        }
        // Also check reverse mapping from telegram_topic:agent_name
        // Iterate known agents and check if any has this topic
        for entry in self.kernel.registry.list() {
            if let Some(topic) = self.kernel.get_telegram_topic(&entry.name) {
                if topic == thread_id {
                    return Some(entry.id);
                }
            }
        }
        None
    }

    async fn set_topic_agent(&self, thread_id: &str, agent_id: AgentId) {
        let key = format!("topic_agent:{}", thread_id);
        let _ = self.kernel.memory.structured_set(
            AgentId(uuid::Uuid::nil()),
            &key,
            serde_json::Value::String(agent_id.to_string()),
        );
        // Also set the reverse mapping for the agent name
        if let Some(entry) = self.kernel.registry.get(agent_id) {
            self.kernel.set_telegram_topic(&entry.name, thread_id);
        }
        info!(thread_id = %thread_id, agent_id = %agent_id, "Topic→agent mapping persisted");
    }

    async fn list_topic_mappings(&self) -> Vec<(String, AgentId, String)> {
        let mut mappings = Vec::new();
        for entry in self.kernel.registry.list() {
            if let Some(topic_id) = self.kernel.get_telegram_topic(&entry.name) {
                mappings.push((topic_id, entry.id, entry.name.clone()));
            }
        }
        mappings
    }

    async fn check_auto_reply(&self, agent_id: AgentId, message: &str) -> Option<String> {
        // Check if auto-reply should fire for this message
        let channel_type = "bridge"; // Generic; the bridge layer handles specifics
        self.kernel
            .auto_reply_engine
            .should_reply(message, channel_type, agent_id)?;
        // Fire auto-reply synchronously (bridge already runs in background task)
        match self.kernel.send_message(agent_id, message).await {
            Ok(result) => Some(result.response),
            Err(e) => {
                tracing::warn!(error = %e, "Auto-reply failed");
                None
            }
        }
    }

    async fn recent_agent_diagnostics(
        &self,
        agent_id: AgentId,
        _channel_type: &str,
    ) -> Option<String> {
        let entry = self.kernel.registry.get(agent_id)?;
        let session = match self.kernel.memory.get_session(entry.session_id) {
            Ok(Some(session)) => session,
            Ok(None) => return Some("- Aucune session persistée pour cet agent.".to_string()),
            Err(e) => return Some(format!("- Lecture session impossible: {e}.")),
        };
        Some(recent_session_diagnostics(&session))
    }

    // ── Budget, Network, A2A ──

    async fn budget_text(&self) -> String {
        let budget = &self.kernel.config.budget;
        let status = self.kernel.metering.budget_status(budget);

        let fmt_limit = |v: f64| -> String {
            if v > 0.0 {
                format!("${v:.2}")
            } else {
                "unlimited".to_string()
            }
        };
        let fmt_pct = |pct: f64, limit: f64| -> String {
            if limit > 0.0 {
                format!(" ({:.1}%)", pct * 100.0)
            } else {
                String::new()
            }
        };

        format!(
            "Budget Status:\n\
             \n\
             Hourly:  ${:.4} / {}{}\n\
             Daily:   ${:.4} / {}{}\n\
             Monthly: ${:.4} / {}{}\n\
             \n\
             Alert threshold: {}%",
            status.hourly_spend,
            fmt_limit(status.hourly_limit),
            fmt_pct(status.hourly_pct, status.hourly_limit),
            status.daily_spend,
            fmt_limit(status.daily_limit),
            fmt_pct(status.daily_pct, status.daily_limit),
            status.monthly_spend,
            fmt_limit(status.monthly_limit),
            fmt_pct(status.monthly_pct, status.monthly_limit),
            (status.alert_threshold * 100.0) as u32,
        )
    }

    async fn peers_text(&self) -> String {
        if !self.kernel.config.network_enabled {
            return "OFP peer network is disabled. Set network_enabled = true in config.toml."
                .to_string();
        }
        match self.kernel.peer_registry.get() {
            Some(registry) => {
                let peers = registry.all_peers();
                if peers.is_empty() {
                    "OFP network enabled but no peers connected.".to_string()
                } else {
                    let mut msg = format!("OFP Peers ({} connected):\n", peers.len());
                    for p in &peers {
                        msg.push_str(&format!(
                            "  {} — {} ({:?})\n",
                            p.node_id, p.address, p.state
                        ));
                    }
                    msg
                }
            }
            None => "OFP peer node not started.".to_string(),
        }
    }

    async fn a2a_agents_text(&self) -> String {
        let agents = self
            .kernel
            .a2a_external_agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if agents.is_empty() {
            return "No external A2A agents discovered.\nUse the API to discover agents."
                .to_string();
        }
        let mut msg = format!("External A2A Agents ({}):\n", agents.len());
        for (url, card) in agents.iter() {
            msg.push_str(&format!("  {} — {}\n", card.name, url));
            let desc = &card.description;
            if !desc.is_empty() {
                let short = captain_types::truncate_str(desc, 60);
                msg.push_str(&format!("    {short}\n"));
            }
        }
        msg
    }

    // ── Home channel routing (v3.8h) ──
    async fn set_home_channel(
        &self,
        channel: &str,
        user_platform_id: &str,
        chat_id: &str,
    ) -> Result<String, String> {
        let path = home_channels_file(&self.kernel.config.home_dir);
        let mut map = load_home_channels(&path);
        map.insert(home_key(channel, user_platform_id), chat_id.to_string());
        save_home_channels(&path, &map)?;
        Ok(format!(
            "home_channel_set channel={channel} chat_id={chat_id}"
        ))
    }

    async fn get_home_channel(&self, channel: &str, user_platform_id: &str) -> Option<String> {
        let path = home_channels_file(&self.kernel.config.home_dir);
        let map = load_home_channels(&path);
        map.get(&home_key(channel, user_platform_id)).cloned()
    }
}

/// Storage location for the JSON home-channel map (v3.8h).
fn home_channels_file(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir.join("home_channels.json")
}

/// Composite key keeps multiple channels per user distinct.
fn home_key(channel: &str, user_platform_id: &str) -> String {
    format!("{channel}:{user_platform_id}")
}

fn load_home_channels(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_home_channels(
    path: &std::path::Path,
    map: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create home-channels dir: {e}"))?;
    }
    let data = serde_json::to_string_pretty(map)
        .map_err(|e| format!("Failed to serialize home channels: {e}"))?;
    std::fs::write(path, data).map_err(|e| format!("Failed to write home channels file: {e}"))
}

/// Parse a trigger pattern string from chat into a `TriggerPattern`.
fn parse_trigger_pattern(s: &str) -> Option<captain_kernel::triggers::TriggerPattern> {
    use captain_kernel::triggers::TriggerPattern;
    if let Some(rest) = s.strip_prefix("spawned:") {
        return Some(TriggerPattern::AgentSpawned {
            name_pattern: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("system:") {
        return Some(TriggerPattern::SystemKeyword {
            keyword: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("memory:") {
        return Some(TriggerPattern::MemoryKeyPattern {
            key_pattern: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("match:") {
        return Some(TriggerPattern::ContentMatch {
            substring: rest.to_string(),
        });
    }
    match s {
        "lifecycle" => Some(TriggerPattern::Lifecycle),
        "terminated" => Some(TriggerPattern::AgentTerminated),
        "system" => Some(TriggerPattern::System),
        "memory" => Some(TriggerPattern::MemoryUpdate),
        "all" => Some(TriggerPattern::All),
        _ => None,
    }
}

/// Resolve a token: if the value looks like an actual secret (contains `:`,
/// starts with `xoxb-`, `xapp-`, `sk-`, etc.), use it directly.
/// Otherwise treat it as an env var name and look it up.
fn read_token(env_var_or_token: &str, adapter_name: &str) -> Option<String> {
    // Heuristic: actual tokens contain `:` (Telegram, Discord) or start with
    // known prefixes. Env var names are uppercase ASCII identifiers.
    let looks_like_token = env_var_or_token.contains(':')
        || env_var_or_token.starts_with("xoxb-")
        || env_var_or_token.starts_with("xapp-")
        || env_var_or_token.starts_with("sk-")
        || env_var_or_token.starts_with("Bearer ")
        || env_var_or_token.len() > 80; // Long random strings are tokens, not env var names

    if looks_like_token {
        warn!(
            "{adapter_name}: config field contains what looks like an actual token \
             rather than an env var name — using it directly. \
             Tip: store the token in an env var and use the var name instead for security."
        );
        return Some(env_var_or_token.to_string());
    }

    match std::env::var(env_var_or_token) {
        Ok(t) if !t.is_empty() => Some(t),
        Ok(_) => {
            warn!("{adapter_name} token env var '{env_var_or_token}' is set but empty, skipping");
            None
        }
        Err(_) => {
            warn!(
                "{adapter_name} token env var '{env_var_or_token}' not set, skipping. \
                 Set it with: export {env_var_or_token}=<your-token>"
            );
            None
        }
    }
}

/// Re-read `secrets.env` and inject every key/value into `std::env`.
///
/// Used by both `reload_channels_from_disk` and the IntegrationConfigured
/// hot-reload listener so a freshly-edited token (manual edit OR via
/// `secret_write`) is visible to `read_token`'s `std::env::var` lookup
/// without a daemon restart. Always overwrites — the file is the source
/// of truth after a configure step.
pub fn reload_secrets_into_env(home_dir: &std::path::Path) {
    let secrets_path = home_dir.join("secrets.env");
    let Ok(content) = std::fs::read_to_string(&secrets_path) else {
        return;
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq_pos].trim();
        if key.is_empty() {
            continue;
        }
        let mut value = trimmed[eq_pos + 1..].trim().to_string();
        if ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
            && value.len() >= 2
        {
            value = value[1..value.len() - 1].to_string();
        }
        std::env::set_var(key, &value);
    }
    info!("Reloaded secrets.env into process env");
}

/// Start the channel bridge for all configured channels based on kernel config.
///
/// Returns `Some(BridgeManager)` if any channels were configured and started,
/// or `None` if no channels are configured.
pub async fn start_channel_bridge(kernel: Arc<CaptainKernel>) -> Option<BridgeManager> {
    let channels = kernel.config.channels.clone();
    let (bridge, _names) = start_channel_bridge_with_config(kernel, &channels).await;
    bridge
}

fn channel_config_has_any(config: &captain_types::config::ChannelsConfig) -> bool {
    crate::channel_runtime_policy::has_active_channel_config(config)
}

fn push_active_channel_adapters(
    kernel: &Arc<CaptainKernel>,
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    push_telegram_adapter(kernel, config, adapters);
    push_discord_adapter(config, adapters);
    push_signal_adapter(config, adapters);
    push_email_adapter(config, adapters);
}

fn push_telegram_adapter(
    kernel: &Arc<CaptainKernel>,
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(tg_config) = config.telegram.as_ref() else {
        return;
    };
    let Some(token) = read_token(&tg_config.bot_token_env, "Telegram") else {
        return;
    };
    let poll_interval = Duration::from_secs(tg_config.poll_interval_secs);
    let adapter = Arc::new(TelegramAdapter::new(
        token,
        tg_config.allowed_users.clone(),
        poll_interval,
        tg_config.api_url.clone(),
    ));
    if let Some(chat_id_str) = tg_config.default_chat_id.clone() {
        let routing_adapter: Arc<dyn ChannelAdapter> = adapter.clone();
        captain_kernel::channel_routing::spawn_telegram_memory_routing(
            kernel.event_bus.clone(),
            routing_adapter,
            chat_id_str.clone(),
        );
        if let Ok(chat_id_int) = chat_id_str.parse::<i64>() {
            captain_kernel::channel_routing::spawn_telegram_memory_approval_routing(
                kernel.event_bus.clone(),
                adapter.clone(),
                chat_id_int,
            );
            captain_kernel::channel_routing::spawn_telegram_skill_proposal_routing(
                kernel.event_bus.clone(),
                adapter.clone(),
                chat_id_int,
            );
            captain_kernel::channel_routing::spawn_telegram_skill_refinement_routing(
                kernel.event_bus.clone(),
                adapter.clone(),
                chat_id_int,
            );
            captain_kernel::channel_routing::spawn_telegram_project_ask_routing(
                kernel.event_bus.clone(),
                adapter.clone(),
                chat_id_int,
            );
        } else {
            warn!(
                chat_id = %chat_id_str,
                "telegram default_chat_id is not numeric — approval routing disabled"
            );
        }
    }
    adapters.push((adapter, tg_config.default_agent.clone()));
}

fn push_discord_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(dc_config) = config.discord.as_ref() else {
        return;
    };
    let Some(token) = read_token(&dc_config.bot_token_env, "Discord") else {
        return;
    };
    let adapter = Arc::new(DiscordAdapter::new(
        token,
        dc_config.allowed_guilds.clone(),
        dc_config.allowed_users.clone(),
        dc_config.ignore_bots,
        dc_config.intents,
    ));
    adapters.push((adapter, dc_config.default_agent.clone()));
}

fn push_signal_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(sig_config) = config.signal.as_ref() else {
        return;
    };
    if sig_config.phone_number.is_empty() {
        warn!("Signal configured but phone_number is empty, skipping");
        return;
    }
    let adapter = Arc::new(SignalAdapter::new(
        sig_config.api_url.clone(),
        sig_config.phone_number.clone(),
        sig_config.allowed_users.clone(),
    ));
    adapters.push((adapter, sig_config.default_agent.clone()));
}

fn push_email_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(em_config) = config.email.as_ref() else {
        return;
    };
    let Some(password) = read_token(&em_config.password_env, "Email") else {
        return;
    };
    if em_config.allowed_senders.is_empty() {
        warn!(
            "Email bridge will deny every inbound message: \
             [channels.email] has no `allowed_senders`. Add \
             `allowed_senders = [\"@example.org\"]` (or `[\"*\"]` to \
             opt back into the legacy permissive default)."
        );
    }
    let adapter = Arc::new(EmailAdapter::new(
        em_config.imap_host.clone(),
        em_config.imap_port,
        em_config.smtp_host.clone(),
        em_config.smtp_port,
        em_config.username.clone(),
        password,
        em_config.poll_interval_secs,
        em_config.folders.clone(),
        em_config.allowed_senders.clone(),
    ));
    adapters.push((adapter, em_config.default_agent.clone()));
}

fn push_frozen_channel_adapters(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    push_slack_adapter(config, adapters);
    push_whatsapp_adapter(config, adapters);
    push_matrix_adapter(config, adapters);
    push_teams_adapter(config, adapters);
    push_mattermost_adapter(config, adapters);
    push_irc_adapter(config, adapters);
    push_google_chat_adapter(config, adapters);
    push_twitch_adapter(config, adapters);
    push_rocketchat_adapter(config, adapters);
    push_zulip_adapter(config, adapters);
    push_xmpp_adapter(config, adapters);
    push_line_adapter(config, adapters);
    push_viber_adapter(config, adapters);
    push_messenger_adapter(config, adapters);
    push_reddit_adapter(config, adapters);
    push_mastodon_adapter(config, adapters);
    push_bluesky_adapter(config, adapters);
    push_feishu_adapter(config, adapters);
    push_revolt_adapter(config, adapters);
    push_wecom_adapter(config, adapters);
    push_nextcloud_adapter(config, adapters);
    push_guilded_adapter(config, adapters);
    push_keybase_adapter(config, adapters);
    push_threema_adapter(config, adapters);
    push_nostr_adapter(config, adapters);
    push_webex_adapter(config, adapters);
    push_pumble_adapter(config, adapters);
    push_flock_adapter(config, adapters);
    push_twist_adapter(config, adapters);
    push_mumble_adapter(config, adapters);
    push_dingtalk_adapter(config, adapters);
    push_dingtalk_stream_adapter(config, adapters);
    push_discourse_adapter(config, adapters);
    push_gitter_adapter(config, adapters);
    push_ntfy_adapter(config, adapters);
    push_gotify_adapter(config, adapters);
    push_webhook_adapter(config, adapters);
    push_linkedin_adapter(config, adapters);
}

fn push_slack_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(sl_config) = config.slack.as_ref() else {
        return;
    };
    let Some(app_token) = read_token(&sl_config.app_token_env, "Slack (app)") else {
        return;
    };
    let Some(bot_token) = read_token(&sl_config.bot_token_env, "Slack (bot)") else {
        return;
    };
    let adapter = Arc::new(SlackAdapter::new(
        app_token,
        bot_token,
        sl_config.allowed_users.clone(),
        sl_config.allowed_channels.clone(),
        sl_config.auto_thread_reply,
        sl_config.thread_ttl_hours,
        sl_config.unfurl_links,
    ));
    adapters.push((adapter, sl_config.default_agent.clone()));
}

fn push_whatsapp_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(wa_config) = config.whatsapp.as_ref() else {
        return;
    };
    let cloud_token = read_token(&wa_config.access_token_env, "WhatsApp");
    let gateway_url = std::env::var(&wa_config.gateway_url_env)
        .ok()
        .filter(|u| !u.is_empty());
    if cloud_token.is_none() && gateway_url.is_none() {
        return;
    }
    let token = cloud_token.unwrap_or_default();
    let verify_token =
        read_token(&wa_config.verify_token_env, "WhatsApp (verify)").unwrap_or_default();
    let adapter = Arc::new(
        WhatsAppAdapter::new(
            wa_config.phone_number_id.clone(),
            token,
            verify_token,
            wa_config.webhook_port,
            wa_config.allowed_users.clone(),
        )
        .with_gateway(gateway_url),
    );
    adapters.push((adapter, wa_config.default_agent.clone()));
}

fn push_matrix_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(mx_config) = config.matrix.as_ref() else {
        return;
    };
    let Some(token) = read_token(&mx_config.access_token_env, "Matrix") else {
        return;
    };
    let adapter = Arc::new(MatrixAdapter::new(
        mx_config.homeserver_url.clone(),
        mx_config.user_id.clone(),
        token,
        mx_config.allowed_users.clone(),
        mx_config.allowed_rooms.clone(),
    ));
    adapters.push((adapter, mx_config.default_agent.clone()));
}

fn push_teams_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(tm_config) = config.teams.as_ref() else {
        return;
    };
    let Some(password) = read_token(&tm_config.app_password_env, "Teams") else {
        return;
    };
    let adapter = Arc::new(TeamsAdapter::new(
        tm_config.app_id.clone(),
        password,
        tm_config.webhook_port,
        tm_config.allowed_users.clone(),
        tm_config.allowed_tenants.clone(),
    ));
    adapters.push((adapter, tm_config.default_agent.clone()));
}

fn push_mattermost_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(mm_config) = config.mattermost.as_ref() else {
        return;
    };
    let Some(token) = read_token(&mm_config.token_env, "Mattermost") else {
        return;
    };
    let adapter = Arc::new(MattermostAdapter::new(
        mm_config.server_url.clone(),
        token,
        mm_config.allowed_users.clone(),
        mm_config.allowed_channels.clone(),
    ));
    adapters.push((adapter, mm_config.default_agent.clone()));
}

fn push_irc_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(irc_config) = config.irc.as_ref() else {
        return;
    };
    if irc_config.server.is_empty() {
        warn!("IRC configured but server is empty, skipping");
        return;
    }
    let password = irc_config
        .password_env
        .as_ref()
        .and_then(|env| read_token(env, "IRC"));
    let adapter = Arc::new(IrcAdapter::new(
        irc_config.server.clone(),
        irc_config.port,
        irc_config.nick.clone(),
        password,
        irc_config.allowed_users.clone(),
        irc_config.channels.clone(),
        irc_config.use_tls,
    ));
    adapters.push((adapter, irc_config.default_agent.clone()));
}

fn push_google_chat_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(gc_config) = config.google_chat.as_ref() else {
        return;
    };
    let Some(key) = read_token(&gc_config.service_account_env, "Google Chat") else {
        return;
    };
    let adapter = Arc::new(GoogleChatAdapter::new(
        key,
        gc_config.space_ids.clone(),
        gc_config.webhook_port,
        gc_config.allowed_users.clone(),
    ));
    adapters.push((adapter, gc_config.default_agent.clone()));
}

fn push_twitch_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(tw_config) = config.twitch.as_ref() else {
        return;
    };
    let Some(token) = read_token(&tw_config.oauth_token_env, "Twitch") else {
        return;
    };
    let adapter = Arc::new(TwitchAdapter::new(
        token,
        tw_config.allowed_users.clone(),
        tw_config.channels.clone(),
        tw_config.nick.clone(),
    ));
    adapters.push((adapter, tw_config.default_agent.clone()));
}

fn push_rocketchat_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(rc_config) = config.rocketchat.as_ref() else {
        return;
    };
    let Some(token) = read_token(&rc_config.token_env, "Rocket.Chat") else {
        return;
    };
    let adapter = Arc::new(RocketChatAdapter::new(
        rc_config.server_url.clone(),
        token,
        rc_config.user_id.clone(),
        rc_config.allowed_channels.clone(),
        rc_config.allowed_users.clone(),
    ));
    adapters.push((adapter, rc_config.default_agent.clone()));
}

fn push_zulip_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(z_config) = config.zulip.as_ref() else {
        return;
    };
    let Some(api_key) = read_token(&z_config.api_key_env, "Zulip") else {
        return;
    };
    let adapter = Arc::new(ZulipAdapter::new(
        z_config.server_url.clone(),
        z_config.bot_email.clone(),
        api_key,
        z_config.streams.clone(),
        z_config.allowed_users.clone(),
    ));
    adapters.push((adapter, z_config.default_agent.clone()));
}

fn push_xmpp_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(x_config) = config.xmpp.as_ref() else {
        return;
    };
    let Some(password) = read_token(&x_config.password_env, "XMPP") else {
        return;
    };
    let adapter = Arc::new(XmppAdapter::new(
        x_config.jid.clone(),
        password,
        x_config.server.clone(),
        x_config.port,
        x_config.rooms.clone(),
        x_config.allowed_users.clone(),
    ));
    adapters.push((adapter, x_config.default_agent.clone()));
}

fn push_line_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(ln_config) = config.line.as_ref() else {
        return;
    };
    let Some(secret) = read_token(&ln_config.channel_secret_env, "LINE (secret)") else {
        return;
    };
    let Some(token) = read_token(&ln_config.access_token_env, "LINE (token)") else {
        return;
    };
    let adapter = Arc::new(LineAdapter::new(
        secret,
        token,
        ln_config.webhook_port,
        ln_config.allowed_users.clone(),
    ));
    adapters.push((adapter, ln_config.default_agent.clone()));
}

fn push_viber_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(vb_config) = config.viber.as_ref() else {
        return;
    };
    let Some(token) = read_token(&vb_config.auth_token_env, "Viber") else {
        return;
    };
    let adapter = Arc::new(ViberAdapter::new(
        token,
        vb_config.webhook_url.clone(),
        vb_config.webhook_port,
        vb_config.allowed_users.clone(),
    ));
    adapters.push((adapter, vb_config.default_agent.clone()));
}

fn push_messenger_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(ms_config) = config.messenger.as_ref() else {
        return;
    };
    let Some(page_token) = read_token(&ms_config.page_token_env, "Messenger (page)") else {
        return;
    };
    let verify_token =
        read_token(&ms_config.verify_token_env, "Messenger (verify)").unwrap_or_default();
    let adapter = Arc::new(MessengerAdapter::new(
        page_token,
        verify_token,
        ms_config.webhook_port,
        ms_config.allowed_users.clone(),
    ));
    adapters.push((adapter, ms_config.default_agent.clone()));
}

fn push_reddit_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(rd_config) = config.reddit.as_ref() else {
        return;
    };
    let Some(secret) = read_token(&rd_config.client_secret_env, "Reddit (secret)") else {
        return;
    };
    let Some(password) = read_token(&rd_config.password_env, "Reddit (password)") else {
        return;
    };
    let adapter = Arc::new(RedditAdapter::new(
        rd_config.client_id.clone(),
        secret,
        rd_config.username.clone(),
        password,
        rd_config.allowed_users.clone(),
        rd_config.subreddits.clone(),
    ));
    adapters.push((adapter, rd_config.default_agent.clone()));
}

fn push_mastodon_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(md_config) = config.mastodon.as_ref() else {
        return;
    };
    let Some(token) = read_token(&md_config.access_token_env, "Mastodon") else {
        return;
    };
    let adapter = Arc::new(MastodonAdapter::new(
        md_config.instance_url.clone(),
        token,
        md_config.allowed_users.clone(),
    ));
    adapters.push((adapter, md_config.default_agent.clone()));
}

fn push_bluesky_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(bs_config) = config.bluesky.as_ref() else {
        return;
    };
    let Some(password) = read_token(&bs_config.app_password_env, "Bluesky") else {
        return;
    };
    let adapter = Arc::new(BlueskyAdapter::new(
        bs_config.identifier.clone(),
        password,
        bs_config.allowed_users.clone(),
    ));
    adapters.push((adapter, bs_config.default_agent.clone()));
}

fn push_feishu_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(fs_config) = config.feishu.as_ref() else {
        return;
    };
    let Some(secret) = read_token(&fs_config.app_secret_env, "Feishu") else {
        return;
    };
    let region = captain_channels::feishu::FeishuRegion::parse_region(&fs_config.region);
    let encrypt_key = fs_config
        .encrypt_key_env
        .as_ref()
        .and_then(|env| read_token(env, "Feishu encrypt_key"));
    let adapter = Arc::new(FeishuAdapter::with_config(
        fs_config.app_id.clone(),
        secret,
        fs_config.webhook_port,
        region,
        Some(fs_config.webhook_path.clone()),
        fs_config.verification_token.clone(),
        encrypt_key,
        fs_config.bot_names.clone(),
        fs_config.allowed_users.clone(),
    ));
    adapters.push((adapter, fs_config.default_agent.clone()));
}

fn push_revolt_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(rv_config) = config.revolt.as_ref() else {
        return;
    };
    let Some(token) = read_token(&rv_config.bot_token_env, "Revolt") else {
        return;
    };
    let adapter = Arc::new(RevoltAdapter::new(token, rv_config.allowed_users.clone()));
    adapters.push((adapter, rv_config.default_agent.clone()));
}

fn push_wecom_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(wc_config) = config.wecom.as_ref() else {
        return;
    };
    let Some(secret) = read_token(&wc_config.secret_env, "WeCom") else {
        return;
    };
    let adapter = Arc::new(WeComAdapter::with_verification(
        wc_config.corp_id.clone(),
        wc_config.agent_id.clone(),
        secret,
        wc_config.webhook_port,
        wc_config.allowed_users.clone(),
        wc_config.encoding_aes_key.clone(),
        wc_config.token.clone(),
    ));
    adapters.push((adapter, wc_config.default_agent.clone()));
}

fn push_nextcloud_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(nc_config) = config.nextcloud.as_ref() else {
        return;
    };
    let Some(token) = read_token(&nc_config.token_env, "Nextcloud") else {
        return;
    };
    let adapter = Arc::new(NextcloudAdapter::new(
        nc_config.server_url.clone(),
        token,
        nc_config.allowed_rooms.clone(),
        nc_config.allowed_users.clone(),
    ));
    adapters.push((adapter, nc_config.default_agent.clone()));
}

fn push_guilded_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(gd_config) = config.guilded.as_ref() else {
        return;
    };
    let Some(token) = read_token(&gd_config.bot_token_env, "Guilded") else {
        return;
    };
    let adapter = Arc::new(GuildedAdapter::new(
        token,
        gd_config.server_ids.clone(),
        gd_config.allowed_users.clone(),
    ));
    adapters.push((adapter, gd_config.default_agent.clone()));
}

fn push_keybase_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(kb_config) = config.keybase.as_ref() else {
        return;
    };
    let Some(paperkey) = read_token(&kb_config.paperkey_env, "Keybase") else {
        return;
    };
    let adapter = Arc::new(KeybaseAdapter::new(
        kb_config.username.clone(),
        paperkey,
        kb_config.allowed_teams.clone(),
        kb_config.allowed_users.clone(),
    ));
    adapters.push((adapter, kb_config.default_agent.clone()));
}

fn push_threema_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(tm_config) = config.threema.as_ref() else {
        return;
    };
    let Some(secret) = read_token(&tm_config.secret_env, "Threema") else {
        return;
    };
    let adapter = Arc::new(ThreemaAdapter::new(
        tm_config.threema_id.clone(),
        secret,
        tm_config.webhook_port,
        tm_config.allowed_users.clone(),
    ));
    adapters.push((adapter, tm_config.default_agent.clone()));
}

fn push_nostr_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(ns_config) = config.nostr.as_ref() else {
        return;
    };
    let Some(key) = read_token(&ns_config.private_key_env, "Nostr") else {
        return;
    };
    let adapter = Arc::new(NostrAdapter::new(
        key,
        ns_config.relays.clone(),
        ns_config.allowed_users.clone(),
    ));
    adapters.push((adapter, ns_config.default_agent.clone()));
}

fn push_webex_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(wx_config) = config.webex.as_ref() else {
        return;
    };
    let Some(token) = read_token(&wx_config.bot_token_env, "Webex") else {
        return;
    };
    let adapter = Arc::new(WebexAdapter::new(
        token,
        wx_config.allowed_rooms.clone(),
        wx_config.allowed_users.clone(),
    ));
    adapters.push((adapter, wx_config.default_agent.clone()));
}

fn push_pumble_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(pb_config) = config.pumble.as_ref() else {
        return;
    };
    let Some(token) = read_token(&pb_config.bot_token_env, "Pumble") else {
        return;
    };
    let adapter = Arc::new(PumbleAdapter::new(
        token,
        pb_config.webhook_port,
        pb_config.allowed_users.clone(),
    ));
    adapters.push((adapter, pb_config.default_agent.clone()));
}

fn push_flock_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(fl_config) = config.flock.as_ref() else {
        return;
    };
    let Some(token) = read_token(&fl_config.bot_token_env, "Flock") else {
        return;
    };
    let adapter = Arc::new(FlockAdapter::new(
        token,
        fl_config.webhook_port,
        fl_config.allowed_users.clone(),
    ));
    adapters.push((adapter, fl_config.default_agent.clone()));
}

fn push_twist_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(tw_config) = config.twist.as_ref() else {
        return;
    };
    let Some(token) = read_token(&tw_config.token_env, "Twist") else {
        return;
    };
    let adapter = Arc::new(TwistAdapter::new(
        token,
        tw_config.workspace_id.clone(),
        tw_config.allowed_channels.clone(),
        tw_config.allowed_users.clone(),
    ));
    adapters.push((adapter, tw_config.default_agent.clone()));
}

fn push_mumble_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(mb_config) = config.mumble.as_ref() else {
        return;
    };
    let Some(password) = read_token(&mb_config.password_env, "Mumble") else {
        return;
    };
    let adapter = Arc::new(MumbleAdapter::new(
        mb_config.host.clone(),
        mb_config.port,
        password,
        mb_config.username.clone(),
        mb_config.channel.clone(),
        mb_config.allowed_users.clone(),
    ));
    adapters.push((adapter, mb_config.default_agent.clone()));
}

fn push_dingtalk_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(dt_config) = config.dingtalk.as_ref() else {
        return;
    };
    let Some(token) = read_token(&dt_config.access_token_env, "DingTalk") else {
        return;
    };
    let secret = read_token(&dt_config.secret_env, "DingTalk (secret)").unwrap_or_default();
    let adapter = Arc::new(DingTalkAdapter::new(
        token,
        secret,
        dt_config.webhook_port,
        dt_config.allowed_users.clone(),
    ));
    adapters.push((adapter, dt_config.default_agent.clone()));
}

fn push_dingtalk_stream_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(ds_config) = config.dingtalk_stream.as_ref() else {
        return;
    };
    let Some(app_key) = read_token(&ds_config.app_key_env, "DingTalk Stream (app_key)") else {
        return;
    };
    let Some(app_secret) = read_token(&ds_config.app_secret_env, "DingTalk Stream (app_secret)")
    else {
        return;
    };
    let robot_code = read_token(&ds_config.robot_code_env, "DingTalk Stream (robot_code)")
        .unwrap_or_else(|| app_key.clone());
    let adapter = Arc::new(DingTalkStreamAdapter::new(
        app_key,
        app_secret,
        robot_code,
        ds_config.allowed_users.clone(),
    ));
    adapters.push((adapter, ds_config.default_agent.clone()));
}

fn push_discourse_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(dc_config) = config.discourse.as_ref() else {
        return;
    };
    let Some(api_key) = read_token(&dc_config.api_key_env, "Discourse") else {
        return;
    };
    let adapter = Arc::new(DiscourseAdapter::new(
        dc_config.base_url.clone(),
        api_key,
        dc_config.api_username.clone(),
        dc_config.categories.clone(),
        dc_config.allowed_users.clone(),
    ));
    adapters.push((adapter, dc_config.default_agent.clone()));
}

fn push_gitter_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(gt_config) = config.gitter.as_ref() else {
        return;
    };
    let Some(token) = read_token(&gt_config.token_env, "Gitter") else {
        return;
    };
    let adapter = Arc::new(GitterAdapter::new(
        token,
        gt_config.room_id.clone(),
        gt_config.allowed_users.clone(),
    ));
    adapters.push((adapter, gt_config.default_agent.clone()));
}

fn push_ntfy_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(nf_config) = config.ntfy.as_ref() else {
        return;
    };
    let token = if nf_config.token_env.is_empty() {
        String::new()
    } else {
        read_token(&nf_config.token_env, "ntfy").unwrap_or_default()
    };
    let adapter = Arc::new(NtfyAdapter::new(
        nf_config.server_url.clone(),
        nf_config.topic.clone(),
        token,
    ));
    adapters.push((adapter, nf_config.default_agent.clone()));
}

fn push_gotify_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(gf_config) = config.gotify.as_ref() else {
        return;
    };
    let Some(app_token) = read_token(&gf_config.app_token_env, "Gotify (app)") else {
        return;
    };
    let client_token =
        read_token(&gf_config.client_token_env, "Gotify (client)").unwrap_or_default();
    let adapter = Arc::new(GotifyAdapter::new(
        gf_config.server_url.clone(),
        app_token,
        client_token,
    ));
    adapters.push((adapter, gf_config.default_agent.clone()));
}

fn push_webhook_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(wh_config) = config.webhook.as_ref() else {
        return;
    };
    let Some(secret) = read_token(&wh_config.secret_env, "Webhook") else {
        return;
    };
    let adapter = Arc::new(WebhookAdapter::new(
        secret,
        wh_config.listen_port,
        wh_config.callback_url.clone(),
        wh_config.allowed_users.clone(),
    ));
    adapters.push((adapter, wh_config.default_agent.clone()));
}

fn push_linkedin_adapter(
    config: &captain_types::config::ChannelsConfig,
    adapters: &mut Vec<ChannelAdapterStartup>,
) {
    let Some(li_config) = config.linkedin.as_ref() else {
        return;
    };
    let Some(token) = read_token(&li_config.access_token_env, "LinkedIn") else {
        return;
    };
    let adapter = Arc::new(LinkedInAdapter::new(
        token,
        li_config.organization_id.clone(),
        li_config.allowed_users.clone(),
    ));
    adapters.push((adapter, li_config.default_agent.clone()));
}

async fn build_channel_router(
    handle: &KernelBridgeAdapter,
    kernel: &Arc<CaptainKernel>,
    adapters: &[ChannelAdapterStartup],
) -> AgentRouter {
    let mut router = AgentRouter::new();
    let mut system_default_set = false;
    for (adapter, default_agent) in adapters {
        if let Some(ref name) = default_agent {
            let agent_id = match handle.find_agent_by_name(name).await {
                Ok(Some(id)) => Some(id),
                _ => match handle.spawn_agent_by_name(name).await {
                    Ok(id) => Some(id),
                    Err(e) => {
                        warn!(
                            "{}: could not find or spawn default agent '{}': {e}",
                            adapter.name(),
                            name
                        );
                        None
                    }
                },
            };
            if let Some(agent_id) = agent_id {
                let channel_key = format!("{:?}", adapter.channel_type());
                info!(
                    "{} default agent: {name} ({agent_id}) [channel: {channel_key}]",
                    adapter.name()
                );
                router.set_channel_default_with_name(channel_key, agent_id, name.clone());
                if !system_default_set {
                    router.set_default(agent_id);
                    system_default_set = true;
                }
            }
        }
    }

    let bindings = kernel.list_bindings();
    if !bindings.is_empty() {
        for entry in kernel.registry.list() {
            router.register_agent(entry.name.clone(), entry.id);
        }
        router.load_bindings(&bindings);
        info!(count = bindings.len(), "Loaded agent bindings into router");
    }
    router.load_broadcast(kernel.broadcast.clone());
    router
}

async fn start_channel_adapters(
    kernel: &Arc<CaptainKernel>,
    manager: &mut BridgeManager,
    adapters: Vec<ChannelAdapterStartup>,
    bridge_started_at: Instant,
) -> Vec<String> {
    let mut started_names = Vec::new();
    for (adapter, _) in adapters {
        let name = adapter.name().to_string();
        kernel
            .channel_adapters
            .insert(name.clone(), adapter.clone());
        let started = match manager.start_adapter(adapter.clone()).await {
            Ok(()) => {
                info!("{name} channel bridge started");
                true
            }
            Err(e) => {
                kernel.channel_adapters.remove(&name);
                error!("Failed to start {name} bridge: {e}");
                false
            }
        };
        if started {
            crate::daemon_commands::notify_pending_ready(
                kernel.clone(),
                adapter.clone(),
                bridge_started_at,
            )
            .await;
            started_names.push(name);
        }
    }
    started_names
}

/// Start channels from an explicit `ChannelsConfig` (used by hot-reload).
///
/// Returns `(Option<BridgeManager>, Vec<started_channel_names>)`.
pub async fn start_channel_bridge_with_config(
    kernel: Arc<CaptainKernel>,
    config: &captain_types::config::ChannelsConfig,
) -> (Option<BridgeManager>, Vec<String>) {
    let frozen_configured = crate::channel_runtime_policy::frozen_channel_config_names(config);
    let start_frozen_channels = crate::channel_runtime_policy::frozen_channel_runtime_enabled();
    if !frozen_configured.is_empty() {
        let channels = frozen_configured.join(",");
        let active_channels = crate::channel_runtime_policy::ACTIVE_RUNTIME_CHANNELS.join(",");
        warn!(
            channels = %channels,
            active_channels = %active_channels,
            "non-core channel configs are frozen and will not be started"
        );
    }

    if !channel_config_has_any(config) {
        return (None, Vec::new());
    }

    let handle = KernelBridgeAdapter::new(kernel.clone());

    let mut adapters: Vec<ChannelAdapterStartup> = Vec::new();
    push_active_channel_adapters(&kernel, config, &mut adapters);

    if start_frozen_channels {
        push_frozen_channel_adapters(config, &mut adapters);
    }

    if adapters.is_empty() {
        return (None, Vec::new());
    }

    let router = build_channel_router(&handle, &kernel, &adapters).await;
    let bridge_adapter = Arc::new(KernelBridgeAdapter::new(kernel.clone()));
    let bridge_started_at = bridge_adapter.started_at;
    let bridge_handle: Arc<dyn ChannelBridgeHandle> = bridge_adapter;
    let router = Arc::new(router);
    let inbound_queue_path = kernel.config.home_dir.join("channel_inbound_queue.json");
    let mut manager =
        BridgeManager::with_inbound_queue_path(bridge_handle, router, inbound_queue_path);

    let started_names =
        start_channel_adapters(&kernel, &mut manager, adapters, bridge_started_at).await;

    if started_names.is_empty() {
        (None, Vec::new())
    } else {
        (Some(manager), started_names)
    }
}

/// Reload channels from disk config — stops old bridge, starts new one.
///
/// Reads `config.toml` fresh, rebuilds the channel bridge, and stores it
/// in `AppState.bridge_manager`. Returns the list of started channel names.
pub async fn reload_channels_from_disk(
    state: &crate::routes::AppState,
) -> Result<Vec<String>, String> {
    // Stop existing bridge
    {
        let mut guard = state.bridge_manager.lock().await;
        if let Some(ref mut bridge) = *guard {
            bridge.stop().await;
        }
        *guard = None;
    }

    reload_secrets_into_env(&state.kernel.config.home_dir);

    // Re-read config from disk
    let config_path = state.kernel.config.home_dir.join("config.toml");
    let fresh_config = captain_kernel::config::load_config(Some(&config_path));

    // Update the live channels config so list_channels() reflects reality
    *state.channels_config.write().await = fresh_config.channels.clone();

    // Start new bridge with fresh channel config
    let (new_bridge, started) =
        start_channel_bridge_with_config(state.kernel.clone(), &fresh_config.channels).await;

    // Store the new bridge
    *state.bridge_manager.lock().await = new_bridge;

    info!(
        started = started.len(),
        channels = ?started,
        "Channel hot-reload complete"
    );

    Ok(started)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use captain_channels::bridge::ChannelBridgeHandle;
    use captain_channels::telegram::{TelegramAdapter, TelegramStreamTarget};
    use captain_channels::types::ChannelType;
    use captain_runtime::llm_driver::StreamEvent;
    use captain_types::agent::AgentId;
    use captain_types::message::{ContentBlock, Message, MessageContent, Role};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn test_kernel() -> (tempfile::TempDir, Arc<super::CaptainKernel>) {
        let tmp = tempfile::tempdir().expect("temp dir");
        let config = captain_types::config::KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            ..Default::default()
        };
        let kernel = Arc::new(super::CaptainKernel::boot_with_config(config).expect("kernel boot"));
        (tmp, kernel)
    }

    fn test_telegram_adapter(base_url: Option<String>) -> Arc<TelegramAdapter> {
        Arc::new(TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            base_url,
        ))
    }

    fn pending_telegram_ask(
        telegram: Arc<TelegramAdapter>,
        agent_id: AgentId,
        session_key: &str,
        question: &str,
        options: Vec<&str>,
        message_id: Option<i64>,
    ) -> super::PendingTelegramAsk {
        super::PendingTelegramAsk {
            agent_id,
            session_key: session_key.to_string(),
            question: question.to_string(),
            options: options.into_iter().map(str::to_string).collect(),
            telegram,
            chat_id: 42,
            message_id,
        }
    }

    #[test]
    fn telegram_progress_is_ephemeral_operational_and_actionable() {
        let short = super::telegram_stream_progress_text(Duration::from_secs(20));
        assert!(short.contains("<tg-thinking>Captain travaille…</tg-thinking>"));
        assert!(short.contains("environ 1 min"));
        assert!(short.contains("Stop"));

        let long = super::telegram_stream_progress_text(Duration::from_secs(12 * 60));
        assert!(long.contains("environ 12 min"));
        assert!(
            super::TELEGRAM_STREAM_PROGRESS_INTERVAL_SECS < 30,
            "drafts expire after 30 seconds and must be refreshed sooner"
        );
    }

    #[test]
    fn telegram_streaming_enabled_requires_telegram_config() {
        let config = captain_types::config::ChannelsConfig::default();

        assert!(!super::telegram_streaming_enabled(&config));
    }

    #[test]
    fn telegram_streaming_enabled_reads_channel_flag() {
        let mut telegram = captain_types::config::TelegramConfig::default();
        let mut config = captain_types::config::ChannelsConfig {
            telegram: Some(telegram.clone()),
            ..Default::default()
        };
        assert!(super::telegram_streaming_enabled(&config));

        telegram.streaming = false;
        config.telegram = Some(telegram);
        assert!(!super::telegram_streaming_enabled(&config));
    }

    #[test]
    fn telegram_thread_metadata_is_empty_without_topic() {
        assert!(super::telegram_thread_metadata(None).is_empty());
    }

    #[test]
    fn invalid_telegram_ask_choice_does_not_consume_the_question() {
        let agent_id = AgentId::new();
        let telegram = test_telegram_adapter(Some("http://127.0.0.1:1".to_string()));
        let pending = Mutex::new(HashMap::from([(
            "ask-1".to_string(),
            pending_telegram_ask(
                telegram,
                agent_id,
                "telegram|session:1",
                "Déployer ?",
                vec!["Oui", "Non"],
                None,
            ),
        )]));

        let error = match super::take_pending_telegram_ask(&pending, "ask-1", 8) {
            Ok(_) => panic!("invalid index must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("n'est pas valide"));
        assert!(pending.lock().unwrap().contains_key("ask-1"));

        let (_, chosen) = super::take_pending_telegram_ask(&pending, "ask-1", 1)
            .expect("valid choice remains available");
        assert_eq!(chosen, "Non");
        assert!(pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_ask_events_register_every_card_and_only_button_choices() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendRichMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 71}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let telegram = test_telegram_adapter(Some(server.uri()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (raw_tx, raw_rx) = tokio::sync::mpsc::channel(4);
        let (mut forwarded, ids) = super::tee_ask_user_events_to_telegram(
            raw_rx,
            telegram,
            Arc::clone(&pending),
            None,
            AgentId::new(),
            Some("telegram|session:1".to_string()),
            42,
            Some(7),
        );

        raw_tx
            .send(StreamEvent::AskUser {
                question: "Déployer ?".to_string(),
                options: Some(vec!["Oui".to_string(), "Non".to_string()]),
            })
            .await
            .unwrap();
        raw_tx
            .send(StreamEvent::AskUser {
                question: "Quel commentaire ?".to_string(),
                options: None,
            })
            .await
            .unwrap();
        drop(raw_tx);
        assert!(forwarded.recv().await.is_none());

        assert_eq!(ids.lock().unwrap().len(), 2);
        let pending = pending.lock().unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending.values().any(|entry| entry.options.is_empty()));
        assert!(pending.values().any(|entry| entry.options.len() == 2));
        drop(pending);

        let requests = server.received_requests().await.expect("requests");
        let bodies = requests
            .iter()
            .map(|request| {
                serde_json::from_slice::<serde_json::Value>(&request.body).expect("Rich request")
            })
            .collect::<Vec<_>>();
        assert_eq!(bodies[0]["message_thread_id"], 7);
        assert!(bodies[0]["reply_markup"]["inline_keyboard"].is_array());
        assert!(bodies[1].get("reply_markup").is_none());
        assert!(bodies[0]["rich_message"]["markdown"]
            .as_str()
            .unwrap()
            .contains("Décision requise"));
        assert!(bodies[1]["rich_message"]["markdown"]
            .as_str()
            .unwrap()
            .contains("### ❓ Question"));
    }

    #[tokio::test]
    async fn telegram_ask_answer_reaches_agent_before_card_is_confirmed() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/editMessageText"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 71}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let telegram = test_telegram_adapter(Some(server.uri()));
        let (_tmp, kernel) = test_kernel();
        let bridge = super::KernelBridgeAdapter::new(kernel);
        let agent_id = AgentId::new();
        let session_key = "telegram|session:1";
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        bridge.active_streams.lock().await.insert(
            session_key.to_string(),
            super::ActiveStream {
                agent_id,
                sender: tx,
                reply_handle: Arc::new(Mutex::new(None)),
                progress_draft: None,
            },
        );
        bridge.pending_ask_users.lock().unwrap().insert(
            "ask-1".to_string(),
            pending_telegram_ask(
                telegram,
                agent_id,
                session_key,
                "Déployer ?",
                vec!["Oui", "Non"],
                Some(71),
            ),
        );

        let chosen = bridge
            .try_answer_ask_user("ask-1", 0)
            .await
            .expect("answer delivered");
        assert_eq!(chosen, "Oui");
        assert_eq!(rx.recv().await.as_deref(), Some("Oui"));
        assert!(bridge.pending_ask_users.lock().unwrap().is_empty());

        let requests = server.received_requests().await.expect("requests");
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("edit request");
        assert!(body["rich_message"]["markdown"]
            .as_str()
            .unwrap()
            .contains("Décision enregistrée"));
        assert_eq!(
            body["reply_markup"],
            serde_json::json!({"inline_keyboard": []})
        );
    }

    #[tokio::test]
    async fn freeform_answer_resolves_waiting_telegram_card_and_progress_state() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/editMessageText"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 72}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let telegram = test_telegram_adapter(Some(server.uri()));
        let progress = TelegramStreamTarget::new(Arc::clone(&telegram), 42, None)
            .progress_draft()
            .expect("private chat progress");
        progress.set_waiting_for_user(true);
        let (_tmp, kernel) = test_kernel();
        let bridge = super::KernelBridgeAdapter::new(kernel);
        let agent_id = AgentId::new();
        let session_key = "telegram|session:freeform";
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        bridge.active_streams.lock().await.insert(
            session_key.to_string(),
            super::ActiveStream {
                agent_id,
                sender: tx,
                reply_handle: Arc::new(Mutex::new(None)),
                progress_draft: Some(progress.clone()),
            },
        );
        bridge.pending_ask_users.lock().unwrap().insert(
            "ask-free".to_string(),
            pending_telegram_ask(
                telegram,
                agent_id,
                session_key,
                "Quel commentaire ?",
                vec![],
                Some(72),
            ),
        );

        assert!(bridge
            .try_forward_active_stream_interjection(
                agent_id,
                Some(session_key),
                "Déployer après le smoke test.",
                Some(101),
            )
            .await
            .expect("interjection"));
        assert_eq!(
            rx.recv().await.as_deref(),
            Some("Déployer après le smoke test.")
        );
        assert!(!progress.is_waiting_for_user());
        assert!(bridge.pending_ask_users.lock().unwrap().is_empty());

        let requests = server.received_requests().await.expect("requests");
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("edit request");
        let markdown = body["rich_message"]["markdown"].as_str().unwrap();
        assert!(markdown.contains("Décision enregistrée"));
        assert!(markdown.contains("Déployer après le smoke test."));
    }

    #[tokio::test]
    async fn stream_end_expires_every_unanswered_telegram_card() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/editMessageText"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 73}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let telegram = test_telegram_adapter(Some(server.uri()));
        let agent_id = AgentId::new();
        let ids = Arc::new(Mutex::new(vec!["ask-1".to_string(), "ask-2".to_string()]));
        let pending = Arc::new(Mutex::new(HashMap::from([
            (
                "ask-1".to_string(),
                pending_telegram_ask(
                    Arc::clone(&telegram),
                    agent_id,
                    "telegram|session:1",
                    "Première question ?",
                    vec!["Oui"],
                    Some(73),
                ),
            ),
            (
                "ask-2".to_string(),
                pending_telegram_ask(
                    telegram,
                    agent_id,
                    "telegram|session:1",
                    "Deuxième question ?",
                    vec![],
                    Some(74),
                ),
            ),
        ])));

        super::expire_unanswered_telegram_asks(&ids, &pending).await;

        assert!(ids.lock().unwrap().is_empty());
        assert!(pending.lock().unwrap().is_empty());
        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2);
        for request in requests {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("edit request");
            assert!(body["rich_message"]["markdown"]
                .as_str()
                .unwrap()
                .contains("Question expirée"));
            assert_eq!(
                body["reply_markup"],
                serde_json::json!({"inline_keyboard": []})
            );
        }
    }

    #[test]
    fn telegram_thread_metadata_keeps_topic_id() {
        let metadata = super::telegram_thread_metadata(Some(42));

        assert_eq!(metadata.get("thread_id"), Some(&serde_json::json!(42)));
    }

    #[test]
    fn cron_action_message_describes_workflow_input() {
        let action = captain_types::scheduler::CronAction::WorkflowRun {
            workflow_id: "daily-review".to_string(),
            input: Some("summarize yesterday".to_string()),
            timeout_secs: None,
        };

        assert_eq!(
            super::cron_action_message(&action),
            "Run workflow daily-review with input: summarize yesterday"
        );
    }

    #[test]
    fn cron_action_message_describes_inline_workflow_size() {
        let action = captain_types::scheduler::CronAction::InlineWorkflow {
            steps: vec![
                captain_types::scheduler::WorkflowStep {
                    tool: "web_search".to_string(),
                    args: serde_json::json!({"q": "captain"}),
                    pipe_output: true,
                },
                captain_types::scheduler::WorkflowStep {
                    tool: "memory_save".to_string(),
                    args: serde_json::json!({}),
                    pipe_output: false,
                },
            ],
        };

        assert_eq!(
            super::cron_action_message(&action),
            "Inline workflow (2 steps)"
        );
    }

    #[test]
    fn active_interjection_send_accepts_when_buffer_has_room() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        assert_eq!(
            super::try_send_active_interjection(&tx, "ajoute le contexte"),
            super::ActiveInterjectionSend::Accepted
        );
        assert_eq!(rx.try_recv().expect("message queued"), "ajoute le contexte");
    }

    #[test]
    fn active_interjection_send_falls_back_when_buffer_is_full() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.try_send("first".to_string()).expect("fills buffer");

        assert_eq!(
            super::try_send_active_interjection(&tx, "second"),
            super::ActiveInterjectionSend::QueueNextTurn
        );
        assert_eq!(rx.try_recv().expect("original message kept"), "first");
        assert!(
            rx.try_recv().is_err(),
            "full fallback must not drop into rx"
        );
    }

    #[test]
    fn active_interjection_send_reports_closed_channel() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);

        assert_eq!(
            super::try_send_active_interjection(&tx, "late"),
            super::ActiveInterjectionSend::Closed
        );
    }

    #[test]
    fn active_stream_lookup_is_scoped_to_session_key_and_agent() {
        let agent_id = AgentId::new();
        let other_agent_id = AgentId::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let mut streams = HashMap::new();
        streams.insert(
            "telegram|chat:1|user:10|captain:-|thread:-".to_string(),
            super::ActiveStream {
                agent_id,
                sender: tx,
                reply_handle: Arc::new(Mutex::new(None)),
                progress_draft: None,
            },
        );

        assert!(super::clone_active_stream_for_session(
            &streams,
            Some("telegram|chat:1|user:10|captain:-|thread:-"),
            agent_id
        )
        .is_some());
        assert!(super::clone_active_stream_for_session(
            &streams,
            Some("telegram|chat:2|user:10|captain:-|thread:-"),
            agent_id
        )
        .is_none());
        assert!(super::clone_active_stream_for_session(
            &streams,
            Some("telegram|chat:1|user:10|captain:-|thread:-"),
            other_agent_id
        )
        .is_none());
        assert!(super::clone_active_stream_for_session(&streams, None, agent_id).is_none());
    }

    #[test]
    fn skill_proposal_channel_decider_requires_external_validation_for_approval() {
        assert_eq!(
            super::skill_proposal_channel_decided_by(true, false).unwrap_err(),
            super::SKILL_PROPOSAL_APPROVAL_USAGE
        );
        assert_eq!(
            super::skill_proposal_channel_decided_by(true, true).unwrap(),
            "channel:schema_diff_tests_human"
        );
        assert_eq!(
            super::skill_proposal_channel_decided_by(false, false).unwrap(),
            "channel"
        );
    }

    #[test]
    fn recent_session_diagnostics_reports_no_visible_tool_errors() {
        let session = captain_memory::session::Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: captain_types::agent::AgentId::new(),
            messages: vec![
                Message::user("Pourquoi tu as eu des erreurs ?"),
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                        id: "call_1".into(),
                        name: "channel_send".into(),
                        input: serde_json::json!({ "channel": "telegram" }),
                        provider_metadata: None,
                    }]),
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".into(),
                        tool_name: "channel_send".into(),
                        content: "sent".into(),
                        is_error: false,
                    }]),
                },
            ],
            context_window_tokens: 0,
            label: None,
        };

        let diag = super::recent_session_diagnostics(&session);

        assert!(diag.contains("Échecs outil récents visibles: aucun"));
        assert!(diag.contains("channel_send"));
    }

    #[test]
    fn recent_session_diagnostics_classifies_failures_without_raw_json() {
        let session = captain_memory::session::Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: captain_types::agent::AgentId::new(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".into(),
                    tool_name: "web_fetch".into(),
                    content: r#"{"error":"request timed out after 30s"}"#.into(),
                    is_error: true,
                }]),
            }],
            context_window_tokens: 0,
            label: None,
        };

        let diag = super::recent_session_diagnostics(&session);

        assert!(diag.contains("web_fetch: timeout"));
        assert!(!diag.contains("request timed out after 30s"));
    }

    #[tokio::test]
    async fn test_bridge_skips_when_no_config() {
        let config = captain_types::config::KernelConfig::default();
        assert!(config.channels.telegram.is_none());
        assert!(config.channels.discord.is_none());
        assert!(config.channels.slack.is_none());
        assert!(config.channels.whatsapp.is_none());
        assert!(config.channels.signal.is_none());
        assert!(config.channels.matrix.is_none());
        assert!(config.channels.email.is_none());
        assert!(config.channels.teams.is_none());
        assert!(config.channels.mattermost.is_none());
        assert!(config.channels.irc.is_none());
        assert!(config.channels.google_chat.is_none());
        assert!(config.channels.twitch.is_none());
        assert!(config.channels.rocketchat.is_none());
        assert!(config.channels.zulip.is_none());
        assert!(config.channels.xmpp.is_none());
        // Wave 3
        assert!(config.channels.line.is_none());
        assert!(config.channels.viber.is_none());
        assert!(config.channels.messenger.is_none());
        assert!(config.channels.reddit.is_none());
        assert!(config.channels.mastodon.is_none());
        assert!(config.channels.bluesky.is_none());
        assert!(config.channels.feishu.is_none());
        assert!(config.channels.revolt.is_none());
        // Wave 4
        assert!(config.channels.nextcloud.is_none());
        assert!(config.channels.guilded.is_none());
        assert!(config.channels.keybase.is_none());
        assert!(config.channels.threema.is_none());
        assert!(config.channels.nostr.is_none());
        assert!(config.channels.webex.is_none());
        assert!(config.channels.pumble.is_none());
        assert!(config.channels.flock.is_none());
        assert!(config.channels.twist.is_none());
        // Wave 5
        assert!(config.channels.mumble.is_none());
        assert!(config.channels.dingtalk.is_none());
        assert!(config.channels.dingtalk_stream.is_none());
        assert!(config.channels.discourse.is_none());
        assert!(config.channels.gitter.is_none());
        assert!(config.channels.ntfy.is_none());
        assert!(config.channels.gotify.is_none());
        assert!(config.channels.webhook.is_none());
        assert!(config.channels.linkedin.is_none());
        assert!(config.channels.wecom.is_none());
        assert!(!super::channel_config_has_any(&config.channels));
    }

    #[test]
    fn test_channel_config_has_any_ignores_frozen_only() {
        let mut config = captain_types::config::ChannelsConfig::default();
        assert!(!super::channel_config_has_any(&config));
        config.wecom = Some(captain_types::config::WeComConfig::default());
        assert!(!super::channel_config_has_any(&config));
        config.email = Some(captain_types::config::EmailConfig::default());
        assert!(super::channel_config_has_any(&config));
        config.email = None;
        config.discord = Some(captain_types::config::DiscordConfig::default());
        assert!(super::channel_config_has_any(&config));
    }

    #[test]
    fn test_channel_config_has_any_ignores_silent_mode() {
        let config = captain_types::config::ChannelsConfig {
            silent_mode: true,
            ..Default::default()
        };
        assert!(!super::channel_config_has_any(&config));
    }

    #[test]
    fn active_channel_adapters_collect_configured_core_channels() {
        let _telegram_token =
            EnvVarGuard::set("CAPTAIN_TEST_ACTIVE_TELEGRAM_TOKEN", "telegram-token");
        let _discord_token = EnvVarGuard::set("CAPTAIN_TEST_ACTIVE_DISCORD_TOKEN", "discord-token");
        let _email_password = EnvVarGuard::set("CAPTAIN_TEST_ACTIVE_EMAIL_PASSWORD", "email-pass");
        let (_tmp, kernel) = test_kernel();
        let config = captain_types::config::ChannelsConfig {
            telegram: Some(captain_types::config::TelegramConfig {
                bot_token_env: "CAPTAIN_TEST_ACTIVE_TELEGRAM_TOKEN".to_string(),
                default_agent: Some("telegram-agent".to_string()),
                ..Default::default()
            }),
            discord: Some(captain_types::config::DiscordConfig {
                bot_token_env: "CAPTAIN_TEST_ACTIVE_DISCORD_TOKEN".to_string(),
                default_agent: Some("discord-agent".to_string()),
                ..Default::default()
            }),
            signal: Some(captain_types::config::SignalConfig {
                phone_number: "+12345678900".to_string(),
                default_agent: Some("signal-agent".to_string()),
                ..Default::default()
            }),
            email: Some(captain_types::config::EmailConfig {
                password_env: "CAPTAIN_TEST_ACTIVE_EMAIL_PASSWORD".to_string(),
                username: "captain@example.test".to_string(),
                imap_host: "imap.example.test".to_string(),
                smtp_host: "smtp.example.test".to_string(),
                default_agent: Some("email-agent".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut adapters = Vec::new();

        super::push_active_channel_adapters(&kernel, &config, &mut adapters);

        let collected = adapters
            .iter()
            .map(|(adapter, default_agent)| (adapter.channel_type(), default_agent.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            collected,
            vec![
                (ChannelType::Telegram, Some("telegram-agent".to_string())),
                (ChannelType::Discord, Some("discord-agent".to_string())),
                (ChannelType::Signal, Some("signal-agent".to_string())),
                (ChannelType::Email, Some("email-agent".to_string())),
            ]
        );
    }

    #[test]
    fn active_channel_adapters_skip_missing_tokens_and_empty_signal_phone() {
        let _telegram_token = EnvVarGuard::unset("CAPTAIN_TEST_MISSING_TELEGRAM_TOKEN");
        let _discord_token = EnvVarGuard::unset("CAPTAIN_TEST_MISSING_DISCORD_TOKEN");
        let _email_password = EnvVarGuard::unset("CAPTAIN_TEST_MISSING_EMAIL_PASSWORD");
        let (_tmp, kernel) = test_kernel();
        let config = captain_types::config::ChannelsConfig {
            telegram: Some(captain_types::config::TelegramConfig {
                bot_token_env: "CAPTAIN_TEST_MISSING_TELEGRAM_TOKEN".to_string(),
                ..Default::default()
            }),
            discord: Some(captain_types::config::DiscordConfig {
                bot_token_env: "CAPTAIN_TEST_MISSING_DISCORD_TOKEN".to_string(),
                ..Default::default()
            }),
            signal: Some(captain_types::config::SignalConfig {
                phone_number: String::new(),
                ..Default::default()
            }),
            email: Some(captain_types::config::EmailConfig {
                password_env: "CAPTAIN_TEST_MISSING_EMAIL_PASSWORD".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut adapters = Vec::new();

        super::push_active_channel_adapters(&kernel, &config, &mut adapters);

        assert!(adapters.is_empty());
    }

    #[test]
    fn frozen_channel_adapters_collect_configured_subset_without_network() {
        let _slack_app_token = EnvVarGuard::set("CAPTAIN_TEST_FROZEN_SLACK_APP_TOKEN", "xapp-test");
        let _slack_bot_token = EnvVarGuard::set("CAPTAIN_TEST_FROZEN_SLACK_BOT_TOKEN", "xoxb-test");
        let config = captain_types::config::ChannelsConfig {
            slack: Some(captain_types::config::SlackConfig {
                app_token_env: "CAPTAIN_TEST_FROZEN_SLACK_APP_TOKEN".to_string(),
                bot_token_env: "CAPTAIN_TEST_FROZEN_SLACK_BOT_TOKEN".to_string(),
                default_agent: Some("slack-agent".to_string()),
                ..Default::default()
            }),
            ntfy: Some(captain_types::config::NtfyConfig {
                topic: "captain-test".to_string(),
                token_env: String::new(),
                default_agent: Some("ntfy-agent".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut adapters = Vec::new();

        super::push_frozen_channel_adapters(&config, &mut adapters);

        let collected = adapters
            .iter()
            .map(|(adapter, default_agent)| {
                (
                    adapter.name().to_string(),
                    adapter.channel_type(),
                    default_agent.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            collected,
            vec![
                (
                    "slack".to_string(),
                    ChannelType::Slack,
                    Some("slack-agent".to_string())
                ),
                (
                    "ntfy".to_string(),
                    ChannelType::Custom("ntfy".to_string()),
                    Some("ntfy-agent".to_string())
                ),
            ]
        );
    }

    #[test]
    fn frozen_channel_adapters_skip_slack_when_required_token_is_missing() {
        let _slack_app_token =
            EnvVarGuard::set("CAPTAIN_TEST_FROZEN_SKIP_SLACK_APP_TOKEN", "xapp-test");
        let _slack_bot_token = EnvVarGuard::unset("CAPTAIN_TEST_FROZEN_SKIP_SLACK_BOT_TOKEN");
        let config = captain_types::config::ChannelsConfig {
            slack: Some(captain_types::config::SlackConfig {
                app_token_env: "CAPTAIN_TEST_FROZEN_SKIP_SLACK_APP_TOKEN".to_string(),
                bot_token_env: "CAPTAIN_TEST_FROZEN_SKIP_SLACK_BOT_TOKEN".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut adapters = Vec::new();

        super::push_frozen_channel_adapters(&config, &mut adapters);

        assert!(adapters.is_empty());
    }

    #[test]
    fn skill_refinement_index_resolves_unique_prefix() {
        let items = vec![
            serde_json::json!({"id": "aaa111", "skill": "alpha"}),
            serde_json::json!({"id": "bbb222", "skill": "beta"}),
        ];
        assert_eq!(
            super::resolve_skill_refinement_index(&items, "bbb").unwrap(),
            1
        );
    }

    #[test]
    fn skill_refinement_index_rejects_ambiguous_prefix() {
        let items = vec![
            serde_json::json!({"id": "aaa111", "skill": "alpha"}),
            serde_json::json!({"id": "aaa222", "skill": "beta"}),
        ];
        let err = super::resolve_skill_refinement_index(&items, "aaa").unwrap_err();
        assert!(err.contains("2 raffinements"));
    }
}
