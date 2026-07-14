use super::*;

fn tool_success(agent: &str, source: &str) -> LearningSignal {
    LearningSignal::ToolSuccess {
        agent_id: agent.into(),
        tool: "web_search".into(),
        duration_ms: 42,
        source: source.into(),
    }
}

fn tool_failure(agent: &str, tool: &str) -> LearningSignal {
    LearningSignal::ToolFailure {
        agent_id: agent.into(),
        tool: tool.into(),
        error: "boom".into(),
        source: "tool_runner".into(),
    }
}

fn approval(agent: &str, action: &str) -> LearningSignal {
    LearningSignal::ApprovalDecision {
        agent_id: agent.into(),
        approved: true,
        action: action.into(),
        source: "approval_flow".into(),
    }
}

#[tokio::test]
async fn emit_delivers_signal_to_receiver() {
    let (bus, mut rx) = LearningBus::new(8);
    assert_eq!(
        bus.emit(tool_success("a1", "tool_runner")),
        EmitResult::Emitted
    );
    let got = rx.recv().await.unwrap();
    assert_eq!(got.agent_id(), "a1");
    assert_eq!(got.kind(), "tool_success");
}

#[tokio::test]
async fn anti_loop_drops_learning_sourced_signals() {
    let (bus, mut rx) = LearningBus::new(8);
    let sig = tool_success("a1", "learning.reflector");
    assert_eq!(bus.emit(sig), EmitResult::SkippedLoop);
    tokio::time::timeout(Duration::from_millis(40), rx.recv())
        .await
        .unwrap_err();
}

