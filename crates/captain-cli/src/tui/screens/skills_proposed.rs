//! Durable learned-workflow review shared with web and desktop.

use crate::tui::theme;
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalCardKind, ProposalCardState, ProposalIsolatedTestStatus,
    WorkflowLearningRecoveryState, WorkflowLearningRuntimeState, WorkflowLearningStatus,
    WorkflowLearningView, WorkflowProjectionStatus,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap};
use ratatui::Frame;

#[derive(Clone, Default)]
pub struct SkillsMetrics {
    pub total: u64,
    pub awaiting_decision: u64,
    pub processing: u64,
    pub active: u64,
    pub attention: u64,
}

impl SkillsMetrics {
    pub fn from_workflows(
        workflows: &[WorkflowLearningView],
        status: &WorkflowLearningStatus,
    ) -> Self {
        let mut metrics = Self {
            total: workflows.len() as u64,
            ..Self::default()
        };
        for workflow in workflows {
            let has_runtime_incident = status
                .attention
                .iter()
                .any(|item| item.proposal_id == workflow.proposal_id);
            if has_runtime_incident {
                metrics.attention += 1;
                continue;
            }
            if workflow.projection_status == WorkflowProjectionStatus::Invalid
                || matches!(
                    workflow.state,
                    ProposalCardState::Rejected
                        | ProposalCardState::InstallFailed
                        | ProposalCardState::RolledBack
                )
            {
                metrics.attention += 1;
            }
            match workflow.state {
                ProposalCardState::Proposed => metrics.awaiting_decision += 1,
                ProposalCardState::Active => metrics.active += 1,
                ProposalCardState::Observed
                | ProposalCardState::Eligible
                | ProposalCardState::Drafting
                | ProposalCardState::Validating
                | ProposalCardState::ApprovedPendingInstall
                | ProposalCardState::ActiveCanary => metrics.processing += 1,
                _ => {}
            }
        }
        metrics
    }
}

