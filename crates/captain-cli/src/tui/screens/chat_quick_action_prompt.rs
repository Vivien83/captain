//! Data builder for the chat quick-action modal.

use super::approvals::ApprovalRequest;
use super::chat::{
    ChatState, ModelSwitchChoice, PendingAskUser, PendingModelSwitch, QuickActionChoiceId,
    QuickActionClickZone,
};
use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};
use ratatui::Frame;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QuickActionChoiceStyle {
    Primary,
    Secondary,
    Warning,
    Danger,
    Muted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QuickActionChoice {
    pub(super) id: QuickActionChoiceId,
    pub(super) label: String,
    pub(super) style: QuickActionChoiceStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QuickActionPrompt {
    pub(super) title: String,
    pub(super) risk: String,
    pub(super) details: Vec<(String, String, bool)>,
    pub(super) lead: String,
    pub(super) choices: Vec<QuickActionChoice>,
    pub(super) hint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelSwitchQuickActionKey {
    Choice(QuickActionChoiceId),
    InvalidAnswer,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Insert(char),
    Continue,
}

pub(super) const MODEL_SWITCH_INVALID_REPLY: &str =
    "Reponds par 'nouvelle session', 'garde le contexte', 1, 2, ou Esc.";

pub(super) fn approval_quick_action_choice_for_key(key: KeyEvent) -> Option<QuickActionChoiceId> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('o') => Some(QuickActionChoiceId::ApprovalOnce),
        KeyCode::Char('s') => Some(QuickActionChoiceId::ApprovalSession),
        KeyCode::Char('A') => Some(QuickActionChoiceId::ApprovalAlways),
        KeyCode::Char('n') | KeyCode::Char('d') | KeyCode::Esc => {
            Some(QuickActionChoiceId::ApprovalReject)
        }
        _ => None,
    }
}

/// Maps `1`..`9` to the matching option index (1-based in the label, like
/// the model switch prompt's `[1]`/`[2]`), bounded to how many options this
/// question actually has.
pub(super) fn ask_user_quick_action_choice_for_key(
    key: KeyEvent,
    n_options: usize,
) -> Option<QuickActionChoiceId> {
    match key.code {
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let idx = c.to_digit(10).unwrap_or(0) as usize;
            (idx <= n_options).then(|| QuickActionChoiceId::AskUserOption(idx - 1))
        }
        _ => None,
    }
}

pub(super) fn model_switch_quick_action_for_key(
    key: KeyEvent,
    answer: &str,
    recommended_strategy: Option<&str>,
) -> ModelSwitchQuickActionKey {
    match key.code {
        KeyCode::Esc => ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchCancel),
        KeyCode::Char('1') if answer.trim().is_empty() => {
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchNewSession)
        }
        KeyCode::Char('2') if answer.trim().is_empty() => {
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchCompactSession)
        }
        KeyCode::Enter => {
            let choice = if answer.trim().is_empty() {
                recommended_strategy.and_then(model_switch_choice_from_strategy)
            } else {
                parse_model_switch_natural_choice(answer)
            };
            choice
                .map(QuickActionChoiceId::from_model_switch)
                .map(ModelSwitchQuickActionKey::Choice)
                .unwrap_or(ModelSwitchQuickActionKey::InvalidAnswer)
        }
        KeyCode::Backspace => ModelSwitchQuickActionKey::Backspace,
        KeyCode::Delete => ModelSwitchQuickActionKey::Delete,
        KeyCode::Left => ModelSwitchQuickActionKey::Left,
        KeyCode::Right => ModelSwitchQuickActionKey::Right,
        KeyCode::Home => ModelSwitchQuickActionKey::Home,
        KeyCode::End => ModelSwitchQuickActionKey::End,
        KeyCode::Char(c) => ModelSwitchQuickActionKey::Insert(c),
        _ => ModelSwitchQuickActionKey::Continue,
    }
}

