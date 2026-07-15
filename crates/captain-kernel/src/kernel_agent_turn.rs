use crate::error::{KernelError, KernelResult};
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{AgentId, SessionId};
use captain_types::message::{ContentBlock, ReplyDirectives, TokenUsage};
use std::sync::Arc;

use super::kernel_project_prompt::resolve_recent_projects;
use super::CaptainKernel;
use crate::capability_routing::ensure_active_model_supports;

impl CaptainKernel {
    /// Full message send with all parameters including channel_type.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_message_full(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        content_blocks: Option<Vec<ContentBlock>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
        channel_type: Option<String>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full_in_session(
            agent_id,
            message,
            kernel_handle,
            content_blocks,
            sender_id,
            sender_name,
            channel_type,
            None,
        )
        .await
    }

    /// Send a message against an explicitly selected persisted session without
    /// changing the agent's global active session.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_message_full_in_session(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        content_blocks: Option<Vec<ContentBlock>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
        channel_type: Option<String>,
        session_id: Option<SessionId>,
    ) -> KernelResult<AgentLoopResult> {
        // Acquire per-agent lock to serialize concurrent messages for the same agent.
        // This prevents session corruption when multiple messages arrive in quick
        // succession (e.g. rapid voice messages via Telegram). Messages for different
        // agents are not blocked — each agent has its own independent lock.
        let lock = self
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        let entry = self.resolve_agent_session_entry(agent_id, session_id)?;

        {
            let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
            ensure_active_model_supports(
                &catalog,
                &entry.manifest.model.provider,
                &entry.manifest.model.model,
                content_blocks.as_deref(),
            )
            .map_err(KernelError::Captain)?;
        }

        // Mark activity at turn START: a long turn (LLM + tools, often 90s+)
        // must not read as inactivity to the 60s heartbeat timeout.
        let _ = self.registry.touch(agent_id);

        if let Some(result) =
            self.maybe_handle_first_use_onboarding(&entry, message, channel_type.as_deref())?
        {
            return Ok(result);
        }

        // Enforce quota before running the agent loop.
        self.scheduler
            .check_quota(agent_id)
            .map_err(KernelError::Captain)?;

        let regex_hint = self.emit_user_message_learning_hint(agent_id, message);

        // Channel-neutral continuation for safe model switches. The TUI keeps
        // this state in its UI layer; Telegram arrives as a fresh user message,
        // so the kernel consumes clear replies like "Nouvelle" before invoking
        // the LLM.
        let result = if let Some(response) =
            self.consume_codex_model_update_keep_request(agent_id, message)?
        {
            Ok(Self::empty_agent_loop_result(response))
        } else if let Some(result) = self.consume_pending_model_switch_choice(agent_id, message)? {
            Ok(result)
        } else if let Some(result) = self.maybe_answer_recent_project_status(message) {
            Ok(result)
        } else if let Some(result) = self.handle_direct_model_switch_request(agent_id, message)? {
            Ok(result)
        } else if entry.manifest.module.starts_with("wasm:") {
            self.execute_wasm_agent(&entry, message, kernel_handle)
                .await
        } else if entry.manifest.module.starts_with("python:") {
            self.execute_python_agent(&entry, agent_id, message).await
        } else {
            // Default: LLM agent loop (builtin:chat or any unrecognized module).
            self.execute_llm_agent(
                &entry,
                agent_id,
                message,
                kernel_handle,
                content_blocks,
                sender_id,
                sender_name,
                channel_type.clone(),
            )
            .await
        };

        match result {
            Ok(result) => {
                self.record_agent_turn_success(
                    agent_id,
                    &entry,
                    message,
                    channel_type.clone(),
                    regex_hint,
                    &result,
                );
                Ok(result)
            }
            Err(error) => {
                self.record_agent_turn_failure(agent_id, &entry, &error);
                Err(error)
            }
        }
    }

    fn maybe_answer_recent_project_status(&self, message: &str) -> Option<AgentLoopResult> {
        if !looks_like_project_status_question(message) {
            return None;
        }
        let projects = resolve_recent_projects(&self.memory, None);
        let project = projects
            .iter()
            .filter_map(|project| {
                let score =
                    project_reference_score(message, &project.slug, &project.name, &project.goal);
                (score >= 2).then_some((score, project))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, project)| project)?;

        let mut response = format!(
            "{} (`{}`) est bien enregistré dans Projects.\nÉtat durable: `{}` ; runtime: `{}/{}` ; progression: {}%.",
            project.name,
            project.slug,
            project.status,
            project.runtime_status,
            project.runtime_phase,
            project.progress.min(100)
        );
        if !project.goal.trim().is_empty() {
            response.push_str(&format!("\nObjectif: {}", project.goal));
        }
        if let Some(action) = project.next_actions.first() {
            response.push_str(&format!("\nProchaine action: `{action}`."));
        }

        Some(AgentLoopResult {
            response,
            total_usage: TokenUsage::default(),
            iterations: 0,
            cost_usd: Some(0.0),
            silent: false,
            directives: ReplyDirectives::default(),
            tool_calls: Vec::new(),
        })
    }
}

fn looks_like_project_status_question(message: &str) -> bool {
    let lower = message.to_lowercase();
    let mentions_project =
        lower.contains("projet") || lower.contains("project") || lower.contains("repo");
    let asks_status = lower.contains("où en est")
        || lower.contains("ou en est")
        || lower.contains("avancement")
        || lower.contains("statut")
        || lower.contains("status")
        || lower.contains("état")
        || lower.contains("etat")
        || lower.contains("point");
    mentions_project && asks_status
}

fn project_reference_score(message: &str, slug: &str, name: &str, goal: &str) -> usize {
    let haystack = format!("{} {} {}", slug, name, goal).to_lowercase();
    let slug = slug.to_lowercase();
    let terms = project_reference_terms(message);
    let matched_terms = terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count();
    let slug_alias_bonus = terms.iter().any(|term| {
        slug == *term
            || slug
                .split(['-', '_'])
                .any(|slug_part| slug_part.len() >= 3 && slug_part == term)
    }) as usize
        * 2;
    matched_terms + slug_alias_bonus
}

fn project_reference_terms(message: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "avec", "dans", "de", "des", "doc", "docs", "donc", "en", "est", "for", "le", "les", "max",
        "nouveau", "nouvelle", "of", "ou", "où", "pour", "projet", "project", "réponds", "sans",
        "the", "un", "une", "where",
    ];
    message
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3 && !STOPWORDS.contains(&term.as_str()))
        .collect()
}

#[cfg(test)]
mod recent_project_status_tests {
    use super::*;

    #[test]
    fn project_status_detector_matches_user_wording() {
        assert!(looks_like_project_status_question(
            "où en est le projet de gestion de doc pour couple ?"
        ));
        assert!(!looks_like_project_status_question(
            "crée une app de documents couple"
        ));
    }

    #[test]
    fn project_reference_score_matches_partial_topic_terms() {
        let score = project_reference_score(
            "où en est le projet de gestion de doc pour couple ?",
            "projet1-documents-couple",
            "Projet1 — Gestion documents couple",
            "Développer une application locale de gestion documentaire du couple.",
        );

        assert!(score >= 2);
    }

    #[test]
    fn project_reference_score_matches_project_slug_alias() {
        let score = project_reference_score(
            "où en est projet1 ?",
            "projet1-documents-couple",
            "Projet1 — Gestion documents couple",
            "Développer une application locale de gestion documentaire du couple.",
        );

        assert!(score >= 2);
    }
}
