use std::sync::{Mutex, OnceLock};

use crate::commands::init::write_config_if_missing;
use crate::commands::setup_access::setup_bootstrap_access;
use crate::commands::setup_options::{
    setup_stt_state, setup_telegram_state, setup_tts_state, SetupOptionState,
};
use crate::commands::setup_support::setup_parse_bool;
use crate::commands::setup_surface::{
    setup_configure_product_surface, setup_normalize_public_url, setup_public_host,
};
use crate::commands::voice_install::{ensure_python_venv, find_python, python_module_importable};

fn setup_env_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn restore_env_var(name: &str, previous: Option<String>) {
    if let Some(value) = previous {
        std::env::set_var(name, value);
    } else {
        std::env::remove_var(name);
    }
}

#[test]
fn test_ensure_python_venv_creates_pip_ready_environment() {
    let Some(python) = find_python() else {
        return;
    };
    if !python_module_importable(&python, "venv") || !python_module_importable(&python, "ensurepip")
    {
        return;
    }
    let venv = std::env::temp_dir().join(format!(
        "captain-voice-venv-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&venv);
    let venv_python =
        ensure_python_venv(&python, &venv, false, "test voice venv").expect("venv ready");
    assert!(python_module_importable(&venv_python, "pip"));
    let _ = std::fs::remove_dir_all(&venv);
}

#[test]
fn test_doctor_skill_registry_loads_bundled() {
    let skills_dir = std::env::temp_dir().join("captain-doctor-test-skills");
    let mut skill_reg = captain_skills::registry::SkillRegistry::new(skills_dir);
    let count = skill_reg.load_bundled();
    assert!(count > 0, "Should load bundled skills");
    assert_eq!(skill_reg.count(), count);
}

#[test]
fn test_doctor_extension_registry_loads_bundled() {
    let tmp = std::env::temp_dir().join("captain-doctor-test-ext");
    let _ = std::fs::create_dir_all(&tmp);
    let mut ext_reg = captain_extensions::registry::IntegrationRegistry::new(&tmp);
    let count = ext_reg.load_bundled();
    assert!(count > 0, "Should load bundled integration templates");
    assert_eq!(ext_reg.template_count(), count);
}

#[test]
fn test_doctor_config_deser_default() {
    let config = captain_types::config::KernelConfig::default();
    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed: captain_types::config::KernelConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.api_listen, config.api_listen);
}

#[test]
fn test_doctor_config_include_field() {
    let config_toml = r#"
api_listen = "127.0.0.1:4200"
include = ["providers.toml", "agents.toml"]

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
    let config: captain_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
    assert_eq!(config.include.len(), 2);
    assert_eq!(config.include[0], "providers.toml");
    assert_eq!(config.include[1], "agents.toml");
}

#[test]
fn test_setup_public_url_normalization() {
    assert_eq!(
        setup_normalize_public_url("captain.example.com"),
        Some("https://captain.example.com".to_string())
    );
    assert_eq!(
        setup_normalize_public_url("http://captain.example.com/"),
        Some("http://captain.example.com".to_string())
    );
    assert_eq!(setup_normalize_public_url("none"), None);
    assert_eq!(
        setup_public_host("https://captain.example.com/path"),
        Some("captain.example.com".to_string())
    );
}

#[test]
fn test_setup_vps_without_domain_exposes_direct_ip_terminal() {
    let _guard = setup_env_test_lock().lock().unwrap();
    let previous_public_url = std::env::var("CAPTAIN_PUBLIC_URL").ok();
    let previous_domain = std::env::var("CAPTAIN_DOMAIN").ok();
    let previous_ip = std::env::var("CAPTAIN_PUBLIC_IP").ok();
    let previous_reverse_proxy = std::env::var("CAPTAIN_REVERSE_PROXY").ok();
    let previous_shell = std::env::var("CAPTAIN_WEB_TERMINAL_SHELL").ok();
    std::env::remove_var("CAPTAIN_PUBLIC_URL");
    std::env::remove_var("CAPTAIN_DOMAIN");
    std::env::set_var("CAPTAIN_PUBLIC_IP", "203.0.113.10");
    std::env::remove_var("CAPTAIN_REVERSE_PROXY");
    std::env::remove_var("CAPTAIN_WEB_TERMINAL_SHELL");

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write_config_if_missing(home, "groq", "llama-3.3-70b-versatile", "GROQ_API_KEY");
    let outcome = setup_configure_product_surface(home, "vps", None, false).unwrap();

    assert_eq!(outcome.public_url, None);
    assert_eq!(outcome.api_listen, "0.0.0.0:50051");
    assert_eq!(
        outcome.direct_url,
        Some("http://203.0.113.10:50051/terminal".to_string())
    );

    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(raw.contains("api_listen = \"0.0.0.0:50051\""));
    assert!(!raw.contains("public_url"));

    restore_env_var("CAPTAIN_PUBLIC_URL", previous_public_url);
    restore_env_var("CAPTAIN_DOMAIN", previous_domain);
    restore_env_var("CAPTAIN_PUBLIC_IP", previous_ip);
    restore_env_var("CAPTAIN_REVERSE_PROXY", previous_reverse_proxy);
    restore_env_var("CAPTAIN_WEB_TERMINAL_SHELL", previous_shell);
}

#[test]
fn test_setup_preserves_custom_api_listen_on_rerun() {
    let _guard = setup_env_test_lock().lock().unwrap();
    let previous_public_url = std::env::var("CAPTAIN_PUBLIC_URL").ok();
    let previous_domain = std::env::var("CAPTAIN_DOMAIN").ok();
    let previous_reverse_proxy = std::env::var("CAPTAIN_REVERSE_PROXY").ok();
    let previous_shell = std::env::var("CAPTAIN_WEB_TERMINAL_SHELL").ok();
    std::env::remove_var("CAPTAIN_PUBLIC_URL");
    std::env::remove_var("CAPTAIN_DOMAIN");
    std::env::remove_var("CAPTAIN_REVERSE_PROXY");
    std::env::remove_var("CAPTAIN_WEB_TERMINAL_SHELL");

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write_config_if_missing(home, "groq", "llama-3.3-70b-versatile", "GROQ_API_KEY");
    std::fs::write(
        home.join("config.toml"),
        std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .replace(
                "api_listen = \"127.0.0.1:50051\"",
                "api_listen = \"127.0.0.1:50098\"",
            ),
    )
    .unwrap();

    let outcome = setup_configure_product_surface(home, "core", None, false).unwrap();

    assert_eq!(outcome.api_listen, "127.0.0.1:50098");
    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(raw.contains("api_listen = \"127.0.0.1:50098\""));

    restore_env_var("CAPTAIN_PUBLIC_URL", previous_public_url);
    restore_env_var("CAPTAIN_DOMAIN", previous_domain);
    restore_env_var("CAPTAIN_REVERSE_PROXY", previous_reverse_proxy);
    restore_env_var("CAPTAIN_WEB_TERMINAL_SHELL", previous_shell);
}

#[test]
fn test_setup_vps_with_domain_keeps_api_local_and_writes_caddyfile() {
    let _guard = setup_env_test_lock().lock().unwrap();
    let previous_public_url = std::env::var("CAPTAIN_PUBLIC_URL").ok();
    let previous_domain = std::env::var("CAPTAIN_DOMAIN").ok();
    let previous_reverse_proxy = std::env::var("CAPTAIN_REVERSE_PROXY").ok();
    let previous_shell = std::env::var("CAPTAIN_WEB_TERMINAL_SHELL").ok();
    std::env::remove_var("CAPTAIN_PUBLIC_URL");
    std::env::remove_var("CAPTAIN_DOMAIN");
    std::env::remove_var("CAPTAIN_REVERSE_PROXY");
    std::env::remove_var("CAPTAIN_WEB_TERMINAL_SHELL");

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write_config_if_missing(home, "groq", "llama-3.3-70b-versatile", "GROQ_API_KEY");
    let answers: toml::Value = r#"
        [deployment]
        public_url = "captain.example.com"
        reverse_proxy = "caddy"
    "#
    .parse()
    .unwrap();

    let outcome = setup_configure_product_surface(home, "vps", Some(&answers), false).unwrap();

    assert_eq!(
        outcome.public_url,
        Some("https://captain.example.com".to_string())
    );
    assert_eq!(outcome.direct_url, None);
    assert_eq!(outcome.api_listen, "127.0.0.1:50051");
    assert!(!outcome.shell_enabled);
    let caddyfile_path = outcome.caddyfile_path.expect("caddyfile path");
    let caddyfile = std::fs::read_to_string(caddyfile_path).unwrap();
    assert!(caddyfile.contains("captain.example.com"));
    assert!(caddyfile.contains("reverse_proxy 127.0.0.1:50051"));

    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(raw.contains("api_listen = \"127.0.0.1:50051\""));
    assert!(raw.contains("public_url = \"https://captain.example.com\""));

    restore_env_var("CAPTAIN_PUBLIC_URL", previous_public_url);
    restore_env_var("CAPTAIN_DOMAIN", previous_domain);
    restore_env_var("CAPTAIN_REVERSE_PROXY", previous_reverse_proxy);
    restore_env_var("CAPTAIN_WEB_TERMINAL_SHELL", previous_shell);
}

#[test]
fn test_setup_bool_parser() {
    assert_eq!(setup_parse_bool("yes"), Some(true));
    assert_eq!(setup_parse_bool("OFF"), Some(false));
    assert_eq!(setup_parse_bool("maybe"), None);
}

#[test]
fn test_setup_optional_state_reproposes_partials() {
    let _guard = setup_env_test_lock().lock().unwrap();
    let previous_telegram = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    let previous_groq = std::env::var("GROQ_API_KEY").ok();
    let previous_elevenlabs = std::env::var("ELEVENLABS_API_KEY").ok();
    std::env::remove_var("TELEGRAM_BOT_TOKEN");
    std::env::remove_var("GROQ_API_KEY");
    std::env::remove_var("ELEVENLABS_API_KEY");

    let empty: toml::Value = "".parse().unwrap();
    assert_eq!(
        setup_telegram_state(Some(&empty)),
        SetupOptionState::Missing
    );
    assert_eq!(setup_stt_state(Some(&empty)), SetupOptionState::Missing);
    assert_eq!(setup_tts_state(Some(&empty)), SetupOptionState::Missing);

    let partial: toml::Value = r#"
[channels.telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
default_chat_id = "123"

[media]
audio_provider = "groq"
audio_model = "whisper-large-v3-turbo"

[tts]
enabled = true
provider = "elevenlabs"
"#
    .parse()
    .unwrap();
    assert_eq!(
        setup_telegram_state(Some(&partial)),
        SetupOptionState::Partial
    );
    assert_eq!(setup_stt_state(Some(&partial)), SetupOptionState::Partial);
    assert_eq!(setup_tts_state(Some(&partial)), SetupOptionState::Partial);

    restore_env_var("TELEGRAM_BOT_TOKEN", previous_telegram);
    restore_env_var("GROQ_API_KEY", previous_groq);
    restore_env_var("ELEVENLABS_API_KEY", previous_elevenlabs);
}

#[test]
fn test_setup_optional_state_detects_complete_existing_config() {
    let _guard = setup_env_test_lock().lock().unwrap();
    let previous_telegram = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    let previous_groq = std::env::var("GROQ_API_KEY").ok();
    let previous_elevenlabs = std::env::var("ELEVENLABS_API_KEY").ok();
    std::env::set_var("TELEGRAM_BOT_TOKEN", "123456789:abcdefghijklmnopqrstuvwxyz");
    std::env::set_var("GROQ_API_KEY", "gsk_abcdefghijklmnopqrstuvwxyz");
    std::env::set_var("ELEVENLABS_API_KEY", "elevenlabs-key-123456");

    let complete: toml::Value = r#"
[channels.telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
default_chat_id = "123"

[media]
audio_provider = "groq"
audio_model = "whisper-large-v3-turbo"

[tts]
enabled = true
provider = "elevenlabs"

[tts.elevenlabs]
api_key_env = "ELEVENLABS_API_KEY"
"#
    .parse()
    .unwrap();
    assert_eq!(
        setup_telegram_state(Some(&complete)),
        SetupOptionState::Complete
    );
    assert_eq!(setup_stt_state(Some(&complete)), SetupOptionState::Complete);
    assert_eq!(setup_tts_state(Some(&complete)), SetupOptionState::Complete);

    restore_env_var("TELEGRAM_BOT_TOKEN", previous_telegram);
    restore_env_var("GROQ_API_KEY", previous_groq);
    restore_env_var("ELEVENLABS_API_KEY", previous_elevenlabs);
}

#[test]
fn test_setup_bootstrap_access_writes_auth_and_vaulted_api_key() {
    let _guard = setup_env_test_lock().lock().unwrap();
    let previous_api_key = std::env::var("CAPTAIN_API_KEY").ok();
    std::env::set_var(
        "CAPTAIN_API_KEY",
        "gsk_PROVIDER_KEY_MUST_NOT_BECOME_DAEMON_KEY",
    );

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write_config_if_missing(home, "groq", "llama-3.3-70b-versatile", "GROQ_API_KEY");
    let answers: toml::Value = r#"
[auth]
username = "test-admin"
password = "test-password"
"#
    .parse()
    .unwrap();

    let outcome = setup_bootstrap_access(home, Some(&answers), false).unwrap();
    assert_eq!(outcome.username, "test-admin");
    assert!(outcome.generated_password.is_none());
    assert!(outcome.generated_api_key);

    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(raw.contains("api_key = \"\""));
    assert!(!raw.contains("captain_api_"));
    let secrets = std::fs::read_to_string(home.join("secrets.env")).unwrap();
    assert!(secrets.contains("CAPTAIN_DAEMON_API_KEY=captain_api_"));
    assert!(!secrets.contains("gsk_PROVIDER_KEY"));
    assert!(raw.contains("[auth]"));
    assert!(raw.contains("enabled = true"));
    assert!(raw.contains("username = \"test-admin\""));
    assert!(raw.contains("password_hash = \""));

    restore_env_var("CAPTAIN_API_KEY", previous_api_key);
}

#[test]
fn test_doctor_exec_policy_field() {
    let config_toml = r#"
api_listen = "127.0.0.1:4200"

[exec_policy]
mode = "allowlist"
safe_bins = ["ls", "cat", "echo"]
timeout_secs = 30

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
    let config: captain_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
    assert_eq!(
        config.exec_policy.mode,
        captain_types::config::ExecSecurityMode::Allowlist
    );
    assert_eq!(config.exec_policy.safe_bins.len(), 3);
    assert_eq!(config.exec_policy.timeout_secs, 30);
}

#[test]
fn test_doctor_mcp_transport_validation() {
    let config_toml = r#"
api_listen = "127.0.0.1:4200"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "github"
timeout_secs = 30

[mcp_servers.transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
"#;
    let config: captain_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
    assert_eq!(config.mcp_servers.len(), 1);
    assert_eq!(config.mcp_servers[0].name, "github");
    match &config.mcp_servers[0].transport {
        captain_types::config::McpTransportEntry::Stdio { command, args } => {
            assert_eq!(command, "npx");
            assert_eq!(args.len(), 2);
        }
        _ => panic!("Expected Stdio transport"),
    }
}

#[test]
fn test_doctor_skill_injection_scan_clean() {
    let clean_content = "This is a normal skill prompt with helpful instructions.";
    let warnings = captain_skills::verify::SkillVerifier::scan_prompt_content(clean_content);
    assert!(warnings.is_empty(), "Clean content should have no warnings");
}

#[test]
fn test_doctor_hook_event_variants() {
    use captain_types::agent::HookEvent;
    let events = [
        HookEvent::BeforeToolCall,
        HookEvent::AfterToolCall,
        HookEvent::BeforePromptBuild,
        HookEvent::AgentLoopEnd,
    ];
    assert_eq!(events.len(), 4);
}