fn model_switch_choice_from_strategy(strategy: &str) -> Option<ModelSwitchChoice> {
    match strategy.trim().to_ascii_lowercase().as_str() {
        "new_session" | "new" | "reset" => Some(ModelSwitchChoice::NewSession),
        "compact_session" | "compact" | "summary" | "resume" => {
            Some(ModelSwitchChoice::CompactSession)
        }
        _ => None,
    }
}

fn parse_model_switch_natural_choice(input: &str) -> Option<ModelSwitchChoice> {
    let value = input.trim().to_lowercase();
    if value.is_empty() {
        return None;
    }

    if value.contains("annule") || value.contains("cancel") || value == "non" || value == "no" {
        return Some(ModelSwitchChoice::Cancel);
    }

    if value.contains("sans contexte")
        || value.contains("sans resume")
        || value.contains("sans résumé")
        || value.contains("no context")
        || value.contains("without context")
    {
        return Some(ModelSwitchChoice::NewSession);
    }

    if value.contains("nouvelle")
        || value.contains("nouveau")
        || value.contains("new")
        || value.contains("fresh")
        || value.contains("vide")
        || value.contains("zero")
        || value.contains("zéro")
        || value.contains("reset")
        || value.contains("repart")
    {
        return Some(ModelSwitchChoice::NewSession);
    }

    if value.contains("compact")
        || value.contains("resume")
        || value.contains("résumé")
        || value.contains("garde")
        || value.contains("contexte")
        || value.contains("continue")
        || value.contains("keep")
        || value.contains("summary")
    {
        return Some(ModelSwitchChoice::CompactSession);
    }

    None
}

pub(super) fn build_quick_action_prompt(state: &ChatState) -> Option<QuickActionPrompt> {
    if let Some(req) = state.pending_approval.as_ref() {
        return Some(build_approval_prompt(req));
    }

    if let Some(pending) = state.pending_ask_user.as_ref() {
        return Some(build_ask_user_prompt(pending));
    }

    state
        .pending_model_switch
        .as_ref()
        .map(build_model_switch_prompt)
}

fn build_ask_user_prompt(pending: &PendingAskUser) -> QuickActionPrompt {
    QuickActionPrompt {
        title: "Question".to_string(),
        risk: "info".to_string(),
        details: Vec::new(),
        lead: pending.question.clone(),
        choices: ask_user_choices(&pending.options),
        hint: "Tape un chiffre ou clique une option.".to_string(),
    }
}

fn ask_user_choices(options: &[String]) -> Vec<QuickActionChoice> {
    options
        .iter()
        .enumerate()
        .map(|(idx, option)| QuickActionChoice {
            id: QuickActionChoiceId::AskUserOption(idx),
            label: format!("[{}] {option}", idx + 1),
            style: QuickActionChoiceStyle::Primary,
        })
        .collect()
}

fn build_approval_prompt(req: &ApprovalRequest) -> QuickActionPrompt {
    QuickActionPrompt {
        title: "Approbation requise".to_string(),
        risk: req.risk_level.clone(),
        details: approval_details(req),
        lead: "Choisis la portee de cette autorisation:".to_string(),
        choices: approval_choices(),
        hint: "Legacy: y approuve une fois, d refuse.".to_string(),
    }
}

fn approval_details(req: &ApprovalRequest) -> Vec<(String, String, bool)> {
    let mut details = vec![
        ("agent".to_string(), req.agent_name.clone(), false),
        ("tool".to_string(), req.tool_name.clone(), true),
        ("action".to_string(), req.action.clone(), false),
    ];
    if !req.description.trim().is_empty() && req.description.trim() != req.action.trim() {
        details.push(("detail".to_string(), req.description.clone(), false));
    }
    details
}

