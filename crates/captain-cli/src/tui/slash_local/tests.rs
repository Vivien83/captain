use super::*;

fn chat_with_identity() -> ChatState {
    let mut chat = ChatState::new();
    chat.agent_name = "Captain".to_string();
    chat.model_label = "openai/gpt-5".to_string();
    chat.mode_label = "daemon".to_string();
    chat
}

#[test]
fn copy_target_accepts_response_command_and_french_aliases() {
    assert_eq!(copy_target(""), Ok(CopyTarget::Response));
    assert_eq!(copy_target("response"), Ok(CopyTarget::Response));
    assert_eq!(copy_target("réponse"), Ok(CopyTarget::Response));
    assert_eq!(copy_target("command"), Ok(CopyTarget::Command));
    assert_eq!(copy_target("CMD"), Ok(CopyTarget::Command));
    assert_eq!(copy_target("commande"), Ok(CopyTarget::Command));
    assert!(copy_target("agent").is_err());
}

#[test]
fn copy_target_text_preserves_full_tui_and_standalone_labels() {
    let command_fr = copy_target_text(CopyTarget::Command, crate::i18n::Lang::Fr);
    assert_eq!(command_fr.label, "Commande");
    assert_eq!(
        command_fr.empty_message,
        "Aucune commande tool-call à copier."
    );

    let response_en = copy_target_text(CopyTarget::Response, crate::i18n::Lang::En);
    assert_eq!(response_en.label, "Response");
    assert_eq!(response_en.empty_message, "No response to copy.");
}

#[test]
fn copy_usage_message_keeps_surface_spacing() {
    assert_eq!(
        copy_usage_message(crate::i18n::Lang::Fr, CopyUsageSurface::FullTui),
        "Usage: /copy ou /copy command"
    );
    assert_eq!(
        copy_usage_message(crate::i18n::Lang::Fr, CopyUsageSurface::StandaloneChat),
        "Usage : /copy ou /copy command"
    );
    assert_eq!(
        copy_usage_message(crate::i18n::Lang::En, CopyUsageSurface::StandaloneChat),
        "Usage: /copy or /copy command"
    );
}

#[test]
fn copy_status_messages_preserve_hermes_surface_text() {
    assert_eq!(
        copy_success_message(CopyStatusSurface::FullTui, "Réponse", 12),
        "Réponse copiée dans le clipboard (12 caractères)."
    );
    assert_eq!(
        copy_failure_message(CopyStatusSurface::FullTui, "boom"),
        "Échec copie clipboard: boom"
    );
    assert_eq!(
        copy_success_message(CopyStatusSurface::StandaloneChat, "Command", 7),
        "Command copied to clipboard (7 chars)."
    );
    assert_eq!(
        copy_failure_message(CopyStatusSurface::StandaloneChat, "boom"),
        "Clipboard copy failed: boom"
    );
}

#[test]
fn mouse_capture_target_keeps_hermes_toggle_and_captain_french_aliases() {
    assert_eq!(mouse_capture_target("", false), Some(true));
    assert_eq!(mouse_capture_target("toggle", true), Some(false));
    assert_eq!(mouse_capture_target("on", false), Some(true));
    assert_eq!(mouse_capture_target("oui", false), Some(true));
    assert_eq!(mouse_capture_target("off", true), Some(false));
    assert_eq!(mouse_capture_target("non", true), Some(false));
    assert_eq!(mouse_capture_target("maybe", true), None);
}

#[test]
fn mouse_messages_preserve_full_tui_hermes_text() {
    assert_eq!(
        mouse_enabled_message(crate::i18n::Lang::Fr, MouseMessageSurface::FullTui),
        "Mode souris activé: clics tool calls + molette. Pour sélectionner/copier, utilise `/mouse off`."
    );
    assert_eq!(
        mouse_disabled_message(crate::i18n::Lang::Fr),
        "Mode souris désactivé: sélection native + clic droit copier disponibles."
    );
    assert_eq!(
        mouse_error_message(crate::i18n::Lang::Fr, MouseMessageSurface::FullTui, "boom"),
        "Échec changement mode souris: boom"
    );
    assert_eq!(
        mouse_usage_message(crate::i18n::Lang::Fr, MouseMessageSurface::FullTui),
        "Usage: /mouse, /mouse on, /mouse off"
    );
}

