//! Commit C.2 — route the `🧠 mémorisé` notice back to its origin channel.
//!
//! When the learning pipeline commits a memory, the kernel publishes a
//! `ChatStreamEvent::MemoryStored` with the origin canal carried by
//! Commit C.1. This module spawns a tiny subscriber that listens for
//! those events and, when the canal matches a configured adapter,
//! emits a one-line notice on it so the user sees the memory land
//! in the same conversation it came from.
//!
//! Helpers are pulled out as pure functions so the routing decision and
//! formatting are unit-testable without any IO.
//!
//! Fire-and-forget on send errors — the notice is best-effort and must
//! never block the agent loop.
//!
//! Scope: Telegram for now (the user's primary external canal). Other
//! adapters reuse `should_route()` and `format_memory_notice()` and just
//! plug in their own send path.

use captain_channels::telegram::{
    build_capspec_approval_keyboard, build_capspec_uncertain_keyboard,
    build_learning_approval_keyboard, build_project_ask_keyboard, build_skill_refinement_keyboard,
    TelegramAdapter,
};
use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_types::event::{ChatStreamEvent, EventPayload};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

use crate::kernel::{CapSpecTelegramPrompt, CapSpecTelegramPromptKind, CaptainKernel};

/// Render the discreet `🧠` notice that we surface back to the user.
/// Kept short and deterministic so subscribers across channels look
/// identical.
pub fn format_memory_notice(subject: &str, predicate: &str, object: &str) -> String {
    format!("🧠 mémorisé : {subject} {predicate} {object}")
}

/// Decide whether a `ChatStreamEvent` should be relayed to a given
/// channel target. Returns `Some(formatted_notice)` when the event
/// is a `MemoryStored` whose canal field exactly matches `target_channel`,
/// `None` otherwise.
pub fn should_route(event: &ChatStreamEvent, target_channel: &str) -> Option<String> {
    if let ChatStreamEvent::MemoryStored {
        subject,
        predicate,
        object,
        channel: Some(canal),
        ..
    } = event
    {
        if canal == target_channel {
            return Some(format_memory_notice(subject, predicate, object));
        }
    }
    None
}

/// Render the approval prompt body — the lines users see above the
/// inline-keyboard buttons. Identifies the candidate by its review_id
/// so support can correlate later if needed.
pub fn format_approval_prompt(
    review_id: &str,
    subject: &str,
    predicate: &str,
    object: &str,
) -> String {
    format!(
        "💭 Mémoire en attente d'approbation\n\
         {subject} {predicate} {object}\n\
         <code>{review_id}</code>"
    )
}

/// Decide whether a `ChatStreamEvent` is an approval prompt that
/// should be relayed to `target_channel`. Returns the review_id +
/// formatted body when matching, `None` otherwise.
pub fn should_route_approval(
    event: &ChatStreamEvent,
    target_channel: &str,
) -> Option<(String, String)> {
    if let ChatStreamEvent::MemoryQueued {
        review_id,
        subject,
        predicate,
        object,
        channel: Some(canal),
        ..
    } = event
    {
        if canal == target_channel {
            return Some((
                review_id.clone(),
                format_approval_prompt(review_id, subject, predicate, object),
            ));
        }
    }
    None
}

