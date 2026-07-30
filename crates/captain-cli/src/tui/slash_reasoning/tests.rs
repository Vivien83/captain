use super::*;
use captain_types::reasoning::{ModelReasoningCapabilities, ReasoningEffortOption};

#[test]
fn parser_distinguishes_show_auto_and_explicit_effort() {
    assert_eq!(
        command_for("/reasoning", ""),
        Some(Ok(ReasoningCommand::Show))
    );
    assert_eq!(
        command_for("/reasoning", "auto"),
        Some(Ok(ReasoningCommand::Set(None)))
    );
    assert_eq!(
        command_for("/reasoning", "XHIGH"),
        Some(Ok(ReasoningCommand::Set(Some("xhigh".to_string()))))
    );
    assert_eq!(
        command_for("/reasoning", "ULTRA"),
        Some(Ok(ReasoningCommand::Set(Some("ultra".to_string()))))
    );
    assert!(command_for("/reasoning", "high extra").unwrap().is_err());
    assert!(command_for("/think", "high").is_none());
}

#[test]
fn status_message_separates_auto_from_effective_model_default() {
    let capabilities = ModelReasoningCapabilities {
        default_effort: Some("low".parse().unwrap()),
        supported_efforts: vec![ReasoningEffortOption {
            effort: "low".parse().unwrap(),
            description: None,
        }],
        reported_by_provider: true,
    };
    let status = AgentReasoningStatus {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
        supported: true,
        configured_effort: None,
        effective_effort: capabilities.default_effort,
        source: ReasoningSelectionSource::ModelDefault,
        override_valid: true,
        options: capabilities.supported_efforts,
        reported_by_provider: true,
    };

    let message = status_message(&status);
    assert!(message.contains("sélection `auto` → effectif `low`"));
    assert!(message.contains("défaut du modèle"));
    assert!(message.contains("`auto` omet l'override"));
    assert!(message.contains("`none`"));
}

#[test]
fn status_message_never_prints_an_empty_options_sentence() {
    let status = AgentReasoningStatus {
        provider: "codex".to_string(),
        model: "future-model".to_string(),
        supported: true,
        configured_effort: None,
        effective_effort: None,
        source: ReasoningSelectionSource::ProviderDefault,
        override_valid: true,
        options: Vec::new(),
        reported_by_provider: false,
    };

    let message = status_message(&status);
    assert!(
        message.contains("aucun niveau explicite publié"),
        "{message}"
    );
    assert!(!message.contains("Niveaux disponibles : ."), "{message}");
}

#[test]
fn status_message_preserves_every_provider_reported_level_including_ultra() {
    let status = AgentReasoningStatus {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
        supported: true,
        configured_effort: Some("ultra".parse().unwrap()),
        effective_effort: Some("ultra".parse().unwrap()),
        source: ReasoningSelectionSource::AgentOverride,
        override_valid: true,
        options: ["low", "medium", "high", "xhigh", "max", "ultra"]
            .into_iter()
            .map(|effort| ReasoningEffortOption {
                effort: effort.parse().unwrap(),
                description: None,
            })
            .collect(),
        reported_by_provider: true,
    };

    let message = status_message(&status);
    assert!(message.contains("sélection `ultra` → effectif `ultra`"));
    assert!(message.contains("low, medium, high, xhigh, max, ultra"));
    assert!(message.contains("effort modèle `max`"));
    assert!(message.contains("délégation proactive uniquement sur l'agent racine"));
}