#[test]
fn mouse_messages_preserve_standalone_localized_text() {
    assert_eq!(
        mouse_enabled_message(crate::i18n::Lang::Fr, MouseMessageSurface::StandaloneChat),
        "Mode souris activé: clics tool calls + scroll TUI. Utilise `/mouse off` pour la sélection native."
    );
    assert_eq!(
        mouse_enabled_message(crate::i18n::Lang::En, MouseMessageSurface::StandaloneChat),
        "Mouse mode enabled: tool-call clicks + TUI scrolling. Use `/mouse off` for native selection."
    );
    assert_eq!(
        mouse_error_message(
            crate::i18n::Lang::Fr,
            MouseMessageSurface::StandaloneChat,
            "boom"
        ),
        "Échec changement mode souris : boom"
    );
    assert_eq!(
        mouse_usage_message(crate::i18n::Lang::Fr, MouseMessageSurface::StandaloneChat),
        "Usage : /mouse, /mouse on, /mouse off"
    );
}

#[test]
fn queue_message_numbers_staged_messages() {
    let empty: Vec<String> = Vec::new();
    assert_eq!(queue_message(&empty, "empty", "queued"), "empty");

    let staged = vec!["first".to_string(), "second".to_string()];
    assert_eq!(
        queue_message(&staged, "empty", "queued"),
        "queued\n  1. first\n  2. second"
    );
}

#[test]
fn queue_message_for_lang_preserves_hermes_i18n_text() {
    assert_eq!(
        queue_message_for_lang(&[], Lang::Fr),
        "La file d'envoi est vide."
    );
    assert_eq!(
        queue_message_for_lang(&["hello".to_string()], Lang::En),
        "Queued messages (auto-sent after the current stream):\n  1. hello"
    );
}

#[test]
fn undo_and_clear_messages_use_shared_i18n_keys() {
    assert_eq!(
        undo_result_message(true, Lang::Fr),
        "Dernier échange annulé."
    );
    assert_eq!(undo_result_message(false, Lang::Fr), "Rien à annuler.");
    assert_eq!(undo_result_message(true, Lang::En), "Last exchange undone.");
    assert_eq!(clear_message(Lang::Fr), "Historique effacé.");
    assert_eq!(clear_message(Lang::En), "Chat history cleared.");
}

#[test]
fn voice_record_secs_defaults_to_five() {
    assert_eq!(voice_record_secs("12"), 12);
    assert_eq!(voice_record_secs("bad"), 5);
    assert_eq!(voice_record_secs(""), 5);
}

#[test]
fn voice_recording_message_preserves_hermes_text() {
    assert_eq!(
        voice_recording_message(7),
        "🎙 Enregistrement 7s en cours..."
    );
}

#[test]
fn voice_completion_messages_preserve_hermes_text() {
    assert_eq!(
        voice_uploading_message("/tmp/captain-voice.wav"),
        "📤 Envoi de l'audio: /tmp/captain-voice.wav"
    );
    assert_eq!(
        voice_error_message("micro indisponible"),
        "🎙 micro indisponible"
    );
}

#[test]
fn undo_last_exchange_drops_until_latest_user_message() {
    let mut chat = ChatState::new();
    chat.push_message(Role::User, "u1".to_string());
    chat.push_message(Role::Agent, "a1".to_string());
    chat.push_message(Role::System, "note".to_string());

    assert!(undo_last_exchange(&mut chat));
    assert!(chat.messages.is_empty());
}

#[test]
fn undo_last_exchange_reports_no_user_message() {
    let mut chat = ChatState::new();
    chat.push_message(Role::System, "note".to_string());

    assert!(!undo_last_exchange(&mut chat));
    assert!(chat.messages.is_empty());
}

#[test]
fn clear_chat_preserving_identity_resets_history_and_keeps_labels() {
    let mut chat = chat_with_identity();
    chat.push_message(Role::User, "hello".to_string());
    chat.session_input_tokens = 10;
    chat.last_cost_usd = Some(0.1);

    clear_chat_preserving_identity(&mut chat);

    assert_eq!(chat.agent_name, "Captain");
    assert_eq!(chat.model_label, "openai/gpt-5");
    assert_eq!(chat.mode_label, "daemon");
    assert!(chat.messages.is_empty());
    assert_eq!(chat.session_input_tokens, 0);
    assert_eq!(chat.last_cost_usd, None);
}
