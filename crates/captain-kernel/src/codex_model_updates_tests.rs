use super::*;
use captain_runtime::model_switch_pending::pending_model_switch_key;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use captain_types::model_catalog::ModelTier;

fn model(id: &str, display_name: &str) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: id.to_string(),
        display_name: display_name.to_string(),
        provider: "codex".to_string(),
        tier: ModelTier::Frontier,
        context_window: 272_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: true,
        supports_streaming: true,
        aliases: Vec::new(),
    }
}

#[test]
fn first_success_without_cache_establishes_a_silent_baseline() {
    let mut state = CodexModelUpdateState::default();
    let available = vec![model("codex/gpt-5.6", "GPT-5.6")];

    let new = reconcile_success(&mut state, &[], &available, "now");

    assert!(new.is_empty());
    assert!(state.pending.is_empty());
    assert!(state.known_model_ids.contains("codex/gpt-5.6"));
    assert!(state.baseline_ready);
}

#[test]
fn a_model_added_after_the_cached_baseline_is_pending_once() {
    let baseline = vec![model("codex/gpt-5.5", "GPT-5.5")];
    let available = vec![
        model("codex/gpt-5.6", "GPT-5.6"),
        model("codex/gpt-5.5", "GPT-5.5"),
    ];
    let mut state = CodexModelUpdateState::default();

    let first = reconcile_success(&mut state, &baseline, &available, "first");
    let second = reconcile_success(&mut state, &available, &available, "second");

    assert_eq!(first, vec!["codex/gpt-5.6"]);
    assert!(second.is_empty());
    assert_eq!(state.pending.len(), 1);
}

#[test]
fn keep_decision_resolves_pending_but_preserves_known_model() {
    let mut state = CodexModelUpdateState {
        baseline_ready: true,
        known_model_ids: BTreeSet::from(["codex/gpt-5.6".to_string()]),
        pending: vec![CodexModelUpdate {
            model_id: "codex/gpt-5.6".to_string(),
            display_name: "GPT-5.6".to_string(),
            discovered_at: "first".to_string(),
            telegram_notified_at: None,
        }],
        ..CodexModelUpdateState::default()
    };

    let resolved = resolve_pending_updates(&mut state, Some("codex/gpt-5.6"), "kept", "now", None);

    assert_eq!(resolved, vec!["codex/gpt-5.6"]);
    assert!(state.pending.is_empty());
    assert!(state.known_model_ids.contains("codex/gpt-5.6"));
    assert_eq!(state.recent_decisions[0].decision, "kept");
}

#[test]
fn keep_parser_requires_an_explicit_model_decision() {
    assert!(has_keep_intent("Garder le modèle actuel"));
    assert!(has_keep_intent("Oui, garder le modèle actuel."));
    assert!(has_keep_intent("Keep the current model"));
    assert!(!has_keep_intent("Garde ce document dans la mémoire"));
    assert!(!has_keep_intent(
        "Est-ce que garder le modèle actuel est préférable ?"
    ));
}

#[test]
fn codex_model_ids_are_canonical_across_provider_spellings() {
    assert_eq!(normalize_codex_model_id("GPT-5.6"), "codex/gpt-5.6");
    assert_eq!(
        normalize_codex_model_id("OpenAI-Codex/GPT-5.6"),
        "codex/gpt-5.6"
    );
}

#[test]
fn conversational_keep_is_durable_and_cancels_a_prepared_switch() {
    let home = tempfile::tempdir().unwrap();
    let config = KernelConfig {
        home_dir: home.path().join("home"),
        data_dir: home.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        language: "fr".to_string(),
        ..KernelConfig::default()
    };
    let kernel = CaptainKernel::boot_with_config(config).unwrap();
    let captain = kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name == "captain")
        .unwrap();
    kernel
        .mutate_codex_model_update_state(|state| {
            state.baseline_ready = true;
            state.known_model_ids.insert("codex/gpt-5.6".to_string());
            state.pending.push(CodexModelUpdate {
                model_id: "codex/gpt-5.6".to_string(),
                display_name: "GPT-5.6".to_string(),
                discovered_at: "first".to_string(),
                telegram_notified_at: None,
            });
        })
        .unwrap();
    let switch_key = pending_model_switch_key(&captain.id.to_string());
    kernel
        .memory
        .structured_set(
            shared_memory_agent_id(),
            &switch_key,
            serde_json::json!({"status": "pending"}),
        )
        .unwrap();

    let response = kernel
        .consume_codex_model_update_keep_request(captain.id, "Garder le modèle actuel")
        .unwrap()
        .unwrap();

    assert!(response.contains("conserve le modèle actuel"));
    let snapshot = kernel.codex_model_update_snapshot().unwrap();
    assert!(snapshot.pending.is_empty());
    assert_eq!(snapshot.recent_decisions[0].decision, "kept");
    assert!(kernel
        .memory
        .structured_get(shared_memory_agent_id(), &switch_key)
        .unwrap()
        .is_none());
    kernel.shutdown();
}
