use super::CaptainKernel;
use captain_memory::MemorySubstrate;
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, SessionId};
use captain_types::config::{DefaultModelConfig, KernelConfig};

fn test_agent_entry(id: AgentId, manifest: AgentManifest) -> AgentEntry {
    AgentEntry {
        id,
        name: manifest.name.clone(),
        tags: manifest.tags.clone(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent: None,
        children: vec![],
        session_id: SessionId::new(),
        identity: Default::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        mission: None,
        mission_set_at: None,
        autoscale: None,
        last_scale_event: None,
    }
}

#[test]
fn direct_model_switch_detects_codex_53_and_54_variants() {
    let tmp = tempfile::tempdir().unwrap();
    let config = KernelConfig {
        home_dir: tmp.path().join("captain-kernel-codex-detect"),
        data_dir: tmp.path().join("captain-kernel-codex-detect-data"),
        ..KernelConfig::default()
    };
    let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

    assert_eq!(
        kernel.detect_direct_model_switch_target("mets gpt-5.3-codex par defaut"),
        Some(("codex".to_string(), "gpt-5.3-codex".to_string()))
    );
    assert_eq!(
        kernel.detect_direct_model_switch_target("mets codex gpt 5.3 spark par defaut"),
        Some(("codex".to_string(), "gpt-5.3-codex-spark".to_string()))
    );
    assert_eq!(
        kernel.detect_direct_model_switch_target("bascule sur codex 5.4 mini"),
        Some(("codex".to_string(), "gpt-5.4-mini".to_string()))
    );
    assert_eq!(
        kernel.detect_direct_model_switch_target("Basculer vers codex/gpt-5.5"),
        Some(("codex".to_string(), "gpt-5.5".to_string()))
    );

    kernel.shutdown();
}

#[test]
fn principal_model_switch_persists_global_default_model() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("captain-kernel-model-switch-config");
    std::fs::create_dir_all(&home_dir).unwrap();
    std::fs::write(
        home_dir.join("config.toml"),
        r#"[default_model]
provider = "anthropic"
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://old.example.invalid"

[workspace]
extra_paths = []
"#,
    )
    .unwrap();
    let config = KernelConfig {
        home_dir: home_dir.clone(),
        data_dir: home_dir.join("data"),
        ..KernelConfig::default()
    };
    let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

    kernel
        .persist_principal_default_model_switch("codex", "gpt-5.5")
        .expect("persist default model");

    let raw = std::fs::read_to_string(home_dir.join("config.toml")).unwrap();
    let parsed: toml::Value = raw.parse().unwrap();
    let default_model = parsed
        .get("default_model")
        .and_then(|v| v.as_table())
        .expect("default_model table");
    assert_eq!(
        default_model.get("provider").and_then(|v| v.as_str()),
        Some("codex")
    );
    assert_eq!(
        default_model.get("model").and_then(|v| v.as_str()),
        Some("gpt-5.5")
    );
    assert_eq!(
        default_model.get("api_key_env").and_then(|v| v.as_str()),
        Some("")
    );
    assert!(
        !default_model.contains_key("base_url"),
        "old provider base_url must not leak into Codex OAuth config"
    );
    assert_eq!(kernel.effective_default_model().provider, "codex");
    assert_eq!(kernel.effective_default_model().model, "gpt-5.5");

    kernel.shutdown();
}

#[test]
fn principal_restore_reconciles_global_default_model() {
    let mut manifest = AgentManifest::default();
    manifest.name = "unnamed".to_string();
    manifest.description = String::new();
    manifest.model.provider = "anthropic".to_string();
    manifest.model.model = "claude-sonnet-4-20250514".to_string();
    manifest.model.api_key_env = Some("ANTHROPIC_API_KEY".to_string());
    manifest.model.base_url = Some("https://api.anthropic.com".to_string());
    let mut entry = test_agent_entry(AgentId::new(), manifest);
    entry.name = "captain".to_string();

    let default_model = DefaultModelConfig {
        provider: "codex".to_string(),
        model: "gpt-5.5".to_string(),
        api_key_env: String::new(),
        base_url: None,
    };

    assert!(CaptainKernel::reconcile_principal_agent_with_default_model(
        &mut entry,
        &default_model
    ));
    assert_eq!(entry.manifest.name, "captain");
    assert_eq!(entry.manifest.description, "Captain — principal agent");
    assert_eq!(entry.manifest.model.provider, "codex");
    assert_eq!(entry.manifest.model.model, "gpt-5.5");
    assert_eq!(entry.manifest.model.api_key_env, None);
    assert_eq!(entry.manifest.model.base_url, None);
}

#[test]
fn boot_repairs_poisoned_principal_agent_from_template_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(home_dir.join("agents/captain")).unwrap();
    std::fs::write(
        home_dir.join("agents/captain/agent.toml"),
        r#"[agent]
name = "captain"
description = "Captain — principal agent"

[personality]
system_prompt_file = "captain.md"
"#,
    )
    .unwrap();

    let db_path = data_dir.join("captain.db");
    let memory = MemorySubstrate::open(&db_path, 0.1).unwrap();
    let mut manifest = AgentManifest::default();
    manifest.name = "unnamed".to_string();
    manifest.description = String::new();
    manifest.model.provider = "anthropic".to_string();
    manifest.model.model = "claude-sonnet-4-20250514".to_string();
    manifest.model.api_key_env = Some("ANTHROPIC_API_KEY".to_string());
    let mut poisoned = test_agent_entry(AgentId::new(), manifest);
    poisoned.name = "captain".to_string();
    memory.save_agent(&poisoned).unwrap();
    drop(memory);

    let config = KernelConfig {
        home_dir: home_dir.clone(),
        data_dir: data_dir.clone(),
        default_model: DefaultModelConfig {
            provider: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            api_key_env: String::new(),
            base_url: None,
        },
        ..KernelConfig::default()
    };

    let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
    let captain = kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name == "captain")
        .expect("captain restored");

    assert_eq!(captain.manifest.name, "captain");
    assert_eq!(captain.manifest.model.provider, "codex");
    assert_eq!(captain.manifest.model.model, "gpt-5.5");
    assert_eq!(captain.manifest.model.api_key_env, None);

    let persisted = kernel.memory.load_all_agents().unwrap();
    let persisted_captain = persisted
        .iter()
        .find(|entry| entry.name == "captain")
        .expect("captain persisted");
    assert_eq!(persisted_captain.manifest.model.provider, "codex");
    assert_eq!(persisted_captain.manifest.model.model, "gpt-5.5");

    kernel.shutdown();
}