/// Extract a learning approval prompt for the configured preferred channel.
///
/// This is intentionally broader than `should_route_approval`: human
/// validation is a control-plane action, so when Telegram is configured as
/// the preferred approval surface we send every learning review prompt there,
/// even if the candidate originated from web/CLI or has no channel metadata.
pub fn should_route_preferred_approval(event: &ChatStreamEvent) -> Option<(String, String)> {
    if let ChatStreamEvent::MemoryQueued {
        review_id,
        subject,
        predicate,
        object,
        ..
    } = event
    {
        return Some((
            review_id.clone(),
            format_approval_prompt(review_id, subject, predicate, object),
        ));
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn format_skill_proposal_prompt(
    proposal_id: &str,
    name: &str,
    description: &str,
    trigger_hint: &str,
    tool_sequence: &[String],
    confidence: f32,
    family: Option<&str>,
    language: &str,
) -> String {
    let french = skill_proposal_language_is_french(language);
    let tools = if tool_sequence.is_empty() {
        if french {
            "Aucune trace d'outil automatique n'a été capturée. Les étapes réutilisables doivent être décrites clairement dans le résumé ci-dessus.".to_string()
        } else {
            "No automatic tool trace was captured. The reusable steps must be clearly described in the summary above.".to_string()
        }
    } else {
        tool_sequence
            .iter()
            .enumerate()
            .map(|(idx, tool)| format!("{}. {}", idx + 1, tool))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let description = description.to_string();
    let trigger_hint = trigger_hint.to_string();
    let why = if french && tool_sequence.is_empty() {
        "Archive v3.13 : aucune étape outillée observée, sans preuve de validation SKILL2."
            .to_string()
    } else if french {
        format!(
            "Archive v3.13 : {} étape(s) observée(s), sans preuve de validation SKILL2.",
            tool_sequence.len()
        )
    } else if tool_sequence.is_empty() {
        "v3.13 archive: no tool step observed, without SKILL2 validation evidence.".to_string()
    } else {
        format!(
            "v3.13 archive: {} observed step(s), without SKILL2 validation evidence.",
            tool_sequence.len()
        )
    };
    let family = family
        .and_then(captain_skills::families::known_family)
        .map(|f| {
            if french {
                french_skill_family_label(f.id)
            } else {
                f.label
            }
        })
        .unwrap_or(if french {
            "Automatisation générale"
        } else {
            "General automation"
        });
    let labels = skill_proposal_labels(french);
    format!(
        "🛠️ {title}\n\n\
         <b>{name}</b>\n\n\
         <b>{purpose}</b>\n\
         {description}\n\n\
         <b>{trigger}</b>\n\
         {trigger_hint}\n\n\
         <b>{why_label}</b>\n\
         {why}\n\n\
         <b>{family_label}</b>\n\
         {family}\n\n\
         <b>{tools_label}</b>\n\
         {tools}\n\n\
         <b>{confidence_label}</b> : {:.0}%\n\
         <b>ID</b> : <code>{proposal_id}</code>\n\n\
         {decision_help}",
        confidence * 100.0,
        title = labels.title,
        purpose = labels.purpose,
        trigger = labels.trigger,
        why_label = labels.why_label,
        family_label = labels.family,
        tools_label = labels.tools,
        confidence_label = labels.confidence,
        decision_help = labels.decision_help,
        name = escape_telegram_html(name),
        description = escape_telegram_html(&description),
        trigger_hint = escape_telegram_html(&trigger_hint),
        why = escape_telegram_html(&why),
        family = escape_telegram_html(family),
        tools = escape_telegram_html(&tools),
        proposal_id = escape_telegram_html(proposal_id),
    )
}

struct SkillProposalLabels {
    title: &'static str,
    purpose: &'static str,
    trigger: &'static str,
    why_label: &'static str,
    family: &'static str,
    tools: &'static str,
    confidence: &'static str,
    decision_help: &'static str,
}

fn skill_proposal_labels(french: bool) -> SkillProposalLabels {
    if french {
        SkillProposalLabels {
            title: "Proposition de skill archivée",
            purpose: "À quoi il servira",
            trigger: "Quand Captain l'utilisera",
            why_label: "Ce que Captain a observé",
            family: "Famille",
            tools: "Étapes / outils observés",
            confidence: "Confiance",
            decision_help: "Cette proposition historique est en lecture seule. Consulte Learning pour les workflows SKILL2 vérifiés.",
        }
    } else {
        SkillProposalLabels {
            title: "Archived skill proposal",
            purpose: "What it will do",
            trigger: "When Captain will use it",
            why_label: "What Captain observed",
            family: "Family",
            tools: "Observed steps / tools",
            confidence: "Confidence",
            decision_help: "This historical proposal is read-only. Open Learning for verified SKILL2 workflows.",
        }
    }
}

fn skill_proposal_language_is_french(language: &str) -> bool {
    let lang = language.trim().to_ascii_lowercase();
    lang.starts_with("fr") || lang.contains("français") || lang.contains("francais")
}

fn french_skill_family_label(id: &str) -> &'static str {
    match id {
        "software-development" => "Développement logiciel",
        "project-management" => "Gestion de projet",
        "review-release" => "Review et publication",
        "platform-devops" => "Plateforme et DevOps",
        "data-ai" => "Data et IA",
        "product-design" => "Produit et design",
        "business-tools" => "Outils métier",
        "security-compliance" => "Sécurité et conformité",
        _ => "Automatisation générale",
    }
}

fn escape_telegram_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn should_route_skill_proposal(
    event: &ChatStreamEvent,
    target_channel: &str,
) -> Option<(String, String)> {
    if let ChatStreamEvent::SkillProposalQueued {
        proposal_id,
        name,
        description,
        trigger_hint,
        tool_sequence,
        confidence,
        family,
        language,
        channel: Some(canal),
        ..
    } = event
    {
        if canal == target_channel {
            return Some((
                proposal_id.clone(),
                format_skill_proposal_prompt(
                    proposal_id,
                    name,
                    description,
                    trigger_hint,
                    tool_sequence,
                    *confidence,
                    family.as_deref(),
                    language.as_deref().unwrap_or("fr"),
                ),
            ));
        }
    }
    None
}

/// Extract a skill proposal prompt for the configured preferred channel.
///
/// Skill proposals are also validation/control-plane items, so they follow
/// the same preferred-channel rule as learning approvals.
pub fn should_route_preferred_skill_proposal(event: &ChatStreamEvent) -> Option<(String, String)> {
    if let ChatStreamEvent::SkillProposalQueued {
        proposal_id,
        name,
        description,
        trigger_hint,
        tool_sequence,
        confidence,
        family,
        language,
        ..
    } = event
    {
        return Some((
            proposal_id.clone(),
            format_skill_proposal_prompt(
                proposal_id,
                name,
                description,
                trigger_hint,
                tool_sequence,
                *confidence,
                family.as_deref(),
                language.as_deref().unwrap_or("fr"),
            ),
        ));
    }
    None
}

pub fn format_skill_refinement_prompt(
    refinement_id: &str,
    skill: &str,
    finding: &str,
    suggested_change: &str,
    risk: &str,
    source: &str,
) -> String {
    format!(
        "🛠️ Amélioration de skill proposée\n\
         <b>{skill}</b>\n\
         Signal: {finding}\n\
         Changement: {suggested_change}\n\
         Risque: {risk} · source: {source}\n\
         <code>{refinement_id}</code>"
    )
}

pub fn should_route_skill_refinement(
    event: &ChatStreamEvent,
    target_channel: &str,
) -> Option<(String, String)> {
    if let ChatStreamEvent::SkillRefinementQueued {
        refinement_id,
        skill,
        finding,
        suggested_change,
        risk,
        source,
        channel: Some(canal),
    } = event
    {
        if canal == target_channel {
            return Some((
                refinement_id.clone(),
                format_skill_refinement_prompt(
                    refinement_id,
                    skill,
                    finding,
                    suggested_change,
                    risk,
                    source,
                ),
            ));
        }
    }
    None
}

/// Extract an existing-skill refinement prompt for the preferred validation
/// channel. Refinements are critical durable self-improvement items, so they
/// use the same preferred-channel rule as learning and generated skills.
pub fn should_route_preferred_skill_refinement(
    event: &ChatStreamEvent,
) -> Option<(String, String)> {
    if let ChatStreamEvent::SkillRefinementQueued {
        refinement_id,
        skill,
        finding,
        suggested_change,
        risk,
        source,
        ..
    } = event
    {
        return Some((
            refinement_id.clone(),
            format_skill_refinement_prompt(
                refinement_id,
                skill,
                finding,
                suggested_change,
                risk,
                source,
            ),
        ));
    }
    None
}

pub fn format_project_ask_prompt(
    ask_id: &str,
    project_name: &str,
    project_slug: &str,
    phase: &str,
    worker_role: &str,
    question: &str,
    options: Option<&[String]>,
) -> String {
    let mut lines = vec![
        "❓ Question projet".to_string(),
        String::new(),
        format!("<b>Projet</b> : {}", escape_telegram_html(project_name)),
        format!(
            "<b>Slug</b> : <code>{}</code>",
            escape_telegram_html(project_slug)
        ),
        format!(
            "<b>Phase</b> : {} · <b>agent</b> : {}",
            escape_telegram_html(phase),
            escape_telegram_html(worker_role)
        ),
        String::new(),
        "<b>Question</b>".to_string(),
        escape_telegram_html(question),
    ];
    if let Some(options) = options {
        if !options.is_empty() {
            lines.push(String::new());
            lines.push("<b>Choix proposés</b>".to_string());
            for (idx, option) in options.iter().take(6).enumerate() {
                lines.push(format!("{}. {}", idx + 1, escape_telegram_html(option)));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Réponds avec les boutons ou en texte libre : <code>/project_answer {} ta réponse</code>",
        escape_telegram_html(&ask_id.chars().take(8).collect::<String>())
    ));
    lines.join("\n")
}

pub fn should_route_preferred_project_ask(
    event: &ChatStreamEvent,
) -> Option<(String, String, Vec<String>)> {
    if let ChatStreamEvent::ProjectAskUser {
        ask_id,
        project_slug,
        project_name,
        phase,
        worker_role,
        question,
        options,
        ..
    } = event
    {
        return Some((
            ask_id.clone(),
            format_project_ask_prompt(
                ask_id,
                project_name,
                project_slug,
                phase,
                worker_role,
                question,
                options.as_deref(),
            ),
            options.clone().unwrap_or_default(),
        ));
    }
    None
}

/// Spawn the background task that subscribes to the event bus and
/// forwards `🧠 mémorisé` notices to the Telegram adapter when the
/// committed memory came from a Telegram conversation.
///
/// Returns the `JoinHandle` for shutdown bookkeeping. Never panics —
/// send failures are logged at warn and the loop continues.
/// Spawn the background task that surfaces interactive memory approvals
/// on Telegram. Telegram is treated as the configured preferred validation
/// channel: every `ChatStreamEvent::MemoryQueued` is posted there, regardless
/// of whether it originated from Telegram, web, CLI, or a background job.
/// The prompt includes learning-specific inline buttons (`approve` / `reject`).
/// These callbacks resolve `learning_review_decide`, not the generic tool
/// approval manager.
///
/// `chat_id_str` is parsed once at boot (the bot API expects an i64);
/// when parsing fails, the routing task is not spawned and the user
/// will keep using the dashboard for approvals.
pub fn spawn_telegram_memory_approval_routing(
    event_bus: crate::event_bus::EventBus,
    adapter: Arc<TelegramAdapter>,
    chat_id: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = event_bus.subscribe_all();
        while let Ok(event) = rx.recv().await {
            if let EventPayload::ChatStream(stream_ev) = &event.payload {
                if let Some((review_id, text)) = should_route_preferred_approval(stream_ev) {
                    let keyboard = build_learning_approval_keyboard(&review_id);
                    if let Err(e) = adapter
                        .send_text_with_keyboard(chat_id, &text, &keyboard)
                        .await
                    {
                        warn!(
                            error = %e,
                            review_id = %review_id,
                            "telegram memory approval routing send failed"
                        );
                    } else {
                        debug!(review_id = %review_id, "telegram memory approval prompt sent");
                    }
                }
            }
        }
    })
}

pub fn spawn_telegram_skill_refinement_routing(
    event_bus: crate::event_bus::EventBus,
    adapter: Arc<TelegramAdapter>,
    chat_id: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = event_bus.subscribe_all();
        while let Ok(event) = rx.recv().await {
            if let EventPayload::ChatStream(stream_ev) = &event.payload {
                if let Some((refinement_id, text)) =
                    should_route_preferred_skill_refinement(stream_ev)
                {
                    let keyboard = build_skill_refinement_keyboard(&refinement_id);
                    if let Err(e) = adapter
                        .send_text_with_keyboard(chat_id, &text, &keyboard)
                        .await
                    {
                        warn!(
                            error = %e,
                            refinement_id = %refinement_id,
                            "telegram skill refinement routing send failed"
                        );
                    } else {
                        debug!(refinement_id = %refinement_id, "telegram skill refinement prompt sent");
                    }
                }
            }
        }
    })
}

