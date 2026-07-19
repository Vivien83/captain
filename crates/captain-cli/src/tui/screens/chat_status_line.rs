//! Title/status line rendering for the chat screen.

use super::chat::ChatState;
use crate::tui::{
    provider_quota::{ProviderQuota, ProviderQuotaStatus, ProviderQuotaWindow},
    theme,
};
use chrono::{DateTime, Datelike, FixedOffset, Local, Utc, Weekday};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[cfg(test)]
mod tests;

/// Build the bottom title status line:
///   [spinner] model | mode | duration | tokens | cost
pub(super) fn build_status_line(state: &ChatState) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];

    push_activity_spinner(&mut spans, state);
    push_background_activity(&mut spans, state);
    push_model_and_mode(&mut spans, state);
    push_session_duration(&mut spans, state);
    push_last_tokens(&mut spans, state);
    push_last_cost(&mut spans, state);
    push_session_totals(&mut spans, state);

    spans.push(Span::raw(" "));
    Line::from(spans)
}

const MAX_PROVIDER_WINDOWS: usize = 8;
const MAX_PROVIDER_STATUS_ROWS: usize = 4;
const PROVIDER_STALE_AFTER_SECONDS: i64 = 15 * 60;

struct ProviderStatusItem {
    spans: Vec<Span<'static>>,
    width: usize,
}

impl ProviderStatusItem {
    fn new(spans: Vec<Span<'static>>) -> Self {
        let width = status_spans_width(&spans);
        Self { spans, width }
    }
}

/// Build the responsive provider-subscription band shown at the bottom of
/// every chat TUI. The data has already been observed and persisted by the
/// daemon; this renderer never infers an allowance.
pub(super) fn build_provider_quota_lines(state: &ChatState, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let status = &state.provider_quota_status;
    if !status.has_observation() {
        if !active_model_is_codex(&state.model_label) {
            return Vec::new();
        }
        return pack_provider_status_items(
            vec![ProviderStatusItem::new(vec![
                Span::styled(" Codex ", theme::title_style()),
                Span::styled(
                    "quota abonnement non observé",
                    Style::default().fg(theme::YELLOW),
                ),
            ])],
            width,
        );
    }

    let active_provider = active_model_provider(&state.model_label);
    let provider_quotas = status
        .quotas
        .iter()
        .filter(|quota| {
            active_provider.is_empty() || provider_ids_match(&quota.provider, active_provider)
        })
        .collect::<Vec<_>>();
    if provider_quotas.is_empty() {
        if !active_model_is_codex(&state.model_label) {
            return Vec::new();
        }
        return pack_provider_status_items(
            vec![ProviderStatusItem::new(vec![
                Span::styled(" Codex ", theme::title_style()),
                Span::styled(
                    "quota abonnement non observé",
                    Style::default().fg(theme::YELLOW),
                ),
            ])],
            width,
        );
    }

    let (applicable_quotas, alternative_quotas): (Vec<_>, Vec<_>) = provider_quotas
        .into_iter()
        .partition(|quota| provider_quota_applies_to_active_model(quota, &state.model_label));

    let now = Local::now().fixed_offset();
    let mut items = vec![provider_meta_item(
        status,
        &applicable_quotas,
        &alternative_quotas,
        &state.model_label,
    )];
    let total_windows = applicable_quotas
        .iter()
        .map(|quota| usize::from(quota.primary.is_some()) + usize::from(quota.secondary.is_some()))
        .sum::<usize>();
    let mut rendered_windows = 0usize;

    for quota in applicable_quotas {
        let stale = provider_quota_is_stale(quota);
        for (fallback_name, window) in [
            ("court", quota.primary.as_ref()),
            ("long", quota.secondary.as_ref()),
        ] {
            let Some(window) = window else {
                continue;
            };
            if rendered_windows >= MAX_PROVIDER_WINDOWS {
                break;
            }
            items.push(provider_window_item(
                quota,
                window,
                fallback_name,
                stale,
                now,
                width,
            ));
            rendered_windows += 1;
        }
        if quota.primary.is_none() && quota.secondary.is_none() {
            items.push(provider_empty_limit_item(quota, stale));
        }
    }

    if total_windows > rendered_windows {
        items.push(ProviderStatusItem::new(vec![Span::styled(
            format!(
                "+{} fenêtres applicables dans Budget",
                total_windows - rendered_windows
            ),
            theme::dim_style(),
        )]));
    }
    if !alternative_quotas.is_empty() {
        items.push(provider_alternative_item(
            &alternative_quotas,
            !active_model_name(&state.model_label).is_empty(),
        ));
    }

    pack_provider_status_items(items, width)
}