pub struct SkillsProposedState {
    pub workflows: Vec<WorkflowLearningView>,
    pub metrics: Option<SkillsMetrics>,
    pub status: Option<WorkflowLearningStatus>,
    pub list_state: ListState,
    pub show_all: bool,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum SkillsProposedAction {
    Continue,
    Refresh,
    Decide {
        proposal_id: String,
        operator_token: String,
        decision_version: u64,
        action: ProposalCardAction,
    },
    Retry {
        proposal_id: String,
        expected_error_code: String,
    },
}

impl SkillsProposedState {
    pub fn new() -> Self {
        Self {
            workflows: Vec::new(),
            metrics: None,
            status: None,
            list_state: ListState::default(),
            show_all: false,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SkillsProposedAction {
        match key.code {
            KeyCode::Char('r') => return SkillsProposedAction::Refresh,
            KeyCode::Char('e') => return self.retry(),
            KeyCode::Tab => {
                self.show_all = !self.show_all;
                self.list_state
                    .select((self.current_len() > 0).then_some(0));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.current_len();
                if len == 0 {
                    return SkillsProposedAction::Continue;
                }
                let index = self.list_state.selected().unwrap_or(0);
                self.list_state
                    .select(Some(if index == 0 { len - 1 } else { index - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.current_len();
                if len == 0 {
                    return SkillsProposedAction::Continue;
                }
                let index = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((index + 1) % len));
            }
            KeyCode::Char('a') => return self.decision(ProposalCardAction::Activate),
            KeyCode::Char('t') => return self.decision(ProposalCardAction::Test),
            KeyCode::Char('l') => return self.decision(ProposalCardAction::Later),
            KeyCode::Char('x') => return self.decision(ProposalCardAction::Ignore),
            _ => {}
        }
        SkillsProposedAction::Continue
    }

    fn current_len(&self) -> usize {
        self.visible_workflows().len()
    }

    fn visible_workflows(&self) -> Vec<&WorkflowLearningView> {
        self.workflows
            .iter()
            .filter(|workflow| {
                self.show_all
                    || workflow.state == ProposalCardState::Proposed
                    || workflow.projection_status == WorkflowProjectionStatus::Invalid
                    || self.status.as_ref().is_some_and(|status| {
                        status
                            .attention
                            .iter()
                            .any(|item| item.proposal_id == workflow.proposal_id)
                    })
            })
            .collect()
    }

    fn selected_workflow(&self) -> Option<&WorkflowLearningView> {
        let index = self.list_state.selected()?;
        self.visible_workflows().get(index).copied()
    }

    fn decision(&self, action: ProposalCardAction) -> SkillsProposedAction {
        let Some(workflow) = self.selected_workflow() else {
            return SkillsProposedAction::Continue;
        };
        let Some(card) = workflow.card.as_ref() else {
            return SkillsProposedAction::Continue;
        };
        if workflow.projection_status != WorkflowProjectionStatus::Verified
            || !card.available_actions.contains(&action)
        {
            return SkillsProposedAction::Continue;
        }
        SkillsProposedAction::Decide {
            proposal_id: workflow.proposal_id.clone(),
            operator_token: card.lookup_token.clone(),
            decision_version: card.decision_version,
            action,
        }
    }

    fn retry(&self) -> SkillsProposedAction {
        let Some(workflow) = self.selected_workflow() else {
            return SkillsProposedAction::Continue;
        };
        let Some(item) = self.status.as_ref().and_then(|status| {
            status
                .attention
                .iter()
                .find(|item| item.proposal_id == workflow.proposal_id && item.retry_available)
        }) else {
            return SkillsProposedAction::Continue;
        };
        let Some(error_code) = item.error_code.clone() else {
            return SkillsProposedAction::Continue;
        };
        SkillsProposedAction::Retry {
            proposal_id: workflow.proposal_id.clone(),
            expected_error_code: error_code,
        }
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut SkillsProposedState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Workflows appris ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(runtime_lines(state)).wrap(Wrap { trim: true }),
        chunks[0],
    );
    f.render_widget(Paragraph::new(metrics_line(state)), chunks[1]);
    let (decision_style, all_style) = if state.show_all {
        (theme::tab_inactive(), theme::tab_active())
    } else {
        (theme::tab_active(), theme::tab_inactive())
    };
    let awaiting = state
        .metrics
        .as_ref()
        .map(|metrics| metrics.awaiting_decision)
        .unwrap_or(0);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" À décider ({awaiting}) "), decision_style),
            Span::raw("  "),
            Span::styled(format!(" Tous ({}) ", state.workflows.len()), all_style),
        ])),
        chunks[2],
    );

    let body = if chunks[3].width >= 96 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(chunks[3])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(chunks[3])
    };
    let visible = state
        .visible_workflows()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let items = visible
        .iter()
        .map(|workflow| {
            let name = workflow
                .card
                .as_ref()
                .map(|card| card.name.as_str())
                .or(workflow.name.as_deref())
                .unwrap_or("Workflow en construction");
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<11} ", state_label(workflow.state)),
                    state_style(workflow),
                ),
                Span::styled(name.to_string(), Style::default().fg(theme::TEXT)),
            ]))
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT))
        .highlight_style(theme::selected_style());
    f.render_stateful_widget(list, body[0], &mut state.list_state);

    let selected = state
        .list_state
        .selected()
        .and_then(|index| visible.get(index));
    f.render_widget(
        Paragraph::new(detail_lines(selected))
            .wrap(Wrap { trim: true })
            .block(Block::default().padding(Padding::horizontal(1))),
        body[1],
    );

    let footer = if state.status_msg.is_empty() {
        "[↑↓] nav  [Tab] filtre  [a] activer  [t] tester  [e] relancer  [l] plus tard  [x] ignorer  [r] actualiser"
    } else {
        state.status_msg.as_str()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(footer, theme::hint_style()))),
        chunks[4],
    );
}

