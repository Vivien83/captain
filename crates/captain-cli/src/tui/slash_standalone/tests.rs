use super::*;

#[test]
fn unknown_message_includes_command_when_available() {
    assert_eq!(
        unknown_slash_message(crate::i18n::Lang::Fr, "/wat"),
        "Commande inconnue. Tapez /help. (/wat)"
    );
    assert_eq!(
        unknown_slash_message(crate::i18n::Lang::En, "/wat"),
        "Unknown command. Type /help. (/wat)"
    );
    assert_eq!(
        unknown_slash_message(crate::i18n::Lang::En, ""),
        "Unknown command. Type /help."
    );
}

#[test]
fn full_tui_navigation_points_to_captain_tui() {
    let msg = full_tui_navigation_message(crate::i18n::Lang::En, "/projects");

    assert!(msg.contains("/projects is a full TUI navigation command"));
    assert!(msg.contains("captain tui"));
    assert!(msg.contains("captain chat"));
}

#[test]
fn standalone_command_predicates_route_unavailable_groups() {
    assert!(is_attachment_or_voice_command("/image"));
    assert!(is_attachment_or_voice_command("/file"));
    assert!(is_attachment_or_voice_command("/voice"));
    assert!(!is_attachment_or_voice_command("/help"));

    assert!(is_feedback_command("/like"));
    assert!(is_feedback_command("/dislike"));
    assert!(!is_feedback_command("/status"));
}

#[test]
fn full_tui_navigation_predicate_covers_hubs_and_hides_frozen_surfaces() {
    for command in [
        "/home",
        "/projects",
        "/automation",
        "/learning",
        "/capabilities",
        "/channels",
        "/settings",
    ] {
        assert!(is_full_tui_navigation_command(command), "{command}");
    }
    for command in ["/connections", "/extensions", "/peers", "/comms", "/hands"] {
        assert!(!is_full_tui_navigation_command(command), "{command}");
    }
    assert!(!is_full_tui_navigation_command("/status"));
    assert!(!is_full_tui_navigation_command("/unknown"));
}

#[test]
fn attachments_and_feedback_messages_are_standalone_specific() {
    assert!(attachments_voice_message(crate::i18n::Lang::En).contains("standalone `captain chat`"));
    assert!(feedback_unavailable_message(crate::i18n::Lang::En).contains("does not persist"));
    assert!(attachments_voice_message(crate::i18n::Lang::Fr).contains("captain tui"));
    assert!(feedback_unavailable_message(crate::i18n::Lang::Fr).contains("feedback"));
}

#[test]
fn runtime_status_messages_preserve_hermes_standalone_text() {
    assert_eq!(stream_error_message("boom"), "Error: boom");
    assert_eq!(no_active_connection_message(), "No active connection");
    assert_eq!(
        daemon_agent_spawn_failed_message("offline"),
        "Failed to spawn agent: offline"
    );
    assert_eq!(
        spawning_agent_message("assistant"),
        "Spawning 'assistant' agent\u{2026}"
    );
    assert_eq!(
        no_agent_templates_message(),
        "No agent templates found. Run `captain init`."
    );
    assert_eq!(
        invalid_template_message("assistant", "bad toml"),
        "Invalid template 'assistant': bad toml"
    );
    assert_eq!(
        inprocess_agent_spawn_failed_message("capacity"),
        "Spawn failed: capacity"
    );
}