pub(super) fn draw_provider_quota_status(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    if area.height == 0 || lines.is_empty() {
        return;
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::BG_CODE)),
        area,
    );
}

fn provider_meta_item(
    status: &ProviderQuotaStatus,
    applicable_quotas: &[&ProviderQuota],
    alternative_quotas: &[&ProviderQuota],
    model_label: &str,
) -> ProviderStatusItem {
    let first = applicable_quotas
        .first()
        .copied()
        .or_else(|| alternative_quotas.first().copied());
    let provider = first
        .map(|quota| provider_display_name(&quota.provider))
        .unwrap_or_else(|| "Provider".to_string());
    let active_model = active_model_name(model_label);
    let mut spans = if active_model.is_empty() {
        vec![Span::styled(format!(" {provider}"), theme::title_style())]
    } else {
        vec![
            Span::styled(" Actif ", theme::dim_style()),
            Span::styled(active_model.to_string(), theme::title_style()),
            Span::styled(format!(" · {provider}"), theme::dim_style()),
        ]
    };
    if let Some(plan) = first.and_then(|quota| quota.plan_type.as_deref()) {
        spans.push(Span::styled(format!(" [{plan}]"), theme::dim_style()));
    }
    if let Some(credits) = applicable_quotas
        .iter()
        .chain(alternative_quotas.iter())
        .find_map(|quota| quota.credits.as_ref())
    {
        let (label, style) = if credits.unlimited {
            ("crédits ∞".to_string(), Style::default().fg(theme::GREEN))
        } else if let Some(balance) = credits.balance.as_deref() {
            (
                format!("crédits {balance}"),
                Style::default().fg(if credits.has_credits {
                    theme::GREEN
                } else {
                    theme::RED
                }),
            )
        } else if credits.has_credits {
            ("crédits disponibles".to_string(), theme::dim_style())
        } else {
            (
                "crédits épuisés".to_string(),
                Style::default().fg(theme::RED),
            )
        };
        spans.push(Span::styled(" · ", Style::default().fg(theme::BORDER)));
        spans.push(Span::styled(label, style));
    }
    if status.state == "stale" {
        spans.push(Span::styled(" · stale", Style::default().fg(theme::YELLOW)));
    }
    ProviderStatusItem::new(spans)
}

fn provider_alternative_item(
    quotas: &[&ProviderQuota],
    has_active_model: bool,
) -> ProviderStatusItem {
    let critical = quotas.iter().any(|quota| {
        quota.rate_limit_reached_type.is_some()
            || matches!(quota.alert_level.as_str(), "critical" | "exhausted")
            || provider_quota_max_percent(quota) >= 90.0
    });
    let warning = critical
        || quotas.iter().any(|quota| {
            quota.alert_level == "warning" || provider_quota_max_percent(quota) >= 70.0
        });
    let count = quotas.len();
    let noun = if count == 1 {
        "quota annexe"
    } else {
        "quotas annexes"
    };
    let pressure = if critical {
        " critique"
    } else if warning {
        " sous tension"
    } else {
        ""
    };
    let scope = if has_active_model {
        " · hors modèle actif"
    } else {
        ""
    };
    let style = if critical {
        Style::default().fg(theme::RED)
    } else if warning {
        Style::default().fg(theme::YELLOW)
    } else {
        theme::dim_style()
    };
    ProviderStatusItem::new(vec![Span::styled(
        format!("+{count} {noun}{pressure}{scope} · Budget"),
        style,
    )])
}

fn provider_quota_max_percent(quota: &ProviderQuota) -> f64 {
    quota
        .primary
        .iter()
        .chain(quota.secondary.iter())
        .map(|window| window.used_percent)
        .fold(0.0_f64, f64::max)
}