fn runtime_lines(state: &SkillsProposedState) -> Vec<Line<'static>> {
    let Some(status) = state.status.as_ref() else {
        return vec![
            Line::from(Span::styled(
                "État du moteur : chargement…",
                theme::dim_style(),
            )),
            Line::default(),
            Line::default(),
        ];
    };
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
    let model_detail = if bound_model == expected_model {
        bound_model
    } else {
        format!("{bound_model} · attendu:{expected_model}")
    };
    let worker_line = status.worker.as_ref().map_or_else(
        || "worker absent".to_string(),
        |worker| {
            let scan = worker
                .last_scan_at_unix_ms
                .map(|at| format!("scan {}", relative_age(status.generated_at_unix_ms, at)))
                .unwrap_or_else(|| "scan en attente".to_string());
            let error = worker
                .last_error_scope
                .as_deref()
                .map(|scope| format!(" · erreur:{scope}"))
                .unwrap_or_default();
            format!(
                "heartbeat {} · {scan}{error}",
                relative_age(status.generated_at_unix_ms, worker.heartbeat_at_unix_ms),
            )
        },
    );
    let mode = format!("{:?}", status.mode).to_lowercase();
    let incident = status.attention.first().map(|item| {
        format!(
            " · {}:{} ({}/{})",
            job_stage_label(item.stage),
            job_error_label(item.error_code.as_deref()),
            item.attempt_count,
            item.max_attempts
        )
    });
    vec![
        Line::from(vec![
            Span::styled(
                format!("● {}", runtime_state_label(status.state)),
                runtime_state_style(status.state),
            ),
            Span::styled(
                format!("  mode:{mode}  modèle:{model_detail}"),
                theme::dim_style(),
            ),
        ]),
        Line::from(Span::styled(worker_line, theme::dim_style())),
        Line::from(Span::styled(
            format!(
                "jobs attente:{} actifs:{} retry:{} incertains:{} bloqués:{} · reprise:{}{}",
                status.jobs.pending,
                status.jobs.running,
                status.jobs.retry_wait,
                status.jobs.uncertain,
                status.jobs.dead,
                recovery_label(status.recovery),
                incident.unwrap_or_default(),
            ),
            theme::dim_style(),
        )),
    ]
}

fn runtime_state_label(state: WorkflowLearningRuntimeState) -> &'static str {
    match state {
        WorkflowLearningRuntimeState::Disabled => "désactivé",
        WorkflowLearningRuntimeState::Starting => "démarrage",
        WorkflowLearningRuntimeState::Healthy => "opérationnel",
        WorkflowLearningRuntimeState::Active => "actif",
        WorkflowLearningRuntimeState::Recovering => "reprise automatique",
        WorkflowLearningRuntimeState::Degraded => "Learning à vérifier",
        WorkflowLearningRuntimeState::Stalled => "worker bloqué",
    }
}

fn job_stage_label(
    stage: captain_types::workflow_learning::WorkflowLearningJobStage,
) -> &'static str {
    use captain_types::workflow_learning::WorkflowLearningJobStage;
    match stage {
        WorkflowLearningJobStage::Analyze => "analyse",
        WorkflowLearningJobStage::Draft => "génération",
        WorkflowLearningJobStage::Validate => "validation",
        WorkflowLearningJobStage::Install => "installation",
        WorkflowLearningJobStage::Canary => "canary",
        WorkflowLearningJobStage::Rollback => "rollback",
    }
}

fn job_error_label(code: Option<&str>) -> &str {
    match code {
        Some("model_timeout") => "délai modèle dépassé",
        Some("model_completion_failed") => "appel modèle interrompu",
        Some("invalid_structured_output") => "réponse structurée invalide",
        Some("invalid_draft") => "brouillon invalide",
        Some("effect_interrupted") => "effet incertain",
        Some("attempts_exhausted") => "essais épuisés",
        Some(other) => other,
        None => "cause non classée",
    }
}

fn runtime_state_style(state: WorkflowLearningRuntimeState) -> Style {
    match state {
        WorkflowLearningRuntimeState::Healthy | WorkflowLearningRuntimeState::Active => {
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD)
        }
        WorkflowLearningRuntimeState::Recovering | WorkflowLearningRuntimeState::Starting => {
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD)
        }
        WorkflowLearningRuntimeState::Disabled => theme::dim_style(),
        WorkflowLearningRuntimeState::Degraded | WorkflowLearningRuntimeState::Stalled => {
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
        }
    }
}

