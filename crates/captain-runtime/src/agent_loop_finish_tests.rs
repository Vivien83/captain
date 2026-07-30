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
        suggested_replies: None,
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
    assert_eq!(
        memory.recall("final answer", 10, None).await.unwrap().len(),
        1,
        "ordinary successful turns must keep episodic recall"
    );
}

#[tokio::test]
async fn finish_successful_turn_honors_memory_opt_out_before_episodic_capture() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session();
    let mut manifest = AgentManifest::default();
    manifest.name = "captain".to_string();
    let marker = "PRIVATE-OPT-OUT-7264";
    let user_message = format!("Do not remember this preference. The private marker is {marker}.");

    let result = finish_successful_turn(SuccessfulTurnInput {
        manifest: &manifest,
        user_message: &user_message,
        final_response: "Acknowledged without durable memory.".to_string(),
        assistant_message: Message::assistant("Acknowledged without durable memory."),
        completed_iterations: 1,
        session: &mut session,
        memory: &memory,
        embedding_driver: None,
        on_phase: None,
        hooks: None,
        agent_id_str: "agent",
        total_usage: TokenUsage::default(),
        tool_calls_recorded: &[],
        streaming: true,
    })
    .await
    .unwrap();

    assert_eq!(result.response, "Acknowledged without durable memory.");
    assert_eq!(
        session.messages.last().unwrap().content.text_content(),
        "Acknowledged without durable memory.",
        "the resumable session transcript must still be persisted"
    );
    assert!(
        memory.recall(marker, 10, None).await.unwrap().is_empty(),
        "the finalizer must not create an episodic fragment for an opted-out turn"
    );
}