pub fn spawn_telegram_project_ask_routing(
    event_bus: crate::event_bus::EventBus,
    adapter: Arc<TelegramAdapter>,
    chat_id: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = event_bus.subscribe_all();
        while let Ok(event) = rx.recv().await {
            if let EventPayload::ChatStream(stream_ev) = &event.payload {
                if let Some((ask_id, text, options)) = should_route_preferred_project_ask(stream_ev)
                {
                    let result = if options.is_empty() {
                        let user = ChannelUser {
                            platform_id: chat_id.to_string(),
                            display_name: chat_id.to_string(),
                            captain_user: None,
                        };
                        adapter
                            .send(&user, ChannelContent::Text(text.clone()))
                            .await
                    } else {
                        let keyboard = build_project_ask_keyboard(&ask_id, &options);
                        adapter
                            .send_text_with_keyboard(chat_id, &text, &keyboard)
                            .await
                    };
                    if let Err(e) = result {
                        warn!(
                            error = %e,
                            ask_id = %ask_id,
                            "telegram project ask-user routing send failed"
                        );
                    } else {
                        debug!(ask_id = %ask_id, "telegram project ask-user prompt sent");
                    }
                }
            }
        }
    })
}

/// Surface every currently actionable Captain Forge decision on Telegram.
///
/// This is a state scanner rather than a transient-event subscriber: pending
/// approvals and crash-recovery decisions therefore reappear after a daemon
/// restart. The deterministic identity set suppresses duplicates while an
/// adapter instance is alive, and the adapter shutdown signal terminates the
/// task during channel hot reload.
pub fn spawn_telegram_capspec_routing(
    kernel: Arc<CaptainKernel>,
    adapter: Arc<TelegramAdapter>,
    chat_id: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut sent = BTreeSet::new();
        let mut shutdown = adapter.shutdown_signal();
        loop {
            if *shutdown.borrow() {
                break;
            }
            match kernel.capspec_telegram_prompts() {
                Ok(prompts) => {
                    let current = prompts
                        .iter()
                        .map(capspec_prompt_identity)
                        .collect::<BTreeSet<_>>();
                    sent.retain(|identity| current.contains(identity));
                    for prompt in prompts {
                        let identity = capspec_prompt_identity(&prompt);
                        if sent.contains(&identity) {
                            continue;
                        }
                        let (text, keyboard) = format_capspec_telegram_prompt(&prompt);
                        match adapter
                            .send_text_with_keyboard(chat_id, &text, &keyboard)
                            .await
                        {
                            Ok(()) => {
                                sent.insert(identity);
                                debug!(token = %prompt.token, "Telegram CapSpec operator prompt sent");
                            }
                            Err(error) => warn!(
                                error = %error,
                                token = %prompt.token,
                                "Telegram CapSpec operator routing send failed"
                            ),
                        }
                    }
                }
                Err(error) => warn!(error = %error, "Telegram CapSpec operator scan failed"),
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    })
}