fn recovery_label(recovery: WorkflowLearningRecoveryState) -> &'static str {
    match recovery {
        WorkflowLearningRecoveryState::Disabled => "désactivée",
        WorkflowLearningRecoveryState::Starting => "démarrage",
        WorkflowLearningRecoveryState::InSync => "synchronisée",
        WorkflowLearningRecoveryState::AutomaticRetryActive => "retry automatique",
        WorkflowLearningRecoveryState::OperatorAttention => "attention requise",
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

fn metrics_line(state: &SkillsProposedState) -> Line<'static> {
    match &state.metrics {
        Some(metrics) => Line::from(vec![
            Span::styled(
                format!("{} total  ", metrics.total),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "à décider:{}  en cours:{}  actifs:{}  attention:{}",
                    metrics.awaiting_decision,
                    metrics.processing,
                    metrics.active,
                    metrics.attention
                ),
                theme::dim_style(),
            ),
        ]),
        None if state.loading => Line::from(Span::styled("chargement…", theme::dim_style())),
        None => Line::from(Span::styled("aucun workflow", theme::dim_style())),
    }
}

fn detail_lines(workflow: Option<&WorkflowLearningView>) -> Vec<Line<'static>> {
    let Some(workflow) = workflow else {
        return vec![Line::from(Span::styled(
            "Aucun workflow dans ce filtre.",
            theme::dim_style(),
        ))];
    };
    let kind = workflow
        .kind
        .map(kind_label)
        .unwrap_or("classification en cours");
    let mut lines = vec![
        Line::from(vec![
            Span::styled("État  ", theme::dim_style()),
            Span::styled(state_label(workflow.state), state_style(workflow)),
            Span::styled(format!("  {kind}"), theme::dim_style()),
        ]),
        Line::default(),
    ];
    if let Some(card) = &workflow.card {
        lines.push(Line::from(Span::styled(
            card.name.clone(),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(card.purpose.clone()));
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("Déclencheur  ", theme::dim_style()),
            Span::raw(card.trigger.clone()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Preuves  ", theme::dim_style()),
            Span::raw(format!(
                "{} usages · {} sessions",
                card.evidence.occurrences, card.evidence.distinct_sessions
            )),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Validation  ", theme::dim_style()),
            Span::raw(format!(
                "{} contrôles · {}:{}",
                card.validation.len(),
                card.validated_by.provider,
                card.validated_by.model
            )),
        ]));
        if let Some(test) = &card.isolated_test {
            lines.push(Line::from(vec![
                Span::styled("Test isolé  ", theme::dim_style()),
                Span::raw(test_status_label(test.status)),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Captain collecte ou valide encore les preuves de ce workflow.",
            theme::dim_style(),
        )));
    }
    if let Some(installation) = &workflow.installation {
        lines.push(Line::from(vec![
            Span::styled("Installation  ", theme::dim_style()),
            Span::raw(format!(
                "{} · {}",
                installation_phase_label(installation.phase),
                installation.target_locator
            )),
        ]));
    }
    if let Some(error) = workflow
        .projection_error
        .as_deref()
        .or(workflow.last_error_message.as_deref())
    {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!("Attention : {error}"),
            Style::default().fg(theme::RED),
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        format!(
            "révision {} · {} événements",
            workflow
                .revision_sha256
                .as_deref()
                .map(|revision| &revision[..revision.len().min(12)])
                .unwrap_or("en attente"),
            workflow.timeline.len()
        ),
        theme::dim_style(),
    )));
    lines
}

fn state_style(workflow: &WorkflowLearningView) -> Style {
    if workflow.projection_status == WorkflowProjectionStatus::Invalid
        || matches!(
            workflow.state,
            ProposalCardState::Rejected | ProposalCardState::InstallFailed
        )
    {
        Style::default().fg(theme::RED)
    } else if workflow.state == ProposalCardState::Active {
        Style::default().fg(theme::GREEN)
    } else if workflow.state == ProposalCardState::Proposed {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::dim_style()
    }
}