fn provider_window_item(
    quota: &ProviderQuota,
    window: &ProviderQuotaWindow,
    fallback_name: &str,
    stale: bool,
    now: DateTime<FixedOffset>,
    width: usize,
) -> ProviderStatusItem {
    let compact = width < 80;
    let gauge_cells = if width >= 120 {
        8
    } else if width >= 72 {
        6
    } else {
        4
    };
    let label_width = if width >= 120 {
        20
    } else if width >= 72 {
        14
    } else {
        8
    };
    let family = truncate_display_width(&quota.limit_name, label_width);
    let duration = window
        .window_seconds
        .map(provider_duration_label)
        .unwrap_or_else(|| fallback_name.to_string());
    let percent = window.used_percent.clamp(0.0, 100.0);
    let filled = if percent <= 0.0 {
        0
    } else {
        ((percent / 100.0) * gauge_cells as f64).ceil() as usize
    }
    .min(gauge_cells);
    let style = provider_pressure_style(percent, stale);
    let percent_label = if (window.used_percent.fract()).abs() < 0.05 {
        format!("{:.0}%", window.used_percent)
    } else {
        format!("{:.1}%", window.used_percent)
    };
    let resume = provider_resume_label(window, now, compact);

    let mut spans = vec![
        Span::styled(format!("{family} {duration} "), theme::dim_style()),
        Span::styled("[", Style::default().fg(theme::BORDER)),
        Span::styled("█".repeat(filled), style),
        Span::styled(
            "░".repeat(gauge_cells.saturating_sub(filled)),
            Style::default().fg(theme::BORDER),
        ),
        Span::styled("] ", Style::default().fg(theme::BORDER)),
        Span::styled(percent_label, style),
        Span::styled(format!(" ↻ {resume}"), theme::dim_style()),
    ];
    if stale {
        spans.push(Span::styled(" stale", Style::default().fg(theme::YELLOW)));
    }
    if quota.rate_limit_reached_type.is_some() || quota.alert_level == "exhausted" {
        spans.push(Span::styled(" bloqué", Style::default().fg(theme::RED)));
    }
    ProviderStatusItem::new(spans)
}

fn provider_empty_limit_item(quota: &ProviderQuota, stale: bool) -> ProviderStatusItem {
    let mut spans = vec![
        Span::styled(
            truncate_display_width(&quota.limit_name, 24),
            theme::dim_style(),
        ),
        Span::styled(" fenêtre non fournie", Style::default().fg(theme::YELLOW)),
    ];
    if stale {
        spans.push(Span::styled(" stale", Style::default().fg(theme::YELLOW)));
    }
    ProviderStatusItem::new(spans)
}

fn pack_provider_status_items(items: Vec<ProviderStatusItem>, width: usize) -> Vec<Line<'static>> {
    let separator = " │ ";
    let separator_width = UnicodeWidthStr::width(separator);
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0usize;

    for item in items {
        let item = truncate_provider_status_item(item, width);
        let needed = item.width + usize::from(!current.is_empty()) * separator_width;
        if !current.is_empty() && current_width + needed > width {
            lines.push(Line::from(std::mem::take(&mut current)));
            current_width = 0;
        }
        if !current.is_empty() {
            current.push(Span::styled(separator, Style::default().fg(theme::BORDER)));
            current_width += separator_width;
        }
        current_width += item.width;
        current.extend(item.spans);
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }

    if lines.len() > MAX_PROVIDER_STATUS_ROWS {
        lines.truncate(MAX_PROVIDER_STATUS_ROWS - 1);
        lines.push(Line::from(Span::styled(
            truncate_display_width(" autres limites disponibles dans Budget", width),
            theme::dim_style(),
        )));
    }
    lines
}

fn truncate_provider_status_item(item: ProviderStatusItem, max_width: usize) -> ProviderStatusItem {
    if item.width <= max_width {
        return item;
    }
    if max_width == 0 {
        return ProviderStatusItem::new(Vec::new());
    }

    let content_width = max_width.saturating_sub(1);
    let mut remaining = content_width;
    let mut spans = Vec::new();
    'outer: for span in item.spans {
        let mut content = String::new();
        for character in span.content.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if character_width > remaining {
                if !content.is_empty() {
                    spans.push(Span::styled(content, span.style));
                }
                break 'outer;
            }
            content.push(character);
            remaining = remaining.saturating_sub(character_width);
        }
        if !content.is_empty() {
            spans.push(Span::styled(content, span.style));
        }
        if remaining == 0 {
            break;
        }
    }
    spans.push(Span::styled("…", theme::dim_style()));
    ProviderStatusItem::new(spans)
}

