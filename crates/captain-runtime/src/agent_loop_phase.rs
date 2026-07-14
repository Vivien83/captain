use std::sync::Arc;

/// Agent lifecycle phase within the execution loop.
/// Used for UX indicators (typing, reactions) without coupling to channel types.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopPhase {
    /// Agent is calling the LLM.
    Thinking,
    /// Agent is executing a tool.
    ToolUse { tool_name: String },
    /// Agent is streaming tokens.
    Streaming,
    /// Agent finished successfully.
    Done,
    /// Agent encountered an error.
    Error,
}

/// Callback for agent lifecycle phase changes.
/// Implementations should be non-blocking (fire-and-forget) to avoid slowing the loop.
pub type PhaseCallback = Arc<dyn Fn(LoopPhase) + Send + Sync>;

pub(crate) fn notify_thinking_phase(on_phase: Option<&PhaseCallback>) {
    let Some(callback) = on_phase else {
        return;
    };

    callback(LoopPhase::Thinking);
}

pub(crate) fn notify_stream_iteration_phase(on_phase: Option<&PhaseCallback>, iteration: u32) {
    let Some(callback) = on_phase else {
        return;
    };

    if iteration == 0 {
        callback(LoopPhase::Streaming);
    } else {
        callback(LoopPhase::Thinking);
    }
}

pub(crate) fn notify_tool_use_phase(on_phase: Option<&PhaseCallback>, tool_name: &str) {
    let Some(callback) = on_phase else {
        return;
    };
    callback(LoopPhase::ToolUse {
        tool_name: sanitize_tool_phase_name(tool_name),
    });
}

fn sanitize_tool_phase_name(tool_name: &str) -> String {
    tool_name
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn notify_thinking_phase_emits_thinking() {
        let seen = Arc::new(Mutex::new(None));
        let seen_for_cb = Arc::clone(&seen);
        let callback: PhaseCallback = Arc::new(move |phase| {
            *seen_for_cb.lock().unwrap() = Some(phase);
        });

        notify_thinking_phase(Some(&callback));

        assert_eq!(*seen.lock().unwrap(), Some(LoopPhase::Thinking));
    }

    #[test]
    fn notify_thinking_phase_allows_absent_callback() {
        notify_thinking_phase(None);
    }

    #[test]
    fn notify_stream_iteration_phase_uses_streaming_then_thinking() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_for_cb = Arc::clone(&seen);
        let callback: PhaseCallback = Arc::new(move |phase| {
            seen_for_cb.lock().unwrap().push(phase);
        });

        notify_stream_iteration_phase(Some(&callback), 0);
        notify_stream_iteration_phase(Some(&callback), 1);

        let phases = seen.lock().unwrap();
        assert_eq!(
            phases.as_slice(),
            &[LoopPhase::Streaming, LoopPhase::Thinking]
        );
    }

    #[test]
    fn notify_stream_iteration_phase_allows_absent_callback() {
        notify_stream_iteration_phase(None, 0);
        notify_stream_iteration_phase(None, 2);
    }

    #[test]
    fn notify_tool_use_phase_sanitizes_control_chars_and_caps_length() {
        let seen = Arc::new(Mutex::new(None));
        let seen_for_cb = Arc::clone(&seen);
        let callback: PhaseCallback = Arc::new(move |phase| {
            if let LoopPhase::ToolUse { tool_name } = phase {
                *seen_for_cb.lock().unwrap() = Some(tool_name);
            }
        });

        notify_tool_use_phase(Some(&callback), &format!("ab\ncd{}", "x".repeat(80)));

        let tool_name = seen.lock().unwrap().clone().unwrap();
        assert_eq!(tool_name.chars().count(), 64);
        assert!(!tool_name.contains('\n'));
        assert!(tool_name.starts_with("abcd"));
    }

    #[test]
    fn notify_tool_use_phase_allows_absent_callback() {
        notify_tool_use_phase(None, "shell_exec");
    }
}