fn state_label(state: ProposalCardState) -> &'static str {
    match state {
        ProposalCardState::Observed => "observé",
        ProposalCardState::Eligible => "éligible",
        ProposalCardState::Drafting => "génération",
        ProposalCardState::Validating => "validation",
        ProposalCardState::Proposed => "à décider",
        ProposalCardState::Dismissed => "ignoré",
        ProposalCardState::Snoozed => "reporté",
        ProposalCardState::Superseded => "remplacé",
        ProposalCardState::ApprovedPendingInstall => "installation",
        ProposalCardState::ActiveCanary => "canary",
        ProposalCardState::Active => "actif",
        ProposalCardState::Rejected => "rejeté",
        ProposalCardState::InstallFailed => "échec",
        ProposalCardState::RolledBack => "rollback",
    }
}

fn kind_label(kind: ProposalCardKind) -> &'static str {
    match kind {
        ProposalCardKind::Skill => "Skill",
        ProposalCardKind::Capspec => "CapSpec",
        ProposalCardKind::Automation => "Automation",
        ProposalCardKind::Refinement => "Amélioration",
    }
}

fn test_status_label(status: ProposalIsolatedTestStatus) -> &'static str {
    match status {
        ProposalIsolatedTestStatus::Queued => "en attente",
        ProposalIsolatedTestStatus::Passed => "réussi",
        ProposalIsolatedTestStatus::Failed => "échoué",
    }
}

fn installation_phase_label(
    phase: captain_types::workflow_learning::WorkflowInstallationViewPhase,
) -> &'static str {
    use captain_types::workflow_learning::WorkflowInstallationViewPhase;
    match phase {
        WorkflowInstallationViewPhase::Prepared => "préparée",
        WorkflowInstallationViewPhase::Promoted => "installée",
        WorkflowInstallationViewPhase::Verified => "vérifiée",
        WorkflowInstallationViewPhase::Active => "active",
        WorkflowInstallationViewPhase::RollbackPending => "rollback en cours",
        WorkflowInstallationViewPhase::RolledBack => "restaurée",
        WorkflowInstallationViewPhase::Quarantined => "quarantaine",
        WorkflowInstallationViewPhase::Failed => "échec",
    }
}

#[cfg(test)]
mod tests {
    use super::{runtime_lines, SkillsMetrics, SkillsProposedAction, SkillsProposedState};
    use captain_types::config::LearningMode;
    use captain_types::workflow_learning::{
        ProposalCardState, WorkflowLearningAttentionItem, WorkflowLearningAttentionState,
        WorkflowLearningJobQueueView, WorkflowLearningJobStage, WorkflowLearningModelIdentity,
        WorkflowLearningNotificationQueueView, WorkflowLearningRecoveryState,
        WorkflowLearningRuntimeState, WorkflowLearningStatus, WorkflowLearningView,
        WorkflowLearningWorkerPhase, WorkflowLearningWorkerView, WorkflowLearningWorkloadView,
        WorkflowProjectionStatus, WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
        WORKFLOW_LEARNING_VIEW_SCHEMA_VERSION,
    };
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn workflow(
        state: ProposalCardState,
        status: WorkflowProjectionStatus,
    ) -> WorkflowLearningView {
        WorkflowLearningView {
            schema_version: WORKFLOW_LEARNING_VIEW_SCHEMA_VERSION,
            proposal_id: "proposal".to_string(),
            decision_version: 0,
            state,
            revision_sha256: None,
            kind: None,
            name: None,
            source_agent_id: "captain".to_string(),
            origin_channel: None,
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            last_error_code: None,
            last_error_message: None,
            projection_status: status,
            projection_error: None,
            card: None,
            installation: None,
            timeline: Vec::new(),
        }
    }

