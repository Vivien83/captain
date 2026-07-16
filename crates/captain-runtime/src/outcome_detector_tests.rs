use super::*;

fn tool_success(agent: &str, tool: &str) -> LearningSignal {
    LearningSignal::ToolSuccess {
        agent_id: agent.into(),
        tool: tool.into(),
        duration_ms: 10,
        source: "tool_runner".into(),
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

#[test]
fn classify_user_message_fr_correction() {
    assert_eq!(
        classify_user_message("non c'est pas ce que je veux"),
        Some(UserMessageKind::Correction)
    );
    assert_eq!(
        classify_user_message("Non, pas comme ça"),
        Some(UserMessageKind::Correction)
    );
    assert_eq!(
        classify_user_message("je t'avais dit de faire X"),
        Some(UserMessageKind::Correction)
    );
}

#[test]
fn classify_user_message_fr_satisfaction() {
    assert_eq!(
        classify_user_message("parfait merci"),
        Some(UserMessageKind::Satisfaction)
    );
    assert_eq!(
        classify_user_message("top !"),
        Some(UserMessageKind::Satisfaction)
    );
    assert_eq!(
        classify_user_message("ok ça marche"),
        Some(UserMessageKind::Satisfaction)
    );
    assert_eq!(
        classify_user_message("génial"),
        Some(UserMessageKind::Satisfaction)
    );
}

#[test]
fn commit_b_classify_recognizes_extended_fr_remember_patterns() {
    let cases = [
        "Rappelle-toi que je préfère le vert",
        "Garde en tête mes horaires de travail",
        "N'oublie pas de revérifier nginx",
        "Prends note : j'ai changé de fuseau horaire",
        "Enregistre ma préférence : tutoiement uniquement",
        "Sache que je travaille en remote",
        "Don't forget my deadline next Friday",
    ];
    for msg in cases {
        assert_eq!(
            classify_user_message(msg),
            Some(UserMessageKind::ExplicitRemember),
            "Commit-B should classify '{msg}' as ExplicitRemember"
        );
    }
}

#[test]
fn commit_b_conversation_turn_outcome_for_signal() {
    let mut classifier = Classifier::new();
    let signal = LearningSignal::ConversationTurn {
        agent_id: "captain".into(),
        user_msg: "salut".into(),
        agent_response: "salut !".into(),
        channel: Some("telegram".into()),
        regex_hint: None,
        source: "test".into(),
    };
    assert_eq!(classifier.classify(&signal), Outcome::ConversationTurn);
}

#[test]
fn classify_user_message_fr_explicit_remember() {
    assert_eq!(
        classify_user_message("retiens que j'aime le thé"),
        Some(UserMessageKind::ExplicitRemember)
    );
    assert_eq!(
        classify_user_message("Souviens-toi de ma préférence"),
        Some(UserMessageKind::ExplicitRemember)
    );
    assert_eq!(
        classify_user_message("note que demain c'est férié"),
        Some(UserMessageKind::ExplicitRemember)
    );
}

#[test]
fn explicit_memory_opt_out_overrides_remember_patterns() {
    let cases = [
        "N'enregistre aucun nouveau souvenir à partir de ce message.",
        "Ne mémorise pas cette préférence.",
        "Ne l'enregistre pas dans ma mémoire.",
        "Do not remember this preference.",
        "Don't save this to memory.",
        "No new memory should be created from this request.",
    ];

    for msg in cases {
        assert!(memory_write_opt_out(msg), "opt-out not detected: {msg}");
        assert_eq!(
            classify_user_message(msg),
            None,
            "opt-out must override remember classification: {msg}"
        );
    }
}

#[test]
fn memory_opt_out_does_not_hide_real_preferences() {
    assert!(!memory_write_opt_out(
        "Enregistre ma préférence : réponses courtes."
    ));
    assert!(!memory_write_opt_out("Ne fais jamais de résumé à la fin."));
}

#[test]
fn classify_user_message_en_patterns() {
    assert_eq!(
        classify_user_message("No, not that one"),
        Some(UserMessageKind::Correction)
    );
    assert_eq!(
        classify_user_message("thanks!"),
        Some(UserMessageKind::Satisfaction)
    );
    assert_eq!(
        classify_user_message("remember that I prefer dark mode"),
        Some(UserMessageKind::ExplicitRemember)
    );
}

#[test]
fn classify_user_message_neutral_returns_none() {
    assert_eq!(classify_user_message("peux-tu m'aider ?"), None);
    assert_eq!(classify_user_message("what's the weather in Paris"), None);
    assert_eq!(classify_user_message(""), None);
}

#[test]
fn classify_user_message_avoids_pas_parfait_false_positive() {
    let got = classify_user_message("pas parfait du tout");
    assert_eq!(got, Some(UserMessageKind::Correction));
}

#[test]
fn explicit_remember_wins_over_correction() {
    let got = classify_user_message("Non, retiens que j'aime le café");
    assert_eq!(got, Some(UserMessageKind::ExplicitRemember));
}

#[test]
fn first_tool_failure_updates_buffer_without_reflection() {
    let mut c = Classifier::new();
    assert_eq!(c.classify(&tool_failure("a", "bash")), Outcome::Unknown);
}

#[test]
fn repeated_tool_failure_yields_failure_outcome() {
    let mut c = Classifier::new();
    assert_eq!(c.classify(&tool_failure("a", "bash")), Outcome::Unknown);
    assert_eq!(c.classify(&tool_failure("a", "bash")), Outcome::Failure);
}

#[test]
fn tool_success_without_prior_failures_updates_buffer_without_reflection() {
    let mut c = Classifier::new();
    assert_eq!(c.classify(&tool_success("a", "bash")), Outcome::Unknown);
}

#[test]
fn retry_success_detected_after_two_failures() {
    let mut c = Classifier::new();
    c.classify(&tool_failure("a", "bash"));
    c.classify(&tool_failure("a", "bash"));
    assert_eq!(
        c.classify(&tool_success("a", "bash")),
        Outcome::RetrySuccess
    );
}

#[test]
fn retry_success_requires_consecutive_failures() {
    let mut c = Classifier::new();
    c.classify(&tool_failure("a", "bash"));
    c.classify(&tool_success("a", "bash"));
    c.classify(&tool_failure("a", "bash"));
    assert_eq!(c.classify(&tool_success("a", "bash")), Outcome::Unknown);
}

#[test]
fn retry_success_scoped_to_same_agent_and_tool() {
    let mut c = Classifier::new();
    c.classify(&tool_failure("a", "bash"));
    c.classify(&tool_failure("a", "bash"));
    assert_eq!(c.classify(&tool_success("b", "bash")), Outcome::Unknown);
    assert_eq!(c.classify(&tool_success("a", "curl")), Outcome::Unknown);
}

#[test]
fn user_correction_signal_maps_to_user_corrected() {
    let mut c = Classifier::new();
    let sig = LearningSignal::UserCorrection {
        agent_id: "a".into(),
        user_msg: "non".into(),
        source: "ws".into(),
    };
    assert_eq!(c.classify(&sig), Outcome::UserCorrected);
}

#[test]
fn explicit_remember_signal_maps_to_explicit_remember_outcome() {
    let mut c = Classifier::new();
    let sig = LearningSignal::ExplicitRemember {
        agent_id: "a".into(),
        user_msg: "retiens".into(),
        source: "ws".into(),
    };
    assert_eq!(c.classify(&sig), Outcome::ExplicitRemember);
}

#[test]
fn workflow_cancellation_detected() {
    let mut c = Classifier::new();
    let sig = LearningSignal::WorkflowRunComplete {
        agent_id: "a".into(),
        outcome: "cancelled by user".into(),
        tool_calls: 3,
        source: "kernel".into(),
    };
    assert_eq!(c.classify(&sig), Outcome::Cancelled);
}

#[test]
fn workflow_failure_detected() {
    let mut c = Classifier::new();
    let sig = LearningSignal::WorkflowRunComplete {
        agent_id: "a".into(),
        outcome: "failure: timeout".into(),
        tool_calls: 0,
        source: "kernel".into(),
    };
    assert_eq!(c.classify(&sig), Outcome::Failure);
}

#[test]
fn rolling_buffer_caps_at_capacity() {
    let mut c = Classifier::with_capacity(3);
    for _ in 0..5 {
        c.classify(&tool_failure("a", "bash"));
    }
    assert_eq!(c.events.len(), 3);
}

#[tokio::test]
async fn detector_loop_forwards_classified_signals() {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (_handle, mut out_rx) = OutcomeDetector::spawn(rx, 8);

    tx.send(LearningSignal::UserCorrection {
        agent_id: "a".into(),
        user_msg: "non".into(),
        source: "ws".into(),
    })
    .await
    .unwrap();
    tx.send(LearningSignal::ExplicitRemember {
        agent_id: "a".into(),
        user_msg: "retiens".into(),
        source: "ws".into(),
    })
    .await
    .unwrap();
    drop(tx);

    let a = out_rx.recv().await.unwrap();
    assert_eq!(a.outcome, Outcome::UserCorrected);
    let b = out_rx.recv().await.unwrap();
    assert_eq!(b.outcome, Outcome::ExplicitRemember);
}

#[tokio::test]
async fn detector_drops_unknown_outcomes() {
    let mut classifier = Classifier::new();
    assert!(matches!(
        classifier.classify(&tool_success("a", "t")),
        Outcome::Unknown
    ));
    assert!(matches!(
        classifier.classify(&tool_failure("a", "t")),
        Outcome::Unknown
    ));
    assert!(matches!(
        classifier.classify(&LearningSignal::ApprovalDecision {
            agent_id: "a".into(),
            approved: true,
            action: "x".into(),
            source: "s".into(),
        }),
        Outcome::ApprovalDecision
    ));
}
