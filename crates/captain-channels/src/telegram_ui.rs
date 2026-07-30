//! Native Rich control-plane cards for Telegram.

use captain_types::compaction::{CompactionPhase, CompactionProgress, CompactionState};
use captain_types::workflow_learning::{
    WorkflowLearningRecoveryState, WorkflowLearningRuntimeState, WorkflowLearningStatus,
};

const COMPACTION_GAUGE_WIDTH: usize = 16;

pub fn render_telegram_ask_user_prompt(question: &str, has_buttons: bool) -> String {
    let title = if has_buttons {
        "Décision requise"
    } else {
        "Question"
    };
    format!(
        "### ❓ {title}\n\n<blockquote>{}</blockquote>",
        escape_rich_text(question)
    )
}

pub fn render_telegram_ask_user_answer(question: &str, chosen: &str) -> String {
    format!(
        "### ✓ Décision enregistrée\n\n<blockquote>{}</blockquote>\n\n<b>Choix</b>\n<pre>{}</pre>",
        escape_rich_text(question),
        escape_rich_text(chosen)
    )
}

pub fn render_telegram_ask_user_expired(question: &str) -> String {
    format!(
        "### ⏱ Question expirée\n\n<blockquote>{}</blockquote>",
        escape_rich_text(question)
    )
}

pub fn render_telegram_channel_error(message: &str) -> String {
    format!(
        "### ⚠️ Captain\n\n<details open>\n<summary><b>Action interrompue</b></summary>\n\n<blockquote>{}</blockquote>\n</details>",
        escape_rich_text(message)
    )
}

pub fn render_telegram_compaction_progress(
    progress: &CompactionProgress,
    animation_tick: usize,
) -> String {
    let phase = match progress.phase {
        CompactionPhase::Preparing => "Préparation",
        CompactionPhase::Pruning => "Élagage des sorties d'outils",
        CompactionPhase::Summarizing => "Synthèse du contexte",
        CompactionPhase::Chunking => "Synthèse par lots",
        CompactionPhase::Merging => "Fusion des synthèses",
        CompactionPhase::Persisting => "Enregistrement durable",
        CompactionPhase::Completed => "Terminé",
        CompactionPhase::Failed => "Échec",
        CompactionPhase::Interrupted => "Interrompu",
    };
    let icon = match progress.state {
        CompactionState::Running => "⏳",
        CompactionState::Succeeded => "✓",
        CompactionState::Failed => "⚠️",
        CompactionState::Interrupted => "⏹",
    };
    let (gauge, measure) = match progress.state {
        CompactionState::Succeeded => (
            bounded_compaction_gauge(COMPACTION_GAUGE_WIDTH),
            "100% · terminé".to_string(),
        ),
        CompactionState::Failed | CompactionState::Interrupted => {
            (bounded_compaction_gauge(0), phase.to_lowercase())
        }
        CompactionState::Running => {
            if let Some(percent) = progress.determinate_percent() {
                let filled = usize::from(percent) * COMPACTION_GAUGE_WIDTH / 100;
                let measure = match (progress.completed_units, progress.total_units) {
                    (Some(done), Some(total)) => format!("{done}/{total} lots · {percent}%"),
                    _ => format!("{percent}%"),
                };
                (bounded_compaction_gauge(filled), measure)
            } else {
                let cursor = animation_tick % COMPACTION_GAUGE_WIDTH;
                let mut cells = vec!["·"; COMPACTION_GAUGE_WIDTH];
                cells[cursor] = "█";
                (
                    format!("[{}]", cells.join("")),
                    "progression indéterminée".to_string(),
                )
            }
        }
    };

    format!(
        "### {icon} Compactage du contexte\n\n<b>{}</b>\n<pre>{gauge}</pre>\n{}\n\n<blockquote>{}</blockquote>\n\n<small>{} messages · ~{} tokens · fenêtre {}</small>",
        escape_rich_text(phase),
        escape_rich_text(&measure),
        escape_rich_text(&progress.detail),
        progress.message_count,
        progress.estimated_tokens,
        progress.context_window_tokens,
    )
}