fn approval_choices() -> Vec<QuickActionChoice> {
    vec![
        QuickActionChoice {
            id: QuickActionChoiceId::ApprovalOnce,
            label: "[o] Une fois".to_string(),
            style: QuickActionChoiceStyle::Primary,
        },
        QuickActionChoice {
            id: QuickActionChoiceId::ApprovalSession,
            label: "[s] Session".to_string(),
            style: QuickActionChoiceStyle::Secondary,
        },
        QuickActionChoice {
            id: QuickActionChoiceId::ApprovalAlways,
            label: "[A] Toujours".to_string(),
            style: QuickActionChoiceStyle::Warning,
        },
        QuickActionChoice {
            id: QuickActionChoiceId::ApprovalReject,
            label: "[n/Esc] Refuser".to_string(),
            style: QuickActionChoiceStyle::Danger,
        },
    ]
}

fn build_model_switch_prompt(prompt: &PendingModelSwitch) -> QuickActionPrompt {
    QuickActionPrompt {
        title: "Changement de modele".to_string(),
        risk: prompt.risk.clone(),
        details: model_switch_details(prompt),
        lead: "Choisis comment demarrer la prochaine session:".to_string(),
        choices: model_switch_choices(prompt.recommended_session_strategy.as_str()),
        hint: "Tu peux aussi ecrire: 'nouvelle session' ou 'garde le contexte'.".to_string(),
    }
}

fn model_switch_details(prompt: &PendingModelSwitch) -> Vec<(String, String, bool)> {
    vec![
        (
            "actuel".to_string(),
            format!("{}/{}", prompt.current_provider, prompt.current_model),
            false,
        ),
        (
            "cible".to_string(),
            format!("{}/{}", prompt.target_provider, prompt.target_model),
            true,
        ),
        (
            "contexte".to_string(),
            model_switch_context_label(prompt),
            false,
        ),
    ]
}

fn model_switch_context_label(prompt: &PendingModelSwitch) -> String {
    if prompt.active_message_count == 0 && !prompt.canonical_summary_present {
        return "aucun contexte actif".to_string();
    }

    format!(
        "{} messages actifs{}",
        prompt.active_message_count,
        if prompt.canonical_summary_present {
            " + resume canonique"
        } else {
            ""
        }
    )
}

fn model_switch_choices(recommended: &str) -> Vec<QuickActionChoice> {
    vec![
        QuickActionChoice {
            id: QuickActionChoiceId::ModelSwitchNewSession,
            label: format!(
                "[1] Nouvelle session{}",
                recommended_suffix(recommended, "new_session")
            ),
            style: QuickActionChoiceStyle::Primary,
        },
        QuickActionChoice {
            id: QuickActionChoiceId::ModelSwitchCompactSession,
            label: format!(
                "[2] Resume compact{}",
                recommended_suffix(recommended, "compact_session")
            ),
            style: QuickActionChoiceStyle::Secondary,
        },
        QuickActionChoice {
            id: QuickActionChoiceId::ModelSwitchCancel,
            label: "[Esc] Annuler".to_string(),
            style: QuickActionChoiceStyle::Muted,
        },
    ]
}

fn recommended_suffix(recommended: &str, strategy: &str) -> &'static str {
    if recommended == strategy {
        "  recommande"
    } else {
        ""
    }
}

pub(super) fn draw_quick_action_prompt(f: &mut Frame, area: Rect, state: &mut ChatState) {
    let Some(prompt) = build_quick_action_prompt(state) else {
        return;
    };
    let Some(modal) = quick_action_modal_rect(area, &prompt) else {
        return;
    };

    f.render_widget(Clear, modal);
    let block = quick_action_block(&prompt);
    let inner_modal = block.inner(modal);
    f.render_widget(block, modal);
    let lines =
        quick_action_prompt_lines(&prompt, &mut state.quick_action_click_zones, inner_modal);
    f.render_widget(Paragraph::new(lines), inner_modal);
}

fn quick_action_modal_rect(area: Rect, prompt: &QuickActionPrompt) -> Option<Rect> {
    if area.width < 48 || area.height < 8 {
        return None;
    }

    let width = area.width.saturating_sub(2).clamp(48, 86);
    let max_height = area.height.saturating_sub(1);
    let needed_height = prompt.details.len() as u16 + 7;
    let height = needed_height.clamp(8, max_height);
    Some(Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    })
}