fn provider_resume_label(
    window: &ProviderQuotaWindow,
    now: DateTime<FixedOffset>,
    compact: bool,
) -> String {
    if let Some(reset) = window.resets_at {
        let reset = reset.with_timezone(now.offset());
        let days = reset
            .date_naive()
            .signed_duration_since(now.date_naive())
            .num_days();
        let time = reset.format("%H:%M");
        return match days {
            i64::MIN..=-1 => format!("{}", reset.format("%d/%m %H:%M")),
            0 => time.to_string(),
            1 if compact => format!("J+1 {time}"),
            1 => format!("demain {time}"),
            2..=6 => format!("{} {time}", french_weekday(reset.weekday())),
            _ => format!("{}", reset.format("%d/%m %H:%M")),
        };
    }
    window
        .reset_after_seconds
        .map(|seconds| format!("~{}", provider_duration_label(seconds)))
        .unwrap_or_else(|| "inconnue".to_string())
}

fn provider_duration_label(seconds: u64) -> String {
    if seconds != 0 && seconds % 604_800 == 0 {
        format!("{}sem", seconds / 604_800)
    } else if seconds != 0 && seconds % 86_400 == 0 {
        format!("{}j", seconds / 86_400)
    } else if seconds != 0 && seconds % 3_600 == 0 {
        format!("{}h", seconds / 3_600)
    } else if seconds != 0 && seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn french_weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "lun",
        Weekday::Tue => "mar",
        Weekday::Wed => "mer",
        Weekday::Thu => "jeu",
        Weekday::Fri => "ven",
        Weekday::Sat => "sam",
        Weekday::Sun => "dim",
    }
}

fn provider_pressure_style(percent: f64, stale: bool) -> Style {
    let color = if percent >= 90.0 {
        theme::RED
    } else if percent >= 70.0 || stale {
        theme::YELLOW
    } else {
        theme::GREEN
    };
    Style::default().fg(color)
}

fn provider_quota_is_stale(quota: &ProviderQuota) -> bool {
    quota.stale
        || quota.observed_at.is_some_and(|observed_at| {
            Utc::now().signed_duration_since(observed_at).num_seconds()
                > PROVIDER_STALE_AFTER_SECONDS
        })
}

fn provider_display_name(value: &str) -> String {
    if value.eq_ignore_ascii_case("codex") || value.eq_ignore_ascii_case("openai-codex") {
        "Codex".to_string()
    } else {
        value.to_string()
    }
}

fn active_model_is_codex(model_label: &str) -> bool {
    canonical_provider_id(active_model_provider(model_label)) == "codex"
}

fn active_model_provider(model_label: &str) -> &str {
    model_label
        .split_once('/')
        .map(|(provider, _)| provider)
        .unwrap_or(model_label)
        .trim()
}

fn active_model_name(model_label: &str) -> &str {
    model_label
        .split_once('/')
        .map(|(_, model)| model)
        .unwrap_or("")
        .trim()
}

fn provider_quota_applies_to_active_model(quota: &ProviderQuota, model_label: &str) -> bool {
    if provider_quota_is_general(quota) {
        return true;
    }
    let active_model = normalized_model_identifier(active_model_name(model_label));
    !active_model.is_empty()
        && [quota.limit_id.as_str(), quota.limit_name.as_str()]
            .iter()
            .any(|candidate| normalized_model_identifier(candidate) == active_model)
}

fn provider_quota_is_general(quota: &ProviderQuota) -> bool {
    let limit_id = canonical_provider_id(&quota.limit_id);
    let provider = canonical_provider_id(&quota.provider);
    let limit_name = normalized_model_identifier(&quota.limit_name);
    let provider_name = normalized_model_identifier(&provider_display_name(&quota.provider));
    (!limit_id.is_empty() && !provider.is_empty() && limit_id == provider)
        || (!limit_name.is_empty() && !provider_name.is_empty() && limit_name == provider_name)
}

