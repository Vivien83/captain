//! Telegram callback payload parsing and inline keyboards.

use crate::types::{ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use captain_types::release_update::{
    RuntimeUpdateTelegramAction, RUNTIME_UPDATE_CALLBACK_TOKEN_LEN,
};
use captain_types::workflow_learning::ProposalCardAction;
use std::collections::HashMap;

const CAPSPEC_CALLBACK_TOKEN_LEN: usize = 20;
const WORKFLOW_LEARNING_CALLBACK_TOKEN_LEN: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapSpecTelegramAction {
    Approve,
    Reject,
    Retry,
    ConfirmSucceeded,
    MarkFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapSpecTelegramCallback {
    pub action: CapSpecTelegramAction,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLearningTelegramCallback {
    pub action: ProposalCardAction,
    pub token: String,
    pub decision_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUpdateTelegramCallback {
    pub action: RuntimeUpdateTelegramAction,
    pub token: String,
    pub decision_version: u64,
}

/// Parse a model-independent runtime update decision.
///
/// The compact token only performs lookup. The kernel still compares the
/// exact candidate version, decision version, recipient chat and operator.
pub fn parse_runtime_update_callback(data: &str) -> Option<RuntimeUpdateTelegramCallback> {
    let rest = data.strip_prefix("runtime_update:")?;
    let mut parts = rest.split(':');
    let action = match parts.next()? {
        "install" => RuntimeUpdateTelegramAction::Install,
        "defer" => RuntimeUpdateTelegramAction::Defer,
        "refuse" => RuntimeUpdateTelegramAction::Refuse,
        _ => return None,
    };
    let token = parts.next()?.trim();
    let decision_version = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || decision_version == 0 {
        return None;
    }
    if token.len() != RUNTIME_UPDATE_CALLBACK_TOKEN_LEN
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(RuntimeUpdateTelegramCallback {
        action,
        token: token.to_ascii_lowercase(),
        decision_version,
    })
}

/// Parse a compact Skill Learning V2 operator callback.
///
/// The callback carries an 80-bit lookup token and the exact proposal state
/// version rendered in the card. The control plane must still resolve the
/// token and compare the complete revision before applying any decision.
pub fn parse_workflow_learning_callback(data: &str) -> Option<WorkflowLearningTelegramCallback> {
    let rest = data.strip_prefix("workflow:")?;
    let mut parts = rest.split(':');
    let action = match parts.next()? {
        "activate" => ProposalCardAction::Activate,
        "test" => ProposalCardAction::Test,
        "details" => ProposalCardAction::Details,
        "edit" => ProposalCardAction::Edit,
        "later" => ProposalCardAction::Later,
        "ignore" => ProposalCardAction::Ignore,
        _ => return None,
    };
    let token = parts.next()?.trim();
    let decision_version = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || decision_version == 0 {
        return None;
    }
    if token.len() != WORKFLOW_LEARNING_CALLBACK_TOKEN_LEN
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(WorkflowLearningTelegramCallback {
        action,
        token: token.to_ascii_lowercase(),
        decision_version,
    })
}

/// Parse a compact Captain Forge operator callback.
///
/// The token is a deterministic 80-bit lookup key. The kernel never trusts it
/// as the decision identity: it resolves the token against the current pending
/// records and then applies a compare-and-set using the complete source hash or
/// run/node/tool-use/attempt tuple.
pub fn parse_capspec_callback(data: &str) -> Option<CapSpecTelegramCallback> {
    let rest = data.strip_prefix("capspec:")?;
    let mut parts = rest.splitn(2, ':');
    let action = match parts.next()? {
        "approve" => CapSpecTelegramAction::Approve,
        "reject" => CapSpecTelegramAction::Reject,
        "retry" => CapSpecTelegramAction::Retry,
        "confirm" => CapSpecTelegramAction::ConfirmSucceeded,
        "fail" => CapSpecTelegramAction::MarkFailed,
        _ => return None,
    };
    let token = parts.next()?.trim();
    if token.len() != CAPSPEC_CALLBACK_TOKEN_LEN
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(CapSpecTelegramCallback {
        action,
        token: token.to_ascii_lowercase(),
    })
}

/// Parse an `approval:<action>:<id>` callback payload into a slash command.
///
/// Q.11.b.2 — 4 distinct routes:
/// - `approval:once:<id>`    -> `/approve <id>`
/// - `approval:session:<id>` -> `/approve_session <id>`
/// - `approval:always:<id>`  -> `/approve_always <id>`
/// - `approval:deny:<id>`    -> `/reject <id>`
///
/// Returns `None` for unrelated callback data (agent reads via LLM path).
pub fn parse_approval_callback(data: &str) -> Option<(String, Vec<String>)> {
    let rest = data.strip_prefix("approval:")?;
    let mut parts = rest.splitn(2, ':');
    let action = parts.next()?;
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    let cmd = match action {
        "once" => "approve",
        "session" => "approve_session",
        "always" => "approve_always",
        "deny" => "reject",
        _ => return None,
    };
    Some((cmd.to_string(), vec![id.to_string()]))
}

/// Parse a learning-review callback payload into the dedicated slash command.
///
/// Learning approvals are not tool-execution approvals: they must resolve
/// `learning_review_decide`, not the ApprovalManager queue. Keeping a
/// distinct callback namespace prevents review ids from being sent to
/// `/approve`, where they would never match.
pub fn parse_learning_callback(data: &str) -> Option<(String, Vec<String>)> {
    let rest = data.strip_prefix("learning:")?;
    let mut parts = rest.splitn(2, ':');
    let action = parts.next()?;
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    let cmd = match action {
        "approve" => "learn_approve",
        "reject" => "learn_reject",
        _ => return None,
    };
    Some((cmd.to_string(), vec![id.to_string()]))
}

/// Parse callbacks from already-delivered v3.13 cards. The command dispatcher
/// renders an archive notice; no legacy decision is executed.
pub fn parse_skill_proposal_callback(data: &str) -> Option<(String, Vec<String>)> {
    let rest = data.strip_prefix("skill_proposal:")?;
    let mut parts = rest.splitn(2, ':');
    let action = parts.next()?;
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    let cmd = match action {
        "approve" => "skill_approve",
        "reject" => "skill_reject",
        _ => return None,
    };
    Some((cmd.to_string(), vec![id.to_string()]))
}

/// Existing-skill refinement approvals resolve the controlled refinement
/// registry, not generated-skill proposals and not generic approvals.
pub fn parse_skill_refinement_callback(data: &str) -> Option<(String, Vec<String>)> {
    let rest = data.strip_prefix("skill_refinement:")?;
    let mut parts = rest.splitn(2, ':');
    let action = parts.next()?;
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    let cmd = match action {
        "approve" => "skill_refine_approve",
        "reject" => "skill_refine_reject",
        _ => return None,
    };
    Some((cmd.to_string(), vec![id.to_string()]))
}

/// Parse a model-switch callback payload into the bridge-only apply command.
///
/// Telegram sends `model:<plan_id>:<choice>` after the user clicks one of the
/// safe rail buttons. The bridge owns `plan_id` persistence and resolves it
/// before calling the kernel apply path.
pub fn parse_model_switch_callback(data: &str) -> Option<(String, Vec<String>)> {
    let rest = data.strip_prefix("model:")?;
    let mut parts = rest.splitn(2, ':');
    let plan_id = parts.next()?.trim();
    let choice = parts.next()?.trim();
    if plan_id.is_empty() || choice.is_empty() {
        return None;
    }
    match choice {
        "new_session" | "compact_session" | "cancel" => Some((
            "model_switch".to_string(),
            vec![plan_id.to_string(), choice.to_string()],
        )),
        _ => None,
    }
}

/// Project ask-user callbacks resolve a pending autonomous project question.
///
/// Payload shape: `project_ask:<ask_id>:<zero_based_option_index>`.
pub fn parse_project_ask_callback(data: &str) -> Option<(String, Vec<String>)> {
    let rest = data.strip_prefix("project_ask:")?;
    let mut parts = rest.splitn(2, ':');
    let ask_id = parts.next()?.trim();
    let choice = parts.next()?.trim();
    if ask_id.is_empty() || choice.is_empty() {
        return None;
    }
    if choice.parse::<usize>().is_err() {
        return None;
    }
    Some((
        "project_answer".to_string(),
        vec![ask_id.to_string(), format!("@idx:{choice}")],
    ))
}

/// Build a Telegram `inline_keyboard` markup for an approval request.
///
/// Q.11.b.2 — 4 buttons, laid out on 2 rows for mobile
/// readability. Click triggers a callback_query that
/// `parse_approval_callback` routes into one of the 4 slash commands.
pub fn build_approval_keyboard(request_id: &str) -> serde_json::Value {
    serde_json::json!({
        "inline_keyboard": [
            [
                {"text": "✅ Approuver une fois", "callback_data": format!("approval:once:{request_id}")},
                {"text": "🕒 Pour la session",    "callback_data": format!("approval:session:{request_id}")},
            ],
            [
                {"text": "🔒 Toujours",           "callback_data": format!("approval:always:{request_id}")},
                {"text": "❌ Rejeter",            "callback_data": format!("approval:deny:{request_id}")},
            ]
        ]
    })
}

/// Build a Telegram inline keyboard for safe model-switch session choices.
pub fn build_model_switch_keyboard(plan_id: &str) -> serde_json::Value {
    build_model_switch_keyboard_with_recommendation(plan_id, None)
}

/// Same as `build_model_switch_keyboard`, with the recommended strategy marked
/// when the kernel preflight plan includes one.
pub fn build_model_switch_keyboard_with_recommendation(
    plan_id: &str,
    recommended_session_strategy: Option<&str>,
) -> serde_json::Value {
    let new_text = if recommended_session_strategy == Some("new_session") {
        "Nouvelle session (recommandé)"
    } else {
        "Nouvelle session"
    };
    let compact_text = if recommended_session_strategy == Some("compact_session") {
        "Resume compact (recommandé)"
    } else {
        "Resume compact"
    };

    serde_json::json!({
        "inline_keyboard": [
            [
                {"text": new_text, "callback_data": format!("model:{plan_id}:new_session")},
                {"text": compact_text, "callback_data": format!("model:{plan_id}:compact_session")},
            ],
            [
                {"text": "Annuler", "callback_data": format!("model:{plan_id}:cancel")},
            ]
        ]
    })
}

/// Build a Telegram inline keyboard for memory/learning review items.
pub fn build_learning_approval_keyboard(review_id: &str) -> serde_json::Value {
    serde_json::json!({
        "inline_keyboard": [[
            {"text": "✅ Approuver", "callback_data": format!("learning:approve:{review_id}")},
            {"text": "❌ Rejeter",   "callback_data": format!("learning:reject:{review_id}")},
        ]]
    })
}

/// Build a Telegram inline keyboard for existing-skill refinement proposals.
pub fn build_skill_refinement_keyboard(refinement_id: &str) -> serde_json::Value {
    serde_json::json!({
        "inline_keyboard": [[
            {"text": "✅ Améliorer le skill", "callback_data": format!("skill_refinement:approve:{refinement_id}")},
            {"text": "❌ Ignorer",            "callback_data": format!("skill_refinement:reject:{refinement_id}")},
        ]]
    })
}

/// Human approval controls for one exact pending CapSpec revision.
pub fn build_capspec_approval_keyboard(token: &str) -> serde_json::Value {
    serde_json::json!({
        "inline_keyboard": [[
            {"text": "✅ Approuver", "callback_data": format!("capspec:approve:{token}")},
            {"text": "❌ Refuser",   "callback_data": format!("capspec:reject:{token}")},
        ]]
    })
}

/// Recovery controls for one exact uncertain CapSpec node attempt.
pub fn build_capspec_uncertain_keyboard(token: &str) -> serde_json::Value {
    serde_json::json!({
        "inline_keyboard": [
            [
                {"text": "🔁 Réessayer", "callback_data": format!("capspec:retry:{token}")},
                {"text": "✅ Confirmer (null)", "callback_data": format!("capspec:confirm:{token}")},
            ],
            [
                {"text": "⛔ Marquer en échec", "callback_data": format!("capspec:fail:{token}")},
            ]
        ]
    })
}

/// Parse an `ask_user:<short_id>:<zero_based_option_index>` callback payload.
///
/// Unlike `parse_project_ask_callback`, this doesn't resolve to a slash
/// command — the generic `ask_user` tool call has no project/goal to route
/// to, only a live agent-loop turn waiting on `user_input_rx`. The caller
/// (channel_bridge.rs) resolves `short_id` against its own registry to
/// recover the full option text and the stream to forward it into.
pub fn parse_ask_user_callback(data: &str) -> Option<(String, usize)> {
    let rest = data.strip_prefix("ask_user:")?;
    let mut parts = rest.splitn(2, ':');
    let short_id = parts.next()?.trim();
    let idx = parts.next()?.trim();
    if short_id.is_empty() {
        return None;
    }
    let idx = idx.parse::<usize>().ok()?;
    Some((short_id.to_string(), idx))
}

/// Build Telegram buttons for a generic `ask_user` tool-call prompt.
///
/// Same shape and truncation rule as `build_project_ask_keyboard` — Telegram
/// callback payloads are capped, so buttons send the option index and the
/// caller keeps its own `short_id -> options` registry to recover the text.
pub fn build_ask_user_keyboard(short_id: &str, options: &[String]) -> serde_json::Value {
    let rows: Vec<Vec<serde_json::Value>> = options
        .iter()
        .take(6)
        .enumerate()
        .map(|(idx, option)| {
            let mut label: String = option.chars().take(44).collect();
            if option.chars().count() > 44 {
                label.push('…');
            }
            vec![serde_json::json!({
                "text": label,
                "callback_data": format!("ask_user:{short_id}:{idx}"),
            })]
        })
        .collect();
    serde_json::json!({ "inline_keyboard": rows })
}

/// Build Telegram buttons for a project ask-user prompt.
///
/// Telegram callback payloads are capped, so buttons send the option index. The
/// pending ask registry maps that index back to the full answer text.
pub fn build_project_ask_keyboard(ask_id: &str, options: &[String]) -> serde_json::Value {
    let rows: Vec<Vec<serde_json::Value>> = options
        .iter()
        .take(6)
        .enumerate()
        .map(|(idx, option)| {
            let mut label: String = option.chars().take(44).collect();
            if option.chars().count() > 44 {
                label.push('…');
            }
            vec![serde_json::json!({
                "text": label,
                "callback_data": format!("project_ask:{ask_id}:{idx}"),
            })]
        })
        .collect();
    serde_json::json!({ "inline_keyboard": rows })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn callback_command_message(
    callback_query_id: &str,
    callback_data: &str,
    command_name: String,
    args: Vec<String>,
    chat_id: i64,
    from_id: i64,
    from_name: &str,
    thread_id: Option<String>,
) -> ChannelMessage {
    let mut metadata = HashMap::new();
    metadata.insert("callback_query".to_string(), serde_json::json!(true));
    metadata.insert(
        "callback_data".to_string(),
        serde_json::json!(callback_data),
    );
    metadata.insert("sender_user_id".to_string(), serde_json::json!(from_id));
    metadata.insert("chat_id".to_string(), serde_json::json!(chat_id));

    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: callback_query_id.to_string(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: from_name.to_string(),
            captain_user: None,
        },
        content: ChannelContent::Command {
            name: command_name,
            args,
        },
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id,
        metadata,
    }
}

/// TG2 — build the synthetic inbound message for an `ask_user:<short_id>:<idx>`
/// callback click.
///
/// Unlike `callback_command_message`, this carries `ChannelContent::Text`
/// (not `Command`) so it flows through the same inbound pipeline as a
/// normal typed reply and reaches `send_inbound_agent_message`, which
/// special-cases the `ask_user_short_id` metadata key to resolve the real
/// answer via `ChannelBridgeHandle::try_answer_ask_user` instead of
/// starting a new turn. The text content is only a defensive fallback in
/// case that metadata check is ever bypassed.
///
/// `platform_message_id` is set to the ORIGINAL keyboard message's id
/// (`original_message_id`), not the callback_query's own id.
/// `callback_command_message` uses the callback_query id there, which is
/// safe for `Command`-content messages (routed to the slash-command
/// dispatcher before `platform_message_id` is ever parsed as a Telegram
/// message id) but would corrupt `Text`-content messages here: the
/// streaming reply path parses `platform_message_id` as `i64` and uses it
/// as `reply_to_message_id`/edit target, and a callback_query id is not a
/// valid message id (discovered live during TG1 testing — 400 "message_id
/// must be a valid Number").
#[allow(clippy::too_many_arguments)]
pub(crate) fn ask_user_answer_callback_message(
    short_id: &str,
    idx: usize,
    chat_id: i64,
    from_id: i64,
    from_name: &str,
    thread_id: Option<String>,
    original_message_id: Option<i64>,
) -> ChannelMessage {
    let mut metadata = HashMap::new();
    metadata.insert("ask_user_short_id".to_string(), serde_json::json!(short_id));
    metadata.insert("ask_user_idx".to_string(), serde_json::json!(idx));
    metadata.insert("sender_user_id".to_string(), serde_json::json!(from_id));
    metadata.insert("chat_id".to_string(), serde_json::json!(chat_id));

    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: original_message_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: from_name.to_string(),
            captain_user: None,
        },
        content: ChannelContent::Text(format!("[ask_user answer: option {idx}]")),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id,
        metadata,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn capspec_operator_callback_message(
    callback_data: &str,
    chat_id: i64,
    from_id: i64,
    from_name: &str,
    thread_id: Option<String>,
    original_message_id: Option<i64>,
    original_text: &str,
) -> ChannelMessage {
    let mut metadata = HashMap::new();
    metadata.insert(
        "capspec_callback_data".to_string(),
        serde_json::json!(callback_data),
    );
    metadata.insert("sender_user_id".to_string(), serde_json::json!(from_id));
    metadata.insert("chat_id".to_string(), serde_json::json!(chat_id));
    metadata.insert(
        "original_text".to_string(),
        serde_json::json!(original_text),
    );

    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: original_message_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: from_name.to_string(),
            captain_user: None,
        },
        content: ChannelContent::Text("[Captain Forge operator decision]".to_string()),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id,
        metadata,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn workflow_learning_operator_callback_message(
    callback_data: &str,
    chat_id: i64,
    from_id: i64,
    from_name: &str,
    thread_id: Option<String>,
    original_message_id: Option<i64>,
    original_text: &str,
    language: &str,
) -> ChannelMessage {
    let mut metadata = HashMap::new();
    metadata.insert(
        "workflow_learning_callback_data".to_string(),
        serde_json::json!(callback_data),
    );
    metadata.insert("sender_user_id".to_string(), serde_json::json!(from_id));
    metadata.insert("chat_id".to_string(), serde_json::json!(chat_id));
    metadata.insert(
        "original_text".to_string(),
        serde_json::json!(original_text),
    );
    metadata.insert("language".to_string(), serde_json::json!(language));

    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: original_message_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: from_name.to_string(),
            captain_user: None,
        },
        content: ChannelContent::Text("[Workflow learning operator decision]".to_string()),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id,
        metadata,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn runtime_update_operator_callback_message(
    callback_data: &str,
    chat_id: i64,
    from_id: i64,
    from_name: &str,
    thread_id: Option<String>,
    original_message_id: Option<i64>,
    original_text: &str,
    language: &str,
) -> ChannelMessage {
    let mut metadata = HashMap::new();
    metadata.insert(
        "runtime_update_callback_data".to_string(),
        serde_json::json!(callback_data),
    );
    metadata.insert("sender_user_id".to_string(), serde_json::json!(from_id));
    metadata.insert("chat_id".to_string(), serde_json::json!(chat_id));
    metadata.insert(
        "original_text".to_string(),
        serde_json::json!(original_text),
    );
    metadata.insert("language".to_string(), serde_json::json!(language));

    ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: original_message_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: from_name.to_string(),
            captain_user: None,
        },
        content: ChannelContent::Text("[Captain runtime update decision]".to_string()),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capspec_callbacks_are_compact_and_strict() {
        let token = "a".repeat(CAPSPEC_CALLBACK_TOKEN_LEN);
        let callback = parse_capspec_callback(&format!("capspec:retry:{token}")).unwrap();
        assert_eq!(callback.action, CapSpecTelegramAction::Retry);
        assert_eq!(callback.token, token);
        assert!(parse_capspec_callback("capspec:retry:short").is_none());
        assert!(parse_capspec_callback("capspec:retry:0123456789abcdefabc!").is_none());
        assert!(parse_capspec_callback(&format!("capspec:unknown:{token}")).is_none());

        for keyboard in [
            build_capspec_approval_keyboard(&token),
            build_capspec_uncertain_keyboard(&token),
        ] {
            for row in keyboard["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    assert!(button["callback_data"].as_str().unwrap().len() <= 64);
                }
            }
        }
    }

    #[test]
    fn runtime_update_callbacks_are_compact_versioned_and_strict() {
        let token = "a".repeat(RUNTIME_UPDATE_CALLBACK_TOKEN_LEN);
        let callback =
            parse_runtime_update_callback(&format!("runtime_update:install:{token}:7")).unwrap();
        assert_eq!(callback.action, RuntimeUpdateTelegramAction::Install);
        assert_eq!(callback.token, token);
        assert_eq!(callback.decision_version, 7);
        assert!(parse_runtime_update_callback("runtime_update:install:short:7").is_none());
        assert!(
            parse_runtime_update_callback(&format!("runtime_update:install:{token}:0")).is_none()
        );
        assert!(
            parse_runtime_update_callback(&format!("runtime_update:unknown:{token}:7")).is_none()
        );
    }

    #[test]
    fn runtime_update_callback_envelope_preserves_operator_and_card() {
        let callback = "runtime_update:defer:0123456789abcdefabcd:4";
        let message = runtime_update_operator_callback_message(
            callback,
            -1001,
            42,
            "Vivien",
            Some("7".to_string()),
            Some(99),
            "Update card",
            "fr",
        );
        assert_eq!(message.platform_message_id, "99");
        assert_eq!(message.metadata["runtime_update_callback_data"], callback);
        assert_eq!(message.metadata["sender_user_id"], 42);
        assert_eq!(message.metadata["original_text"], "Update card");
        assert_eq!(message.metadata["language"], "fr");
        assert!(matches!(message.content, ChannelContent::Text(_)));
    }

    #[test]
    fn capspec_callback_envelope_preserves_human_and_original_message() {
        let callback = "capspec:approve:0123456789abcdefabcd";
        let message = capspec_operator_callback_message(
            callback,
            -1001,
            42,
            "Vivien",
            Some("7".to_string()),
            Some(99),
            "Captain Forge prompt",
        );
        assert_eq!(message.platform_message_id, "99");
        assert_eq!(message.metadata["capspec_callback_data"], callback);
        assert_eq!(message.metadata["sender_user_id"], 42);
        assert_eq!(message.metadata["original_text"], "Captain Forge prompt");
        assert!(matches!(message.content, ChannelContent::Text(_)));
    }

    #[test]
    fn workflow_callback_envelope_preserves_operator_message_and_language() {
        let callback = "workflow:activate:0123456789abcdefabcd:4";
        let message = workflow_learning_operator_callback_message(
            callback,
            -1001,
            42,
            "Vivien",
            Some("7".to_string()),
            Some(99),
            "Workflow proposal",
            "fr",
        );
        assert_eq!(message.platform_message_id, "99");
        assert_eq!(
            message.metadata["workflow_learning_callback_data"],
            callback
        );
        assert_eq!(message.metadata["sender_user_id"], 42);
        assert_eq!(message.metadata["original_text"], "Workflow proposal");
        assert_eq!(message.metadata["language"], "fr");
        assert!(matches!(message.content, ChannelContent::Text(_)));
    }

    #[test]
    fn telegram_callbacks_parse_approval_session_and_deny() {
        assert_eq!(
            parse_approval_callback("approval:session:req-42"),
            Some(("approve_session".to_string(), vec!["req-42".to_string()]))
        );
        assert_eq!(
            parse_approval_callback("approval:deny:req-42"),
            Some(("reject".to_string(), vec!["req-42".to_string()]))
        );
        assert!(parse_approval_callback("approval:later:req-42").is_none());
    }

    #[test]
    fn telegram_callbacks_parse_ask_user_by_index() {
        assert_eq!(
            parse_ask_user_callback("ask_user:short-1:2"),
            Some(("short-1".to_string(), 2))
        );
        assert!(parse_ask_user_callback("ask_user:short-1:freeform").is_none());
        assert!(parse_ask_user_callback("ask_user::2").is_none());
        assert!(parse_ask_user_callback("project_ask:short-1:2").is_none());
    }

    #[test]
    fn telegram_callbacks_ask_user_keyboard_caps_options_and_uses_indices() {
        let options: Vec<String> = (0..8)
            .map(|idx| format!("Option {idx} avec une etiquette longue qui sera tronquee"))
            .collect();
        let kb = build_ask_user_keyboard("short-1", &options);
        let rows = kb["inline_keyboard"].as_array().expect("rows");

        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0][0]["callback_data"], "ask_user:short-1:0");
        assert_eq!(rows[5][0]["callback_data"], "ask_user:short-1:5");
        let label = rows[0][0]["text"].as_str().expect("label");
        assert!(label.ends_with('…'));
        assert!(label.chars().count() <= 45);
    }

    #[test]
    fn telegram_callbacks_route_project_ask_by_index() {
        assert_eq!(
            parse_project_ask_callback("project_ask:ask-42:2"),
            Some((
                "project_answer".to_string(),
                vec!["ask-42".to_string(), "@idx:2".to_string()]
            ))
        );
        assert!(parse_project_ask_callback("project_ask:ask-42:freeform").is_none());
    }

    #[test]
    fn telegram_callbacks_project_ask_keyboard_caps_options_and_uses_indices() {
        let options: Vec<String> = (0..8)
            .map(|idx| format!("Option {idx} avec une etiquette longue qui sera tronquee"))
            .collect();
        let kb = build_project_ask_keyboard("ask-42", &options);
        let rows = kb["inline_keyboard"].as_array().expect("rows");

        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0][0]["callback_data"], "project_ask:ask-42:0");
        assert_eq!(rows[5][0]["callback_data"], "project_ask:ask-42:5");
    }

    #[test]
    fn telegram_callbacks_model_keyboard_marks_recommendation() {
        let kb = build_model_switch_keyboard_with_recommendation("plan-42", Some("new_session"));
        assert_eq!(
            kb["inline_keyboard"][0][0]["text"],
            "Nouvelle session (recommandé)"
        );
        assert_eq!(
            kb["inline_keyboard"][0][1]["callback_data"],
            "model:plan-42:compact_session"
        );
    }

    #[test]
    fn telegram_callbacks_command_message_preserves_chat_thread_and_clicking_user() {
        let msg = callback_command_message(
            "callback-1",
            "project_ask:ask-42:1",
            "project_answer".to_string(),
            vec!["ask-42".to_string(), "@idx:1".to_string()],
            -100123,
            456,
            "Alex",
            Some("topic-7".to_string()),
        );

        assert_eq!(msg.sender.platform_id, "-100123");
        assert_eq!(msg.thread_id.as_deref(), Some("topic-7"));
        assert_eq!(msg.metadata["sender_user_id"], serde_json::json!(456));
        assert_eq!(msg.metadata["chat_id"], serde_json::json!(-100123));
        match msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "project_answer");
                assert_eq!(args, vec!["ask-42".to_string(), "@idx:1".to_string()]);
            }
            other => panic!("expected command content, got {other:?}"),
        }
    }

    #[test]
    fn telegram_callbacks_ask_user_answer_message_carries_text_not_command() {
        let msg = ask_user_answer_callback_message(
            "short-1",
            2,
            -100123,
            456,
            "Alex",
            Some("topic-7".to_string()),
            Some(999),
        );

        assert_eq!(msg.sender.platform_id, "-100123");
        assert_eq!(msg.thread_id.as_deref(), Some("topic-7"));
        // The original keyboard message id, NOT the callback_query id —
        // this is the exact bug found live during TG1 testing when the
        // (structurally similar) callback_command_message() pattern was
        // first tried for a Text-content message.
        assert_eq!(msg.platform_message_id, "999");
        assert_eq!(
            msg.metadata["ask_user_short_id"],
            serde_json::json!("short-1")
        );
        assert_eq!(msg.metadata["ask_user_idx"], serde_json::json!(2));
        match msg.content {
            ChannelContent::Text(_) => {}
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn telegram_callbacks_ask_user_answer_message_handles_missing_original_message_id() {
        let msg = ask_user_answer_callback_message("short-1", 0, -100123, 456, "Alex", None, None);

        assert_eq!(msg.platform_message_id, "");
    }
}
