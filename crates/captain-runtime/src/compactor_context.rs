//! Context-window reporting for session compaction.

use captain_types::message::Message;
use captain_types::tool::ToolDefinition;
use serde::Serialize;

/// Context window pressure level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextPressure {
    /// < 50% usage
    Low,
    /// 50-70% usage
    Medium,
    /// 70-85% usage
    High,
    /// > 85% usage
    Critical,
}

impl ContextPressure {
    pub(crate) fn from_percent(pct: f64) -> Self {
        if pct > 85.0 {
            Self::Critical
        } else if pct > 70.0 {
            Self::High
        } else if pct > 50.0 {
            Self::Medium
        } else {
            Self::Low
        }
    }

    /// CSS-friendly color name.
    pub fn color(&self) -> &'static str {
        match self {
            Self::Low => "green",
            Self::Medium => "yellow",
            Self::High => "orange",
            Self::Critical => "red",
        }
    }
}

/// Token breakdown by source.
#[derive(Debug, Clone, Serialize)]
pub struct ContextBreakdown {
    pub system_prompt_tokens: usize,
    pub message_tokens: usize,
    pub tool_definition_tokens: usize,
}

/// Context window usage report.
#[derive(Debug, Clone, Serialize)]
pub struct ContextReport {
    pub estimated_tokens: usize,
    pub context_window: usize,
    pub usage_percent: f64,
    pub pressure: ContextPressure,
    pub message_count: usize,
    pub breakdown: ContextBreakdown,
    pub recommendation: String,
}

/// Generate a context window usage report.
pub fn generate_context_report(
    messages: &[Message],
    system_prompt: Option<&str>,
    tools: Option<&[ToolDefinition]>,
    context_window: usize,
) -> ContextReport {
    let sp_tokens = system_prompt.map_or(0, |s| s.len() / 4);
    let msg_tokens = message_tokens(messages);
    let tool_tokens = tools.map_or(0, tool_definition_tokens);

    let total = sp_tokens + msg_tokens + tool_tokens;
    let cw = context_window.max(1);
    let pct = (total as f64 / cw as f64 * 100.0).min(100.0);
    let pressure = ContextPressure::from_percent(pct);

    ContextReport {
        estimated_tokens: total,
        context_window: cw,
        usage_percent: (pct * 10.0).round() / 10.0,
        pressure,
        message_count: messages.len(),
        breakdown: ContextBreakdown {
            system_prompt_tokens: sp_tokens,
            message_tokens: msg_tokens,
            tool_definition_tokens: tool_tokens,
        },
        recommendation: recommendation_for(pressure).to_string(),
    }
}

fn message_tokens(messages: &[Message]) -> usize {
    let mut chars: usize = 0;
    for msg in messages {
        chars += msg.content.text_length() + 16;
    }
    chars / 4
}

fn tool_definition_tokens(defs: &[ToolDefinition]) -> usize {
    let mut chars: usize = 0;
    for t in defs {
        chars += t.name.len() + t.description.len();
        if let Ok(s) = serde_json::to_string(&t.input_schema) {
            chars += s.len();
        }
    }
    chars / 4
}

fn recommendation_for(pressure: ContextPressure) -> &'static str {
    match pressure {
        ContextPressure::Low => "Context usage is healthy.",
        ContextPressure::Medium => "Consider using /compact if the conversation grows longer.",
        ContextPressure::High => {
            "Context is getting full. Use /compact to summarize older messages."
        }
        ContextPressure::Critical => "Context is nearly full! Use /compact or /new immediately.",
    }
}

/// Format a context report as human-readable text with ASCII progress bar.
pub fn format_context_report(report: &ContextReport) -> String {
    let bar_len: usize = 20;
    let filled = ((report.usage_percent / 100.0) * bar_len as f64).round() as usize;
    let empty = bar_len.saturating_sub(filled);
    let bar: String = std::iter::repeat_n('█', filled)
        .chain(std::iter::repeat_n('░', empty))
        .collect();

    format!(
        "**Context Usage:** {bar} {:.1}% ({} / {} tokens)\n\n\
         **Breakdown:**\n\
         - System prompt: ~{} tokens\n\
         - Messages ({}): ~{} tokens\n\
         - Tool definitions: ~{} tokens\n\n\
         **Pressure:** {:?}\n\
         **Recommendation:** {}",
        report.usage_percent,
        report.estimated_tokens,
        report.context_window,
        report.breakdown.system_prompt_tokens,
        report.message_count,
        report.breakdown.message_tokens,
        report.breakdown.tool_definition_tokens,
        report.pressure,
        report.recommendation,
    )
}