fn provider_ids_match(left: &str, right: &str) -> bool {
    canonical_provider_id(left) == canonical_provider_id(right)
}

fn canonical_provider_id(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai-codex" => "codex".to_string(),
        other => other.to_string(),
    }
}

fn normalized_model_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn truncate_display_width(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }
    let content_width = max_width.saturating_sub(1);
    let mut width = 0usize;
    let mut output = String::new();
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > content_width {
            break;
        }
        output.push(character);
        width += character_width;
    }
    output.push('…');
    output
}

fn status_spans_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn separator_span() -> Span<'static> {
    Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER))
}

fn push_separator(spans: &mut Vec<Span<'static>>) {
    spans.push(separator_span());
}

fn push_activity_spinner(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if state.is_streaming || state.thinking {
        let frame = theme::SPINNER_FRAMES[state.spinner_frame % theme::SPINNER_FRAMES.len()];
        spans.push(Span::styled(
            format!("{frame} "),
            Style::default().fg(theme::YELLOW),
        ));
    }
}

/// Persistent badge for sub-agents/detached tool_runs still in flight —
/// unlike `push_activity_spinner`, this isn't scoped to the current HTTP
/// turn and stays visible even after the turn that started the background
/// work has ended.
fn push_background_activity(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if state.background_activity.is_empty() {
        return;
    }
    spans.push(Span::styled(
        background_activity_label(state.background_activity.len()),
        Style::default().fg(theme::YELLOW),
    ));
}

fn background_activity_label(count: usize) -> String {
    if count == 1 {
        "\u{23f3} 1 en arrière-plan  ".to_string()
    } else {
        format!("\u{23f3} {count} en arrière-plan  ")
    }
}

fn push_model_and_mode(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    spans.push(Span::styled(
        state.model_label.clone(),
        Style::default().fg(theme::ACCENT),
    ));
    push_separator(spans);
    spans.push(Span::styled(state.mode_label.clone(), theme::dim_style()));
}

fn push_session_duration(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if let Some(start) = state.session_start {
        push_separator(spans);
        spans.push(Span::styled(
            duration_label(start.elapsed()),
            theme::dim_style(),
        ));
    }
}

fn duration_label(elapsed: Duration) -> String {
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    format!("{mins}m{secs:02}s")
}

fn push_last_tokens(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if let Some((input, output)) = state.last_tokens {
        push_separator(spans);
        spans.push(Span::styled(
            token_usage_label(input, output, state.last_cached_input_tokens),
            Style::default().fg(theme::TEXT_PRIMARY),
        ));
    }
}

fn token_usage_label(input: u64, output: u64, cached_input: u64) -> String {
    if cached_input > 0 {
        let effective_input = input.saturating_sub(cached_input);
        return format!(
            "{input}\u{2191} {output}\u{2193} · eff {}",
            compact_token_count(effective_input)
        );
    }
    format!("{input}\u{2191} {output}\u{2193}")
}

fn push_last_cost(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if let Some(cost) = state.last_cost_usd {
        push_separator(spans);
        spans.push(Span::styled(
            format!("${cost:.4}"),
            Style::default().fg(theme::GREEN),
        ));
    }
}

fn push_session_totals(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if state.session_input_tokens + state.session_output_tokens > 0 {
        push_separator(spans);
        let total = state.session_input_tokens + state.session_output_tokens;
        spans.push(Span::styled(
            format!("\u{03A3} {total} tok"),
            theme::dim_style(),
        ));
        if state.session_cost_usd > 0.0 {
            spans.push(Span::styled(
                format!(" / ${:.4}", state.session_cost_usd),
                Style::default().fg(theme::GREEN),
            ));
        }
    }
}

fn compact_token_count(tokens: u64) -> String {
    if tokens >= 1_000 {
        let value = tokens as f64 / 1_000.0;
        if tokens >= 10_000 {
            format!("{value:.0}k tok")
        } else {
            format!("{value:.1}k tok")
        }
    } else {
        format!("{tokens} tok")
    }
}
