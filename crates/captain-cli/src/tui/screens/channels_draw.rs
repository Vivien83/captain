//! Drawing helpers for the channel setup screen.

use super::{ChannelFieldType, ChannelState, ChannelStatus, ChannelSubScreen, CATEGORIES};
use crate::tui::theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Padding, Paragraph};
use ratatui::Frame;

pub fn draw(f: &mut Frame, area: Rect, state: &mut ChannelState) {
    let ready = state.ready_count();
    let total = state.channels.len();
    let title = format!(" Channels ({ready}/{total} ready) ");

    let block = Block::default()
        .title(Line::from(vec![Span::styled(title, theme::title_style())]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    match state.sub {
        ChannelSubScreen::List => draw_list(f, inner, state),
        ChannelSubScreen::Setup => draw_setup(f, inner, state),
        ChannelSubScreen::Testing => draw_testing(f, inner, state),
    }
}

fn draw_list(f: &mut Frame, area: Rect, state: &mut ChannelState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(area);

    let cat_spans: Vec<Span> = CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            if i == state.category_idx {
                Span::styled(
                    format!(" [{cat}] "),
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {cat}  "), theme::dim_style())
            }
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(cat_spans)), chunks[0]);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<18} {:<14} {:<16} {}",
                "Channel", "Category", "Status", "Details"
            ),
            theme::table_header(),
        )])),
        chunks[1],
    );

    if state.loading {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading channels\u{2026}", theme::dim_style()),
            ])),
            chunks[2],
        );
    } else {
        let items: Vec<ListItem> = state
            .filtered_channels()
            .iter()
            .map(|ch| {
                let (badge, badge_style) = channel_status_presentation(ch.status);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<18}", ch.display_name),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(format!("{:<14}", ch.category), theme::dim_style()),
                    Span::styled(format!(" {:<16}", badge), badge_style),
                    Span::styled(format!(" {}", ch.summary), theme::dim_style()),
                ]))
            })
            .collect();

        let list = List::new(items)
            .highlight_style(theme::selected_style())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[2], &mut state.list_state);
    }

    let status = if state.status_msg.is_empty() {
        Line::from("")
    } else {
        Line::from(vec![Span::styled(
            format!("  {}", state.status_msg),
            Style::default().fg(theme::YELLOW),
        )])
    };
    f.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(vec![Span::styled(
                "  [\u{2191}\u{2193}] Navigate  [Tab] Category  [Enter] Setup  [t] Test  [r] Refresh",
                theme::hint_style(),
            )]),
        ]),
        chunks[3],
    );
}

fn channel_status_presentation(status: ChannelStatus) -> (&'static str, Style) {
    match status {
        ChannelStatus::Ready => ("[Ready]", theme::channel_ready()),
        ChannelStatus::Partial => ("[Partial]", theme::channel_missing()),
        ChannelStatus::Locked => ("[Locked]", theme::channel_missing()),
        ChannelStatus::Disabled => ("[Disabled]", theme::channel_off()),
        ChannelStatus::Invalid => ("[Invalid]", Style::default().fg(theme::RED)),
        ChannelStatus::NotConfigured => ("[Not configured]", theme::channel_off()),
    }
}

fn draw_setup(f: &mut Frame, area: Rect, state: &ChannelState) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    let channel = setup_channel(state);
    let ch_display = channel
        .map(|channel| channel.display_name.as_str())
        .unwrap_or("?");
    let ch_desc = channel
        .map(|channel| channel.description.as_str())
        .unwrap_or("");

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![Span::styled(
                format!("  Setup: {ch_display}"),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![Span::styled(
                format!("  {ch_desc}"),
                theme::dim_style(),
            )]),
        ]),
        chunks[0],
    );

    let sep = "\u{2500}".repeat(chunks[1].width as usize);
    f.render_widget(
        Paragraph::new(Span::styled(sep, theme::dim_style())),
        chunks[1],
    );

    draw_setup_field(f, chunks[2], state);
    draw_setup_input(f, chunks[3], state);
    draw_setup_preview(f, chunks[4], state);
    if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::YELLOW),
            )])),
            chunks[5],
        );
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  [Enter] Next field / Save  [Esc] Back",
            theme::hint_style(),
        )])),
        chunks[6],
    );
}