#[tokio::test]
async fn approval_rate_limit_blocks_second_emit_within_window() {
    let (bus, _rx) = LearningBus::new(8);
    assert_eq!(
        bus.emit(approval("a1", "shell_exec:rm")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(approval("a1", "shell_exec:rm")),
        EmitResult::SkippedRateLimit
    );
}

#[tokio::test]
async fn approval_rate_limit_is_per_agent_and_action() {
    let (bus, _rx) = LearningBus::new(8);
    assert_eq!(
        bus.emit(approval("a1", "shell_exec:rm")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(approval("a2", "shell_exec:rm")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(approval("a1", "config_write")),
        EmitResult::Emitted
    );
}

#[tokio::test]
async fn tool_outcomes_bypass_rate_limit_for_retry_detector() {
    let (bus, mut rx) = LearningBus::new(8);
    assert_eq!(
        bus.emit(tool_failure("a1", "ssh_exec")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(tool_failure("a1", "ssh_exec")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(LearningSignal::ToolSuccess {
            agent_id: "a1".into(),
            tool: "ssh_exec".into(),
            duration_ms: 42,
            source: "tool_runner".into(),
        }),
        EmitResult::Emitted
    );

    assert_eq!(rx.recv().await.unwrap().kind(), "tool_failure");
    assert_eq!(rx.recv().await.unwrap().kind(), "tool_failure");
    assert_eq!(rx.recv().await.unwrap().kind(), "tool_success");
}

#[tokio::test]
async fn end_of_turn_signals_bypass_tool_rate_limit() {
    let (bus, mut rx) = LearningBus::new(8);
    assert_eq!(
        bus.emit(tool_success("a1", "tool_runner")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(LearningSignal::ConversationTurn {
            agent_id: "a1".into(),
            user_msg: "do the long task".into(),
            agent_response: "done".into(),
            channel: Some("telegram".into()),
            regex_hint: None,
            source: "kernel.send_message_full".into(),
        }),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(LearningSignal::WorkflowRunComplete {
            agent_id: "a1".into(),
            outcome: "success; tools: capability_search ok -> ssh_exec ok".into(),
            tool_calls: 2,
            source: "kernel.send_message_full".into(),
        }),
        EmitResult::Emitted
    );

    assert_eq!(rx.recv().await.unwrap().kind(), "tool_success");
    assert_eq!(rx.recv().await.unwrap().kind(), "conversation_turn");
    assert_eq!(rx.recv().await.unwrap().kind(), "workflow_run_complete");
}

#[tokio::test]
async fn explicit_learning_signals_bypass_tool_rate_limit() {
    let (bus, mut rx) = LearningBus::new(8);
    assert_eq!(
        bus.emit(tool_success("a1", "tool_runner")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(LearningSignal::ExplicitRemember {
            agent_id: "a1".into(),
            user_msg: "remember that I prefer short reports".into(),
            source: "kernel.send_message_full".into(),
        }),
        EmitResult::Emitted
    );

    assert_eq!(rx.recv().await.unwrap().kind(), "tool_success");
    assert_eq!(rx.recv().await.unwrap().kind(), "explicit_remember");
}

#[tokio::test]
async fn channel_full_returns_skipped_full() {
    let (bus, _rx) = LearningBus::new(1);
    assert_eq!(
        bus.emit(tool_success("a1", "tool_runner")),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(tool_success("a1", "tool_runner")),
        EmitResult::SkippedFull
    );
}

#[tokio::test]
async fn three_different_sources_all_reach_receiver() {
    let (bus, mut rx) = LearningBus::new(8);

    assert_eq!(
        bus.emit(LearningSignal::ToolSuccess {
            agent_id: "agent_a".into(),
            tool: "bash".into(),
            duration_ms: 10,
            source: "tool_runner".into(),
        }),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(LearningSignal::UserCorrection {
            agent_id: "agent_b".into(),
            user_msg: "non c'est pas ca".into(),
            source: "ws.rs".into(),
        }),
        EmitResult::Emitted
    );
    assert_eq!(
        bus.emit(LearningSignal::WorkflowRunComplete {
            agent_id: "agent_c".into(),
            outcome: "success".into(),
            tool_calls: 3,
            source: "kernel.rs".into(),
        }),
        EmitResult::Emitted
    );

    let s1 = rx.recv().await.unwrap();
    assert_eq!(s1.source(), "tool_runner");
    let s2 = rx.recv().await.unwrap();
    assert_eq!(s2.source(), "ws.rs");
    let s3 = rx.recv().await.unwrap();
    assert_eq!(s3.source(), "kernel.rs");
}

#[tokio::test]
async fn truncate_long_strings_in_emitted_signal() {
    let (bus, mut rx) = LearningBus::new(8);
    let long = "x".repeat(10_000);
    bus.emit(LearningSignal::UserCorrection {
        agent_id: "a1".into(),
        user_msg: long,
        source: "ws".into(),
    });
    let got = rx.recv().await.unwrap();
    if let LearningSignal::UserCorrection { user_msg, .. } = got {
        assert!(user_msg.len() <= MAX_TEXT_LEN);
    } else {
        panic!("wrong variant");
    }
}

#[tokio::test]
async fn truncate_long_conversation_turn_payloads() {
    let (bus, mut rx) = LearningBus::new(8);
    let long_user = "u".repeat(10_000);
    let long_agent = "a".repeat(12_000);
    bus.emit(LearningSignal::ConversationTurn {
        agent_id: "a1".into(),
        user_msg: long_user,
        agent_response: long_agent,
        channel: Some("web".into()),
        regex_hint: Some("satisfaction".into()),
        source: "kernel.send_message_full".into(),
    });
    let got = rx.recv().await.unwrap();
    if let LearningSignal::ConversationTurn {
        user_msg,
        agent_response,
        channel,
        regex_hint,
        ..
    } = got
    {
        assert!(user_msg.len() <= MAX_TEXT_LEN);
        assert!(agent_response.len() <= MAX_TEXT_LEN);
        assert_eq!(channel.as_deref(), Some("web"));
        assert_eq!(regex_hint.as_deref(), Some("satisfaction"));
    } else {
        panic!("wrong variant");
    }
}

#[tokio::test]
async fn forget_agent_clears_rate_limit() {
    let (bus, _rx) = LearningBus::new(8);
    bus.emit(approval("a1", "config_write"));
    assert_eq!(
        bus.emit(approval("a1", "config_write")),
        EmitResult::SkippedRateLimit
    );
    bus.forget_agent("a1");
    assert_eq!(
        bus.emit(approval("a1", "config_write")),
        EmitResult::Emitted
    );
}

#[test]
fn signal_serializes_with_type_discriminator() {
    let sig = LearningSignal::ToolFailure {
        agent_id: "a1".into(),
        tool: "web_fetch".into(),
        error: "timeout".into(),
        source: "tool_runner".into(),
    };
    let json = serde_json::to_string(&sig).unwrap();
    assert!(json.contains("\"type\":\"tool_failure\""));
    let back: LearningSignal = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sig);
}

#[test]
fn kind_is_stable_for_each_variant() {
    let kinds = [
        LearningSignal::ToolSuccess {
            agent_id: "a".into(),
            tool: "t".into(),
            duration_ms: 0,
            source: "s".into(),
        }
        .kind(),
        LearningSignal::ToolFailure {
            agent_id: "a".into(),
            tool: "t".into(),
            error: "e".into(),
            source: "s".into(),
        }
        .kind(),
        LearningSignal::UserCorrection {
            agent_id: "a".into(),
            user_msg: "m".into(),
            source: "s".into(),
        }
        .kind(),
    ];
    assert_eq!(kinds, ["tool_success", "tool_failure", "user_correction"]);
}