    fn incident_status(retry_available: bool) -> WorkflowLearningStatus {
        WorkflowLearningStatus {
            schema_version: WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
            enabled: true,
            mode: LearningMode::Approval,
            state: WorkflowLearningRuntimeState::Degraded,
            recovery: WorkflowLearningRecoveryState::OperatorAttention,
            expected_model: WorkflowLearningModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            worker: None,
            jobs: WorkflowLearningJobQueueView {
                dead: 1,
                ..Default::default()
            },
            notifications: WorkflowLearningNotificationQueueView::default(),
            workflows: WorkflowLearningWorkloadView {
                total: 1,
                attention: 1,
                ..Default::default()
            },
            attention: vec![WorkflowLearningAttentionItem {
                proposal_id: "proposal".to_string(),
                stage: WorkflowLearningJobStage::Draft,
                state: WorkflowLearningAttentionState::Dead,
                error_code: Some("model_timeout".to_string()),
                attempt_count: 3,
                max_attempts: 3,
                retry_available,
                updated_at_unix_ms: 9_000,
            }],
            generated_at_unix_ms: 10_000,
        }
    }

    #[test]
    fn metrics_keep_processing_active_and_attention_distinct() {
        let status = WorkflowLearningStatus {
            schema_version: WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
            enabled: true,
            mode: LearningMode::Approval,
            state: WorkflowLearningRuntimeState::Healthy,
            recovery: WorkflowLearningRecoveryState::InSync,
            expected_model: WorkflowLearningModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            worker: None,
            jobs: WorkflowLearningJobQueueView::default(),
            notifications: WorkflowLearningNotificationQueueView::default(),
            workflows: WorkflowLearningWorkloadView::default(),
            attention: Vec::new(),
            generated_at_unix_ms: 10_000,
        };
        let metrics = SkillsMetrics::from_workflows(
            &[
                workflow(
                    ProposalCardState::Drafting,
                    WorkflowProjectionStatus::Building,
                ),
                workflow(
                    ProposalCardState::Proposed,
                    WorkflowProjectionStatus::Verified,
                ),
                workflow(
                    ProposalCardState::Active,
                    WorkflowProjectionStatus::Verified,
                ),
                workflow(ProposalCardState::Active, WorkflowProjectionStatus::Invalid),
            ],
            &status,
        );
        assert_eq!(metrics.total, 4);
        assert_eq!(metrics.processing, 1);
        assert_eq!(metrics.awaiting_decision, 1);
        assert_eq!(metrics.active, 2);
        assert_eq!(metrics.attention, 1);
    }

    #[test]
    fn operational_lines_show_bound_model_queues_and_recovery_without_percentage() {
        let mut state = SkillsProposedState::new();
        state.status = Some(WorkflowLearningStatus {
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
                last_progress_at_unix_ms: None,
                last_error_scope: None,
            }),
            jobs: WorkflowLearningJobQueueView {
                retry_wait: 2,
                ..Default::default()
            },
            notifications: WorkflowLearningNotificationQueueView::default(),
            workflows: WorkflowLearningWorkloadView::default(),
            attention: Vec::new(),
            generated_at_unix_ms: 10_000,
        });
        let text = runtime_lines(&state)
            .into_iter()
            .flat_map(|line| line.spans)
            .map(|span| span.content.into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("reprise automatique"));
        assert!(text.contains("codex:gpt-5.6-sol"));
        assert!(text.contains("jobs attente:0 actifs:0 retry:2"));
        assert!(!text.contains('%'));
    }

    #[test]
    fn blocked_workflow_is_attention_not_processing_and_retries_only_when_safe() {
        let workflow = workflow(
            ProposalCardState::Drafting,
            WorkflowProjectionStatus::Building,
        );
        let status = incident_status(true);
        let metrics = SkillsMetrics::from_workflows(std::slice::from_ref(&workflow), &status);
        assert_eq!(metrics.processing, 0);
        assert_eq!(metrics.attention, 1);

        let mut state = SkillsProposedState::new();
        state.workflows.push(workflow);
        state.status = Some(status);
        state.list_state.select(Some(0));
        assert!(matches!(
            state.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
            SkillsProposedAction::Retry {
                proposal_id,
                expected_error_code
            } if proposal_id == "proposal" && expected_error_code == "model_timeout"
        ));

        state.status = Some(incident_status(false));
        assert!(matches!(
            state.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
            SkillsProposedAction::Continue
        ));
    }
}