pub fn format_capspec_telegram_prompt(
    prompt: &CapSpecTelegramPrompt,
) -> (String, serde_json::Value) {
    match prompt.kind {
        CapSpecTelegramPromptKind::Approval => {
            let authority = if prompt.authority.is_empty() {
                "- no expanded authority".to_string()
            } else {
                prompt
                    .authority
                    .iter()
                    .map(|line| format!("- {}", telegram_code(line)))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            (
                format!(
                "🧩 **Captain Forge — validation requise**\n\n\
                     **{}** v{}\n\
                     Description : `{}`\n\n\
                     Portée : `{}`\n\
                     Hash source exact : `{}`\n\n\
                     **Autorité demandée**\n{}\n\n\
                     La décision s'applique uniquement au hash exact ci-dessus.",
                    telegram_code(&prompt.name),
                    telegram_code(&prompt.version),
                    telegram_inline_text(&prompt.description),
                    telegram_code(&prompt.scope),
                    prompt.source_hash,
                    authority,
                ),
                build_capspec_approval_keyboard(&prompt.token),
            )
        }
        CapSpecTelegramPromptKind::Uncertain => (
            format!(
                "⚠️ **Captain Forge — décision runtime requise**\n\n\
                 **{}** ne peut pas prouver si l'effet externe a abouti.\n\n\
                 Run: `{}`\n\
                 Hash source : `{}`\n\
                 Portée : `{}`\n\
                 Nœud : `{}`\n\
                 Outil : `{}`\n\
                 Tentative : {}\n\
                 Tool use : `{}`\n\
                 Origine : `{}`\n\n\
                 Réessayer peut répéter l'effet. Confirmer enregistre une sortie `null` ; utilise Control ou l'API si une sortie structurée est nécessaire. Marquer en échec arrête le run.",
                telegram_code(&prompt.name),
                telegram_code(prompt.run_id.as_deref().unwrap_or("unknown")),
                prompt.source_hash,
                telegram_code(&prompt.scope),
                telegram_code(prompt.node_id.as_deref().unwrap_or("unknown")),
                telegram_code(prompt.tool_name.as_deref().unwrap_or("unknown")),
                prompt.attempt.unwrap_or_default(),
                telegram_code(prompt.tool_use_id.as_deref().unwrap_or("unknown")),
                telegram_code(prompt.origin.as_deref().unwrap_or("unknown")),
            ),
            build_capspec_uncertain_keyboard(&prompt.token),
        ),
    }
}

fn capspec_prompt_identity(prompt: &CapSpecTelegramPrompt) -> String {
    format!("{:?}:{}", prompt.kind, prompt.token)
}

fn telegram_code(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| if character == '`' { '\'' } else { character })
        .collect()
}

