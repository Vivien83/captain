use super::*;
use captain_types::agent::{AgentId, SessionId};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn test_session() -> Session {
    Session {
        id: SessionId::new(),
        agent_id: AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    }
}

#[tokio::test]
async fn finish_silent_turn_persists_marker_and_directives() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session();
    let directives = ReplyDirectives {
        reply_to: Some("msg_1".to_string()),
        current_thread: true,
        silent: true,
    };

    let result = finish_silent_turn(
        &mut session,
        &memory,
        TokenUsage::default(),
        2,
        directives,
        &[],
    )
    .await
    .unwrap();

    assert!(result.silent);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.directives.reply_to.as_deref(), Some("msg_1"));
    assert_eq!(
        session.messages.last().unwrap().content.text_content(),
        "[no reply needed]"
    );
}

#[tokio::test]
async fn finish_successful_turn_saves_message_and_marks_done() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session();
    let mut manifest = AgentManifest::default();
    manifest.name = "captain".to_string();
    let done_seen = Arc::new(AtomicBool::new(false));
    let done_for_cb = Arc::clone(&done_seen);
    let phase_cb: PhaseCallback = Arc::new(move |phase| {
        if matches!(phase, LoopPhase::Done) {
            done_for_cb.store(true, Ordering::SeqCst);
        }
    });

    let result = finish_successful_turn(SuccessfulTurnInput {
        manifest: &manifest,
        user_message: "hello",
        final_response: "final answer".to_string(),
        assistant_message: Message::assistant("final answer"),
        completed_iterations: 3,
        session: &mut session,
        memory: &memory,
        embedding_driver: None,
        on_phase: Some(&phase_cb),
        hooks: None,
        agent_id_str: "agent",
        total_usage: TokenUsage::default(),
        tool_calls_recorded: &[],
        streaming: false,
    })
    .await
    .unwrap();

    assert_eq!(result.response, "final answer");
    assert_eq!(result.iterations, 3);
    assert!(done_seen.load(Ordering::SeqCst));
    assert_eq!(
        session.messages.last().unwrap().content.text_content(),
        "final answer"
    );
}