fn bounded_compaction_gauge(filled: usize) -> String {
    let filled = filled.min(COMPACTION_GAUGE_WIDTH);
    format!(
        "[{}{}]",
        "█".repeat(filled),
        "░".repeat(COMPACTION_GAUGE_WIDTH.saturating_sub(filled))
    )
}

pub fn render_telegram_workflow_learning_status(status: &WorkflowLearningStatus) -> String {
    let bound_model = status
        .worker
        .as_ref()
        .and_then(|worker| worker.bound_model.as_ref())
        .map(|model| format!("{}:{}", model.provider, model.model))
        .unwrap_or_else(|| "pas encore lié".to_string());
    let expected_model = format!(
        "{}:{}",
        status.expected_model.provider, status.expected_model.model
    );
    let worker = status.worker.as_ref().map_or_else(
        || "Absent — démarrage ou attention opérateur requis".to_string(),
        |worker| {
            let scan = worker
                .last_scan_at_unix_ms
                .map(|at| relative_age(status.generated_at_unix_ms, at))
                .unwrap_or_else(|| "en attente".to_string());
            format!(
                "Heartbeat {} · dernier scan {}",
                relative_age(status.generated_at_unix_ms, worker.heartbeat_at_unix_ms),
                scan
            )
        },
    );
    let error = status
        .worker
        .as_ref()
        .and_then(|worker| worker.last_error_scope.as_deref())
        .map(|scope| {
            format!(
                "\n<b>Erreur bornée</b>\n<pre>{}</pre>",
                escape_rich_text(scope)
            )
        })
        .unwrap_or_default();
    let next_retry = status.jobs.next_retry_at_unix_ms.map_or_else(
        || "aucun retry planifié".to_string(),
        |at| {
            format!(
                "prochain retry {}",
                relative_delay(status.generated_at_unix_ms, at)
            )
        },
    );

    format!(
        "### 🧠 Learning Captain\n\n> {} <b>{}</b> · mode <code>{}</code>\n\n<b>Modèle réellement lié</b>\n<pre>{}</pre>\nAttendu : <code>{}</code>\n\n<b>Worker</b>\n{}\n\n<b>Files durables</b>\n• Jobs : {} attente · {} en cours · {} retry · {} incertain · {} dead\n• Notifications : {} attente · {} livraison · {} retry · {} dead\n• Workflows : {} actifs · {} en cours · {} à décider · {} attention\n\n<b>Reprise</b>\n{} · {}{}",
        runtime_state_icon(status.state),
        runtime_state_label(status.state),
        format!("{:?}", status.mode).to_lowercase(),
        escape_rich_text(&bound_model),
        escape_rich_text(&expected_model),
        escape_rich_text(&worker),
        status.jobs.pending,
        status.jobs.running,
        status.jobs.retry_wait,
        status.jobs.uncertain,
        status.jobs.dead,
        status.notifications.pending,
        status.notifications.delivering,
        status.notifications.retry_wait,
        status.notifications.dead,
        status.workflows.active,
        status.workflows.processing,
        status.workflows.awaiting_decision,
        status.workflows.attention,
        recovery_label(status.recovery),
        next_retry,
        error,
    )
}

fn runtime_state_icon(state: WorkflowLearningRuntimeState) -> &'static str {
    match state {
        WorkflowLearningRuntimeState::Healthy | WorkflowLearningRuntimeState::Active => "🟢",
        WorkflowLearningRuntimeState::Starting | WorkflowLearningRuntimeState::Recovering => "🟡",
        WorkflowLearningRuntimeState::Degraded | WorkflowLearningRuntimeState::Stalled => "🔴",
        WorkflowLearningRuntimeState::Disabled => "⚪",
    }
}