fn telegram_inline_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || character == '`' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn spawn_telegram_memory_routing(
    event_bus: crate::event_bus::EventBus,
    adapter: Arc<dyn ChannelAdapter>,
    default_chat_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = event_bus.subscribe_all();
        while let Ok(event) = rx.recv().await {
            if let EventPayload::ChatStream(stream_ev) = &event.payload {
                if let Some(text) = should_route(stream_ev, "telegram") {
                    let user = ChannelUser {
                        platform_id: default_chat_id.clone(),
                        display_name: default_chat_id.clone(),
                        captain_user: None,
                    };
                    let content = ChannelContent::Text(text.clone());
                    if let Err(e) = adapter.send(&user, content).await {
                        warn!(error = %e, text = %text, "telegram memory routing send failed");
                    } else {
                        debug!(text = %text, "telegram memory routing: 🧠 sent");
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capspec_approval_prompt_keeps_exact_hash_and_compact_callback() {
        let prompt = CapSpecTelegramPrompt {
            kind: CapSpecTelegramPromptKind::Approval,
            token: "a".repeat(20),
            name: "release-check".to_string(),
            scope: "global".to_string(),
            description: "Check **one** `release`.\nNo spoofing.".to_string(),
            version: "1.2.3".to_string(),
            source_hash: "f".repeat(64),
            authority: vec!["network hosts: api.example.com".to_string()],
            run_id: None,
            node_id: None,
            tool_name: None,
            tool_use_id: None,
            attempt: None,
            origin: None,
        };
        let (text, keyboard) = format_capspec_telegram_prompt(&prompt);
        assert!(text.contains(&"f".repeat(64)));
        assert!(text.contains("api.example.com"));
        assert!(text.contains("Description : `Check **one** release . No spoofing.`"));
        let callback = keyboard["inline_keyboard"][0][0]["callback_data"]
            .as_str()
            .unwrap();
        assert!(callback.len() <= 64);
        assert!(captain_channels::telegram::parse_capspec_callback(callback).is_some());
    }

    #[test]
    fn capspec_uncertain_prompt_explains_retry_and_null_confirmation() {
        let prompt = CapSpecTelegramPrompt {
            kind: CapSpecTelegramPromptKind::Uncertain,
            token: "b".repeat(20),
            name: "deploy".to_string(),
            scope: "project (/tmp/repo)".to_string(),
            description: String::new(),
            version: String::new(),
            source_hash: "a".repeat(64),
            authority: Vec::new(),
            run_id: Some("run-1".to_string()),
            node_id: Some("publish".to_string()),
            tool_name: Some("http_request".to_string()),
            tool_use_id: Some("tool-use-1".to_string()),
            attempt: Some(2),
            origin: Some("telegram".to_string()),
        };
        let (text, keyboard) = format_capspec_telegram_prompt(&prompt);
        assert!(text.contains("peut répéter l'effet"));
        assert!(text.contains("sortie `null`"));
        assert_eq!(keyboard["inline_keyboard"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn format_notice_is_a_single_line_with_brain_emoji() {
        let s = format_memory_notice("user", "prefers_color", "rouge brique");
        assert!(s.starts_with("🧠"));
        assert!(s.contains("user"));
        assert!(s.contains("prefers_color"));
        assert!(s.contains("rouge brique"));
        assert!(!s.contains('\n'), "notice must stay on one line");
    }

    #[test]
    fn should_route_matches_only_telegram_when_target_is_telegram() {
        let ev = ChatStreamEvent::MemoryStored {
            subject: "user".into(),
            predicate: "prefers".into(),
            object: "dark mode".into(),
            source: "learning.conversation_turn".into(),
            wing: "learnings".into(),
            room: "user_preferences".into(),
            channel: Some("telegram".into()),
            category: Some("info".into()),
        };
        assert!(should_route(&ev, "telegram").is_some());
        assert!(should_route(&ev, "discord").is_none());
        assert!(should_route(&ev, "cli").is_none());
    }

    #[test]
    fn should_route_returns_none_when_channel_field_absent() {
        let ev = ChatStreamEvent::MemoryStored {
            subject: "x".into(),
            predicate: "y".into(),
            object: "z".into(),
            source: "learning".into(),
            wing: "learnings".into(),
            room: "general".into(),
            channel: None,
            category: None,
        };
        assert!(should_route(&ev, "telegram").is_none());
    }

    #[test]
    fn should_route_returns_none_for_non_memory_events() {
        let ev = ChatStreamEvent::TextDelta {
            agent_id: captain_types::agent::AgentId::default(),
            delta: "hello".into(),
        };
        assert!(should_route(&ev, "telegram").is_none());
    }

    #[test]
    fn format_approval_prompt_includes_triple_and_review_id() {
        let s = format_approval_prompt("rev-42", "user", "prefers", "dark mode");
        assert!(s.contains("user prefers dark mode"));
        assert!(s.contains("rev-42"));
        assert!(s.contains("approbation"));
        assert!(s.contains("💭"));
    }

    #[test]
    fn should_route_skill_proposal_matches_only_target_channel() {
        let ev = ChatStreamEvent::SkillProposalQueued {
            proposal_id: "prop-42".into(),
            name: "status-checker".into(),
            description: "Checks service health".into(),
            trigger_hint: "user asks for a health check".into(),
            tool_sequence: vec!["ssh_exec".into(), "shell_exec".into()],
            confidence: 0.82,
            family: Some("software-development".into()),
            language: Some("fr".into()),
            source_agent_id: "captain".into(),
            channel: Some("telegram".into()),
        };
        let (pid, body) = should_route_skill_proposal(&ev, "telegram").unwrap();
        assert_eq!(pid, "prop-42");
        assert!(body.contains("status-checker"));
        assert!(should_route_skill_proposal(&ev, "cli").is_none());
    }

    #[test]
    fn preferred_skill_proposal_routes_even_from_non_telegram_channel() {
        let ev = ChatStreamEvent::SkillProposalQueued {
            proposal_id: "prop-cli".into(),
            name: "status-checker".into(),
            description: "Checks service health".into(),
            trigger_hint: "user asks for a health check".into(),
            tool_sequence: vec!["ssh_exec".into()],
            confidence: 0.82,
            family: Some("software-development".into()),
            language: Some("fr".into()),
            source_agent_id: "captain".into(),
            channel: Some("cli".into()),
        };
        let (pid, body) = should_route_preferred_skill_proposal(&ev).unwrap();
        assert_eq!(pid, "prop-cli");
        assert!(body.contains("status-checker"));
    }

    #[test]
    fn format_skill_proposal_prompt_marks_legacy_trigger_as_archived_read_only() {
        let body = format_skill_proposal_prompt(
            "prop-fr",
            "smoke-check",
            "printf '1\\n+\\n2\\n' | python3 main.py",
            "When a future task matches `Smoke check approach` / `uses command` and needs this reusable workflow.",
            &[],
            0.8,
            Some("general-automation"),
            "fr",
        );
        assert!(body.contains("Proposition de skill archivée"));
        assert!(body.contains("When a future task matches"));
        assert!(body.contains("Aucune trace d'outil automatique"));
        assert!(body.contains("Ce que Captain a observé"));
        assert!(body.contains("Archive v3.13"));
        assert!(body.contains("lecture seule"));
        assert!(!body.contains("0 étape"));
        assert!(body.contains("Automatisation générale"));
        assert!(!body.contains("tools:"));
    }

    #[test]
    fn format_skill_proposal_prompt_preserves_archived_payload_without_rewriting() {
        let body = format_skill_proposal_prompt(
            "prop-health",
            "status-checker",
            "Checks service health",
            "user asks for a health check",
            &["ssh_exec".into()],
            0.82,
            Some("software-development"),
            "fr",
        );
        assert!(body.contains("Proposition de skill archivée"));
        assert!(body.contains("Checks service health"));
        assert!(body.contains("user asks for a health check"));
        assert!(body.contains("Développement logiciel"));
        assert!(body.contains("sans preuve de validation SKILL2"));
    }

    #[test]
    fn format_skill_proposal_prompt_can_render_archived_notice_in_english() {
        let body = format_skill_proposal_prompt(
            "prop-en",
            "status-checker",
            "Checks service health",
            "user asks for a health check",
            &["ssh_exec".into()],
            0.82,
            Some("software-development"),
            "en",
        );
        assert!(body.contains("Archived skill proposal"));
        assert!(body.contains("What it will do"));
        assert!(body.contains("Observed steps / tools"));
        assert!(body.contains("Software development"));
        assert!(body.contains("Checks service health"));
        assert!(body.contains("v3.13 archive"));
        assert!(body.contains("historical proposal is read-only"));
        assert!(!body.contains("À quoi il servira"));
    }

    #[test]
    fn should_route_skill_refinement_matches_only_target_channel() {
        let ev = ChatStreamEvent::SkillRefinementQueued {
            refinement_id: "ref-42".into(),
            skill: "status-checker".into(),
            finding: "Missing retry guard".into(),
            suggested_change: "Document the retry guard".into(),
            risk: "low".into(),
            source: "skill_use".into(),
            channel: Some("telegram".into()),
        };
        let (rid, body) = should_route_skill_refinement(&ev, "telegram").unwrap();
        assert_eq!(rid, "ref-42");
        assert!(body.contains("status-checker"));
        assert!(body.contains("Missing retry guard"));
        assert!(should_route_skill_refinement(&ev, "cli").is_none());
    }

    #[test]
    fn preferred_skill_refinement_routes_even_from_non_telegram_channel() {
        let ev = ChatStreamEvent::SkillRefinementQueued {
            refinement_id: "ref-cli".into(),
            skill: "status-checker".into(),
            finding: "Missing retry guard".into(),
            suggested_change: "Document the retry guard".into(),
            risk: "medium".into(),
            source: "skill_use".into(),
            channel: Some("cli".into()),
        };
        let (rid, body) = should_route_preferred_skill_refinement(&ev).unwrap();
        assert_eq!(rid, "ref-cli");
        assert!(body.contains("status-checker"));
    }

    #[test]
    fn format_project_ask_prompt_includes_project_context_and_answer_path() {
        let body = format_project_ask_prompt(
            "ask-123456789",
            "Calculatrice Python",
            "calculatrice-python",
            "plan",
            "Planner",
            "Dois-je créer une CLI ou une interface graphique ?",
            Some(&["CLI".into(), "Interface graphique".into()]),
        );
        assert!(body.contains("Question projet"));
        assert!(body.contains("Calculatrice Python"));
        assert!(body.contains("calculatrice-python"));
        assert!(body.contains("Planner"));
        assert!(body.contains("1. CLI"));
        assert!(body.contains("/project_answer ask-1234"));
    }

    #[test]
    fn preferred_project_ask_routes_with_buttons_payload() {
        let ev = ChatStreamEvent::ProjectAskUser {
            agent_id: captain_types::agent::AgentId::default(),
            ask_id: "ask-42".into(),
            project_id: "project-1".into(),
            project_slug: "demo".into(),
            project_name: "Demo".into(),
            phase: "verify".into(),
            worker_role: "Verifier".into(),
            question: "Quel niveau de test ?".into(),
            options: Some(vec!["Smoke".into(), "Complet".into()]),
        };
        let (ask_id, body, options) = should_route_preferred_project_ask(&ev).unwrap();
        assert_eq!(ask_id, "ask-42");
        assert_eq!(options, vec!["Smoke".to_string(), "Complet".to_string()]);
        assert!(body.contains("Quel niveau de test ?"));
    }

    #[test]
    fn should_route_approval_matches_only_target_channel() {
        let ev = ChatStreamEvent::MemoryQueued {
            review_id: "rev-1".into(),
            subject: "user".into(),
            predicate: "likes".into(),
            object: "rouge".into(),
            source: "learning.conversation_turn".into(),
            channel: Some("telegram".into()),
        };
        let (rid, body) = should_route_approval(&ev, "telegram").unwrap();
        assert_eq!(rid, "rev-1");
        assert!(body.contains("user likes rouge"));
        assert!(should_route_approval(&ev, "discord").is_none());
    }

    #[test]
    fn preferred_approval_routes_even_from_web_or_without_channel() {
        let web = ChatStreamEvent::MemoryQueued {
            review_id: "rev-web".into(),
            subject: "user".into(),
            predicate: "prefers".into(),
            object: "telegram approvals".into(),
            source: "learning.conversation_turn".into(),
            channel: Some("web".into()),
        };
        let (rid, body) = should_route_preferred_approval(&web).unwrap();
        assert_eq!(rid, "rev-web");
        assert!(body.contains("telegram approvals"));

        let no_channel = ChatStreamEvent::MemoryQueued {
            review_id: "rev-bg".into(),
            subject: "system".into(),
            predicate: "learned".into(),
            object: "background pattern".into(),
            source: "learning.retrospective".into(),
            channel: None,
        };
        assert!(should_route_preferred_approval(&no_channel).is_some());
    }

    #[test]
    fn should_route_approval_returns_none_when_channel_absent() {
        let ev = ChatStreamEvent::MemoryQueued {
            review_id: "rev-x".into(),
            subject: "x".into(),
            predicate: "y".into(),
            object: "z".into(),
            source: "learning".into(),
            channel: None,
        };
        assert!(should_route_approval(&ev, "telegram").is_none());
    }

    #[test]
    fn should_route_approval_returns_none_for_memory_stored_event() {
        // MemoryStored is a different kind of routing: should_route handles
        // it; should_route_approval must not pick it up by mistake.
        let ev = ChatStreamEvent::MemoryStored {
            subject: "u".into(),
            predicate: "p".into(),
            object: "o".into(),
            source: "s".into(),
            wing: "learnings".into(),
            room: "general".into(),
            channel: Some("telegram".into()),
            category: None,
        };
        assert!(should_route_approval(&ev, "telegram").is_none());
    }
}