fn setup_channel(state: &ChannelState) -> Option<&super::ChannelInfo> {
    state
        .setup_channel_idx
        .and_then(|index| state.channels.get(index))
}

fn draw_setup_field(f: &mut Frame, area: Rect, state: &ChannelState) {
    let Some(channel) = setup_channel(state) else {
        return;
    };
    if channel.setup_fields.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  Run `captain channel setup {}`", channel.name),
                theme::dim_style(),
            )])),
            area,
        );
    } else if let Some(field) = channel.setup_fields.get(state.setup_field_idx) {
        let field_num = state.setup_field_idx + 1;
        let total = channel.setup_fields.len();
        let required = if state.field_requires_input(field) {
            " · required"
        } else {
            " · optional"
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(format!("  [{field_num}/{total}] ")),
                Span::styled(&field.label, Style::default().fg(theme::YELLOW)),
                Span::styled(required, theme::dim_style()),
            ])),
            area,
        );
    }
}

fn draw_setup_input(f: &mut Frame, area: Rect, state: &ChannelState) {
    let field =
        setup_channel(state).and_then(|channel| channel.setup_fields.get(state.setup_field_idx));
    let display = if state.setup_input.is_empty() {
        field
            .map(|field| field.placeholder.as_str())
            .filter(|placeholder| !placeholder.is_empty())
            .unwrap_or("enter value")
            .to_string()
    } else if field.is_some_and(|field| field.field_type == ChannelFieldType::Secret) {
        "*".repeat(state.setup_input.chars().count())
    } else {
        state.setup_input.clone()
    };
    let style = if state.setup_input.is_empty() {
        theme::dim_style()
    } else {
        theme::input_style()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  > "),
            Span::styled(display, style),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])),
        area,
    );
}

fn draw_setup_preview(f: &mut Frame, area: Rect, state: &ChannelState) {
    let Some(channel) = setup_channel(state) else {
        return;
    };
    let mut lines = vec![Line::from(Span::styled(
        format!("  {} configuration", channel.display_name),
        theme::dim_style(),
    ))];
    for (key, value) in &state.setup_values {
        let secret = channel
            .setup_fields
            .iter()
            .find(|field| field.key == *key)
            .is_some_and(|field| field.field_type == ChannelFieldType::Secret);
        lines.push(Line::from(vec![
            Span::styled(format!("  {key}: "), theme::dim_style()),
            Span::styled(
                if secret { "********" } else { value.as_str() },
                Style::default().fg(theme::YELLOW),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_testing(f: &mut Frame, area: Rect, state: &ChannelState) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(area);

    let ch_name = current_channel_display_name(state);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  Testing {ch_name}\u{2026}"),
            Style::default().fg(theme::CYAN),
        )])),
        chunks[0],
    );

    match &state.test_result {
        None => draw_test_pending(f, chunks[1], state),
        Some((true, msg)) => draw_test_done(f, chunks[1], true, msg),
        Some((false, msg)) => draw_test_done(f, chunks[1], false, msg),
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  [Enter/Esc] Back",
            theme::hint_style(),
        )])),
        chunks[2],
    );
}

fn current_channel_display_name(state: &ChannelState) -> &str {
    state
        .setup_channel_idx
        .and_then(|i| state.channels.get(i))
        .map(|c| c.display_name.as_str())
        .or_else(|| {
            state.list_state.selected().and_then(|i| {
                let filtered = state.filtered_channels();
                filtered.get(i).map(|c| c.display_name.as_str())
            })
        })
        .unwrap_or("?")
}

fn draw_test_pending(f: &mut Frame, area: Rect, state: &ChannelState) {
    let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
            Span::styled("Checking credentials\u{2026}", theme::dim_style()),
        ])),
        area,
    );
}

fn draw_test_done(f: &mut Frame, area: Rect, passed: bool, msg: &str) {
    let mark = if passed { "\u{2714}" } else { "\u{2718}" };
    let label = if passed { "Test passed" } else { "Test failed" };
    let style = if passed {
        Style::default().fg(theme::GREEN)
    } else {
        Style::default().fg(theme::RED)
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!("  {mark} "), style),
                Span::raw(label),
            ]),
            Line::from(vec![Span::styled(format!("  {msg}"), style)]),
        ]),
        area,
    );
}