fn quick_action_block(prompt: &QuickActionPrompt) -> Block<'static> {
    Block::default()
        .title(Line::from(vec![
            Span::styled(format!(" {} ", prompt.title), theme::title_style()),
            Span::styled(
                format!(" [{}]", prompt.risk),
                quick_action_risk_style(&prompt.risk),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1))
}

fn quick_action_prompt_lines(
    prompt: &QuickActionPrompt,
    zones: &mut Vec<QuickActionClickZone>,
    inner_modal: Rect,
) -> Vec<Line<'static>> {
    let mut lines = quick_action_detail_lines(&prompt.details);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        prompt.lead.clone(),
        Style::default()
            .fg(theme::TEXT_PRIMARY)
            .add_modifier(Modifier::BOLD),
    )]));
    push_quick_action_choice_lines(&mut lines, zones, &prompt.choices, inner_modal);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        prompt.hint.clone(),
        theme::hint_style(),
    )]));
    truncate_quick_action_lines(&mut lines, inner_modal.height);
    lines
}

fn quick_action_detail_lines(details: &[(String, String, bool)]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (label, value, highlight) in details {
        lines.push(Line::from(vec![
            Span::styled(format!("{label:<8} "), theme::dim_style()),
            Span::styled(value.clone(), quick_action_detail_style(*highlight)),
        ]));
    }
    lines
}

fn quick_action_detail_style(highlight: bool) -> Style {
    if highlight {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::TEXT_PRIMARY)
    }
}

fn truncate_quick_action_lines(lines: &mut Vec<Line<'static>>, height: u16) {
    let max_lines = height as usize;
    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }
}

fn push_quick_action_choice_lines(
    lines: &mut Vec<Line<'static>>,
    zones: &mut Vec<QuickActionClickZone>,
    choices: &[QuickActionChoice],
    inner: Rect,
) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cursor = 0u16;
    let mut y = inner.y.saturating_add(lines.len() as u16);

    for choice in choices {
        let width = choice.label.chars().count() as u16;
        if width == 0 {
            continue;
        }

        let gap = if cursor == 0 { 0 } else { 2 };
        if cursor > 0 && cursor.saturating_add(gap).saturating_add(width) > inner.width {
            lines.push(Line::from(spans));
            spans = Vec::new();
            cursor = 0;
            y = y.saturating_add(1);
        } else if gap > 0 {
            spans.push(Span::raw("  "));
            cursor = cursor.saturating_add(gap);
        }

        let x = inner.x.saturating_add(cursor);
        spans.push(Span::styled(
            choice.label.clone(),
            quick_action_choice_style(choice.style),
        ));
        if y < inner.y.saturating_add(inner.height) {
            zones.push(QuickActionClickZone {
                x_start: x,
                x_end: x.saturating_add(width.saturating_sub(1)),
                y,
                choice: choice.id,
            });
        }
        cursor = cursor.saturating_add(width);
    }

    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
}

fn quick_action_risk_style(risk: &str) -> Style {
    match risk.trim().to_ascii_lowercase().as_str() {
        "critical" | "high" => Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        "medium" => Style::default().fg(theme::YELLOW),
        _ => Style::default().fg(theme::GREEN),
    }
}

fn quick_action_choice_style(style: QuickActionChoiceStyle) -> Style {
    let fg = match style {
        QuickActionChoiceStyle::Primary => theme::GREEN,
        QuickActionChoiceStyle::Secondary => theme::CYAN,
        QuickActionChoiceStyle::Warning => theme::YELLOW,
        QuickActionChoiceStyle::Danger => theme::RED,
        QuickActionChoiceStyle::Muted => theme::TEXT_TERTIARY,
    };
    Style::default().fg(fg).add_modifier(Modifier::BOLD)
}