fn runtime_state_label(state: WorkflowLearningRuntimeState) -> &'static str {
    match state {
        WorkflowLearningRuntimeState::Disabled => "Désactivé",
        WorkflowLearningRuntimeState::Starting => "Démarrage",
        WorkflowLearningRuntimeState::Healthy => "Opérationnel",
        WorkflowLearningRuntimeState::Active => "Actif",
        WorkflowLearningRuntimeState::Recovering => "Reprise automatique",
        WorkflowLearningRuntimeState::Degraded => "Dégradé",
        WorkflowLearningRuntimeState::Stalled => "Worker bloqué",
    }
}

fn recovery_label(state: WorkflowLearningRecoveryState) -> &'static str {
    match state {
        WorkflowLearningRecoveryState::Disabled => "Reprise désactivée",
        WorkflowLearningRecoveryState::Starting => "Démarrage en cours",
        WorkflowLearningRecoveryState::InSync => "Synchronisé",
        WorkflowLearningRecoveryState::AutomaticRetryActive => "Retry automatique actif",
        WorkflowLearningRecoveryState::OperatorAttention => "Attention opérateur requise",
    }
}

fn relative_age(now_unix_ms: i64, at_unix_ms: i64) -> String {
    let seconds = now_unix_ms.saturating_sub(at_unix_ms).max(0) / 1_000;
    if seconds < 60 {
        format!("il y a {seconds}s")
    } else if seconds < 3_600 {
        format!("il y a {}min", seconds / 60)
    } else {
        format!("il y a {}h", seconds / 3_600)
    }
}

fn relative_delay(now_unix_ms: i64, at_unix_ms: i64) -> String {
    let seconds = at_unix_ms.saturating_sub(now_unix_ms).max(0) / 1_000;
    if seconds < 60 {
        format!("dans {seconds}s")
    } else if seconds < 3_600 {
        format!("dans {}min", (seconds + 59) / 60)
    } else {
        format!("dans {}h", (seconds + 3_599) / 3_600)
    }
}

