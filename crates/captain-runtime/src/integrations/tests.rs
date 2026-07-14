use super::*;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn registry_resolves_known_integrations() {
    assert!(get_integration("telegram").is_some());
    assert!(get_integration("tts_elevenlabs").is_some());
    assert!(get_integration("tts_openai").is_some());
    assert!(get_integration("stt_whisper").is_some());
    assert!(get_integration("unknown").is_none());
}

#[test]
fn list_integrations_includes_all_registered() {
    let names = list_integrations();
    assert!(names.contains(&"telegram"));
    assert!(names.contains(&"tts_elevenlabs"));
    assert!(names.contains(&"tts_openai"));
    assert!(names.contains(&"stt_whisper"));
}

#[test]
fn apply_config_patch_preserves_comments() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# Captain config\n# Edited by hand — keep me\nport = 50051\n\n[memory]\n# embedding choice\nbackend = \"mempalace\"\n",
    )
    .unwrap();

    let patches = vec![ConfigPatch {
        path: vec!["channels".into(), "telegram".into()],
        key: "default_chat_id".into(),
        value: toml_edit::value("123456"),
    }];

    apply_config_patch(&path, &patches).unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("# Captain config"));
    assert!(written.contains("# Edited by hand — keep me"));
    assert!(written.contains("# embedding choice"));
    assert!(written.contains("[channels.telegram]"));
    assert!(written.contains("default_chat_id = \"123456\""));
    assert!(written.contains("port = 50051"));
    assert!(written.contains("backend = \"mempalace\""));
}

#[test]
fn backup_config_creates_timestamped_bak() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "port = 50051\n").unwrap();
    let bak = backup_config(&path).unwrap().expect("bak path returned");
    assert!(bak.exists());
    assert!(bak.to_string_lossy().contains(".bak."));
    assert_eq!(std::fs::read_to_string(&bak).unwrap(), "port = 50051\n");
}

#[test]
fn backup_config_returns_none_when_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("does-not-exist.toml");
    let res = backup_config(&path).unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn setup_integration_notify_skipped_on_validate_failure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "port = 50051\n").unwrap();

    let setter = |_k: &str, _v: &str| -> Result<(), String> { Ok(()) };
    let notify = |_n: &str| {
        panic!("notify must NOT fire when setup fails");
    };
    let bad = serde_json::json!({"bot_token": "no_colon", "default_chat_id": "1"});
    let res = setup_integration("telegram", &bad, &path, setter, false, Some(&notify)).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn setup_integration_invalid_creds_no_side_effects() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    let original = "# pre-existing\nport = 50051\n";
    std::fs::write(&path, original).unwrap();

    let setter = |_k: &str, _v: &str| -> Result<(), String> {
        panic!("vault must not be touched when validate fails");
    };

    let bad = json!({"bot_token": "not_valid", "default_chat_id": "1"});
    let res = setup_integration("telegram", &bad, &path, setter, false, None).await;
    assert!(res.is_err(), "validate should reject");

    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "only the config.toml should exist");
}

#[tokio::test]
async fn setup_integration_exports_env_vars() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "port = 50051\n").unwrap();

    let key_value = "sk_R33_TEST_ELEVENLABS_KEY_xxxxx_unique";

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    let store: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let store_c = store.clone();
    let setter = move |k: &str, v: &str| -> Result<(), String> {
        store_c.lock().unwrap().insert(k.to_string(), v.to_string());
        Ok(())
    };

    let creds = serde_json::json!({"api_key": key_value});
    let outcome = setup_integration("tts_elevenlabs", &creds, &path, setter, false, None)
        .await
        .expect("setup must succeed");

    assert_eq!(outcome.env_exports, vec!["ELEVENLABS_API_KEY".to_string()]);

    let s = store.lock().unwrap();
    assert_eq!(
        s.get("integration:tts_elevenlabs:api_key")
            .map(String::as_str),
        Some(key_value)
    );
    assert_eq!(
        s.get("ELEVENLABS_API_KEY").map(String::as_str),
        Some(key_value)
    );
    drop(s);

    assert_eq!(
        std::env::var("ELEVENLABS_API_KEY").as_deref(),
        Ok(key_value)
    );

    std::env::remove_var("ELEVENLABS_API_KEY");
}

#[tokio::test]
async fn setup_integration_telegram_writes_vault_and_config() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# header comment\nport = 50051\n").unwrap();

    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    let store: Rc<RefCell<HashMap<String, String>>> = Rc::new(RefCell::new(HashMap::new()));
    let store_c = store.clone();
    let setter = move |k: &str, v: &str| -> Result<(), String> {
        store_c.borrow_mut().insert(k.to_string(), v.to_string());
        Ok(())
    };

    let creds = json!({
        "bot_token": "1234567890:AAFakeSecretSegmentForTesting",
        "default_chat_id": "987654",
        "allowed_users": ["111", "222"]
    });

    use std::sync::{Arc, Mutex};
    let notified: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let notified_c = notified.clone();
    let notify = move |n: &str| {
        notified_c.lock().unwrap().push(n.to_string());
    };

    let outcome = setup_integration("telegram", &creds, &path, setter, false, Some(&notify))
        .await
        .expect("setup should succeed");

    assert_eq!(outcome.integration, "telegram");
    assert!(outcome.backup_path.is_some());
    assert!(outcome
        .vault_keys
        .contains(&"integration:telegram:bot_token".to_string()));
    assert_eq!(outcome.env_exports, vec!["TELEGRAM_BOT_TOKEN".to_string()]);
    assert!(outcome.test_message.is_none());
    assert_eq!(
        notified.lock().unwrap().as_slice(),
        &["telegram".to_string()]
    );

    let s = store.borrow();
    assert_eq!(
        s.get("integration:telegram:bot_token").map(String::as_str),
        Some("1234567890:AAFakeSecretSegmentForTesting"),
    );
    assert_eq!(
        s.get("TELEGRAM_BOT_TOKEN").map(String::as_str),
        Some("1234567890:AAFakeSecretSegmentForTesting"),
    );

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("# header comment"));
    assert!(written.contains("[channels.telegram]"));
    assert!(written.contains("default_chat_id = \"987654\""));
}
