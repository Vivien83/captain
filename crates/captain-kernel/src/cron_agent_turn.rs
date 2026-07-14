//! Inactivity-based execution guard for cron agent turns.

use crate::error::KernelError;
use crate::kernel::CaptainKernel;
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

pub const DEFAULT_CRON_AGENT_INACTIVITY_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronAgentInactivity {
    pub idle_secs: u64,
    pub limit_secs: u64,
    pub last_activity: String,
}

#[derive(Debug, Error)]
pub enum CronAgentTurnError {
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error("idle for {idle_secs}s (limit {limit_secs}s); last activity: {last_activity}")]
    Inactivity {
        idle_secs: u64,
        limit_secs: u64,
        last_activity: String,
    },
    #[error("agent task join failed: {0}")]
    Join(String),
}

impl From<CronAgentInactivity> for CronAgentTurnError {
    fn from(value: CronAgentInactivity) -> Self {
        Self::Inactivity {
            idle_secs: value.idle_secs,
            limit_secs: value.limit_secs,
            last_activity: value.last_activity,
        }
    }
}

struct CronAgentActivity {
    last_activity: Instant,
    last_description: String,
}

impl CronAgentActivity {
    fn started() -> Self {
        Self::started_at(Instant::now())
    }

    fn started_at(now: Instant) -> Self {
        Self {
            last_activity: now,
            last_description: "agent turn started".to_string(),
        }
    }

    fn observe(&mut self, event: &StreamEvent) {
        self.observe_at(event, Instant::now());
    }

    fn observe_at(&mut self, event: &StreamEvent, now: Instant) {
        self.last_activity = now;
        self.last_description = describe_stream_event(event);
    }

    fn inactivity(&self, limit: Duration) -> CronAgentInactivity {
        self.inactivity_at(limit, Instant::now())
    }

    fn inactivity_at(&self, limit: Duration, now: Instant) -> CronAgentInactivity {
        CronAgentInactivity {
            idle_secs: now.duration_since(self.last_activity).as_secs(),
            limit_secs: limit.as_secs(),
            last_activity: self.last_description.clone(),
        }
    }
}

pub async fn run_agent_turn_with_inactivity_timeout(
    kernel: &Arc<CaptainKernel>,
    agent_id: AgentId,
    message: &str,
    kernel_handle: Arc<dyn KernelHandle>,
    inactivity_limit: Duration,
) -> Result<AgentLoopResult, CronAgentTurnError> {
    if inactivity_limit.is_zero() {
        return kernel
            .send_message_with_handle(agent_id, message, Some(kernel_handle), None, None)
            .await
            .map_err(CronAgentTurnError::Kernel);
    }

    let (mut events, handle, _user_input_tx) = kernel.send_message_streaming(
        agent_id,
        message,
        Some(kernel_handle),
        None,
        None,
        None,
        None,
    )?;
    let mut activity = CronAgentActivity::started();

    loop {
        if handle.is_finished() {
            break;
        }

        match tokio::time::timeout(inactivity_limit, events.recv()).await {
            Ok(Some(event)) => activity.observe(&event),
            Ok(None) => break,
            Err(_) => {
                let inactive = activity.inactivity(inactivity_limit);
                let _ = kernel.stop_agent_run(agent_id);
                handle.abort();
                let _ = handle.await;
                return Err(inactive.into());
            }
        }
    }

    match handle.await {
        Ok(result) => result.map_err(CronAgentTurnError::Kernel),
        Err(err) => Err(CronAgentTurnError::Join(err.to_string())),
    }
}

fn describe_stream_event(event: &StreamEvent) -> String {
    match event {
        StreamEvent::TextDelta { text } => format!("text delta ({} chars)", text.chars().count()),
        StreamEvent::ToolUseStart { name, .. } => format!("tool started: {name}"),
        StreamEvent::ToolInputDelta { text } => {
            format!("tool input delta ({} chars)", text.chars().count())
        }
        StreamEvent::ToolUseEnd { name, .. } => format!("tool input complete: {name}"),
        StreamEvent::ThinkingDelta { text } => {
            format!("thinking delta ({} chars)", text.chars().count())
        }
        StreamEvent::ContentComplete { stop_reason, .. } => {
            format!("content complete: {stop_reason:?}")
        }
        StreamEvent::PhaseChange { phase, detail } => match detail {
            Some(detail) if !detail.is_empty() => format!("phase: {phase} ({detail})"),
            _ => format!("phase: {phase}"),
        },
        StreamEvent::ToolExecutionResult { name, is_error, .. } => {
            let status = if *is_error { "error" } else { "ok" };
            format!("tool finished: {name} ({status})")
        }
        StreamEvent::ToolOutputDelta { stream, chunk, .. } => {
            format!("tool output {stream} ({} chars)", chunk.chars().count())
        }
        StreamEvent::IntermediateMessage { content } => {
            format!("intermediate message ({} chars)", content.chars().count())
        }
        StreamEvent::AskUser { question, .. } => {
            format!("waiting for user ({} chars)", question.chars().count())
        }
        StreamEvent::UserResponse { content } => {
            format!("user response ({} chars)", content.chars().count())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{StopReason, TokenUsage};

    #[test]
    fn stream_event_descriptions_are_operational() {
        let event = StreamEvent::ToolUseStart {
            id: "toolu_1".to_string(),
            name: "shell_exec".to_string(),
        };
        assert_eq!(describe_stream_event(&event), "tool started: shell_exec");

        let event = StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        };
        assert_eq!(describe_stream_event(&event), "content complete: EndTurn");
    }

    #[test]
    fn inactivity_report_keeps_last_activity_detail() {
        let started = Instant::now();
        let mut activity = CronAgentActivity::started_at(started);
        activity.observe_at(
            &StreamEvent::PhaseChange {
                phase: "tool_use".to_string(),
                detail: Some("web_fetch".to_string()),
            },
            started + Duration::from_secs(4),
        );

        let report =
            activity.inactivity_at(Duration::from_secs(10), started + Duration::from_secs(17));

        assert_eq!(
            report,
            CronAgentInactivity {
                idle_secs: 13,
                limit_secs: 10,
                last_activity: "phase: tool_use (web_fetch)".to_string(),
            }
        );
    }
}