fn escape_rich_text(text: &str) -> String {
    html_escape::encode_text(text.trim()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::{AgentId, SessionId};
    use captain_types::compaction::{CompactionProgressUnit, COMPACTION_PROGRESS_SCHEMA_VERSION};
    use captain_types::config::LearningMode;
    use captain_types::workflow_learning::{
        WorkflowLearningJobQueueView, WorkflowLearningModelIdentity,
        WorkflowLearningNotificationQueueView, WorkflowLearningWorkerPhase,
        WorkflowLearningWorkerView, WorkflowLearningWorkloadView,
        WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
    };

    #[test]
    fn ask_user_cards_distinguish_prompt_answer_and_expiry() {
        let prompt = render_telegram_ask_user_prompt("Déployer ?", true);
        assert!(prompt.starts_with("### ❓ Décision requise"));
        assert!(prompt.contains("Déployer ?"));

        let answer = render_telegram_ask_user_answer("Déployer ?", "Oui");
        assert!(answer.starts_with("### ✓ Décision enregistrée"));
        assert!(answer.contains("<pre>Oui</pre>"));

        let expired = render_telegram_ask_user_expired("Déployer ?");
        assert!(expired.starts_with("### ⏱ Question expirée"));
    }

    fn compaction_progress(
        phase: CompactionPhase,
        state: CompactionState,
        completed_units: Option<u32>,
        total_units: Option<u32>,
    ) -> CompactionProgress {
        CompactionProgress {
            schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
            operation_id: "op-1".to_string(),
            runtime_instance_id: "runtime-1".to_string(),
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            phase,
            state,
            detail: "Exact runtime state".to_string(),
            message_count: 42,
            estimated_tokens: 12_000,
            context_window_tokens: 200_000,
            completed_units,
            total_units,
            unit: completed_units.map(|_| CompactionProgressUnit::Chunks),
            started_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    #[test]
    fn compaction_card_distinguishes_exact_and_opaque_progress() {
        let opaque = render_telegram_compaction_progress(
            &compaction_progress(
                CompactionPhase::Summarizing,
                CompactionState::Running,
                None,
                None,
            ),
            3,
        );
        assert!(opaque.contains("progression indéterminée"));
        assert!(!opaque.contains('%'));
        assert!(opaque.contains("<pre>[···█············]</pre>"));

        let exact = render_telegram_compaction_progress(
            &compaction_progress(
                CompactionPhase::Chunking,
                CompactionState::Running,
                Some(2),
                Some(5),
            ),
            0,
        );
        assert!(exact.contains("2/5 lots · 40%"));
        assert!(exact.contains("<pre>[██████░░░░░░░░░░]</pre>"));
    }

    #[test]
    fn completed_compaction_card_is_visibly_full_even_with_stale_units() {
        let completed = render_telegram_compaction_progress(
            &compaction_progress(
                CompactionPhase::Completed,
                CompactionState::Succeeded,
                Some(2),
                Some(5),
            ),
            0,
        );

        assert!(completed.contains("<pre>[████████████████]</pre>"));
        assert!(completed.contains("100% · terminé"));
        assert!(!completed.contains("2/5"));
        assert!(!completed.contains("40%"));

        let failed = render_telegram_compaction_progress(
            &compaction_progress(
                CompactionPhase::Failed,
                CompactionState::Failed,
                Some(2),
                Some(5),
            ),
            0,
        );
        assert!(failed.contains("<pre>[░░░░░░░░░░░░░░░░]</pre>"));
        assert!(!failed.contains("100%"));
    }

    #[test]
    fn control_cards_escape_model_and_provider_content() {
        let hostile = "</blockquote><script>alert(1)</script>";
        for body in [
            render_telegram_ask_user_prompt(hostile, false),
            render_telegram_ask_user_answer(hostile, hostile),
            render_telegram_ask_user_expired(hostile),
            render_telegram_channel_error(hostile),
        ] {
            assert!(!body.contains("<script>"));
            assert!(body.contains("&lt;script&gt;"));
        }
    }

    #[test]
    fn learning_status_is_rich_exact_and_never_invents_progress() {
        let status = WorkflowLearningStatus {
            schema_version: WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
            enabled: true,
            mode: LearningMode::Approval,
            state: WorkflowLearningRuntimeState::Recovering,
            recovery: WorkflowLearningRecoveryState::AutomaticRetryActive,
            expected_model: WorkflowLearningModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            worker: Some(WorkflowLearningWorkerView {
                phase: WorkflowLearningWorkerPhase::Running,
                bound_model: Some(WorkflowLearningModelIdentity {
                    provider: "codex".to_string(),
                    model: "gpt-5.6-sol".to_string(),
                }),
                started_at_unix_ms: 1_000,
                heartbeat_at_unix_ms: 9_000,
                heartbeat_age_ms: 1_000,
                last_scan_at_unix_ms: Some(8_000),
                last_progress_at_unix_ms: Some(7_000),
                last_error_scope: None,
            }),
            jobs: WorkflowLearningJobQueueView {
                retry_wait: 2,
                next_retry_at_unix_ms: Some(40_000),
                ..Default::default()
            },
            notifications: WorkflowLearningNotificationQueueView::default(),
            workflows: WorkflowLearningWorkloadView {
                active: 3,
                awaiting_decision: 1,
                ..Default::default()
            },
            generated_at_unix_ms: 10_000,
        };
        let rendered = render_telegram_workflow_learning_status(&status);
        assert!(rendered.starts_with("### 🧠 Learning Captain"));
        assert!(rendered.contains("gpt-5.6-sol"));
        assert!(rendered.contains("2 retry"));
        assert!(rendered.contains("dans 30s"));
        assert!(!rendered.contains('%'));
    }
}
