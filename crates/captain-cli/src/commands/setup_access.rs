use std::path::{Path, PathBuf};

use super::setup_profile::sanitize_setup_text;
use super::setup_support::{
    setup_config_bool, setup_config_string, setup_config_value, setup_env_or_answer_any,
    setup_read_config_value,
};
use crate::{prompt_input, restrict_file_permissions, ui};

type ConfigPatch = captain_runtime::integrations::ConfigPatch;

#[derive(Debug, Clone)]
pub(crate) struct SetupAccessOutcome {
    pub(crate) username: String,
    pub(crate) generated_password: Option<String>,
    pub(crate) generated_api_key: bool,
    pub(crate) credentials_path: Option<PathBuf>,
}

#[derive(Clone)]
struct ExistingAccessConfig {
    config_api_key: Option<String>,
    secret_api_key: Option<String>,
    username: Option<String>,
    password_hash: Option<String>,
    session_secret: Option<String>,
    session_epoch: u64,
    auth_enabled: bool,
}

#[derive(Clone)]
struct ResolvedAccess {
    username: String,
    generated_password: Option<String>,
    password_hash: String,
    session_secret: String,
    session_epoch: u64,
    api_key: String,
    generated_api_key: bool,
}

pub(crate) fn setup_bootstrap_access(
    captain_dir: &Path,
    answers: Option<&toml::Value>,
    interactive: bool,
) -> Result<SetupAccessOutcome, String> {
    ui::blank();
    ui::section("Accès sécurisé");

    let existing = setup_existing_access(captain_dir);
    let access = setup_resolve_access(answers, interactive, &existing)?;
    setup_store_secret(captain_dir, "CAPTAIN_DAEMON_API_KEY", &access.api_key)?;
    setup_apply_access_config(captain_dir, &access)?;
    let credentials_path = setup_write_initial_credentials(captain_dir, &access, &existing)?;
    setup_print_access_summary(&access.username, credentials_path.as_ref());

    Ok(SetupAccessOutcome {
        username: access.username,
        generated_password: access.generated_password,
        generated_api_key: access.generated_api_key,
        credentials_path,
    })
}

fn setup_existing_access(captain_dir: &Path) -> ExistingAccessConfig {
    let config = setup_read_config_value(captain_dir);
    ExistingAccessConfig {
        config_api_key: setup_config_string(config.as_ref(), "api_key"),
        secret_api_key: setup_read_secret(captain_dir, "CAPTAIN_DAEMON_API_KEY")
            .or_else(|| setup_read_secret(captain_dir, "CAPTAIN_API_KEY")),
        username: setup_config_string(config.as_ref(), "auth.username"),
        password_hash: setup_config_string(config.as_ref(), "auth.password_hash"),
        session_secret: setup_config_string(config.as_ref(), "auth.session_secret"),
        session_epoch: setup_config_value(config.as_ref(), "auth.session_epoch")
            .and_then(toml::Value::as_integer)
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(0),
        auth_enabled: setup_config_bool(config.as_ref(), "auth.enabled").unwrap_or(false),
    }
}

fn setup_resolve_access(
    answers: Option<&toml::Value>,
    interactive: bool,
    existing: &ExistingAccessConfig,
) -> Result<ResolvedAccess, String> {
    let username = setup_resolve_admin_username(answers, interactive, existing.username.clone());
    let provided_password = setup_configured_admin_password(answers);
    let (generated_password, password_hash, password_changed) =
        setup_resolve_password_hash(existing.password_hash.clone(), provided_password)?;
    let session_secret = setup_resolve_session_secret(existing.session_secret.as_deref())?;
    let session_epoch = setup_resolve_session_epoch(
        existing.session_epoch,
        password_changed && existing.password_hash.is_some(),
    )?;
    let (api_key, generated_api_key) = setup_resolve_daemon_api_key(answers, existing);

    Ok(ResolvedAccess {
        username,
        generated_password,
        password_hash,
        session_secret,
        session_epoch,
        api_key,
        generated_api_key,
    })
}

fn setup_resolve_session_epoch(current: u64, password_changed: bool) -> Result<u64, String> {
    if password_changed {
        let next = current
            .checked_add(1)
            .ok_or_else(|| "auth.session_epoch cannot be incremented".to_string())?;
        i64::try_from(next)
            .map_err(|_| "auth.session_epoch exceeds the TOML integer range".to_string())?;
        Ok(next)
    } else {
        Ok(current)
    }
}

fn setup_resolve_admin_username(
    answers: Option<&toml::Value>,
    interactive: bool,
    existing_username: Option<String>,
) -> String {
    let mut username = setup_env_or_answer_any(
        "CAPTAIN_ADMIN_USERNAME",
        answers,
        &["auth.username", "web.username", "admin.username"],
    )
    .or(existing_username)
    .unwrap_or_else(|| "admin".to_string());

    if interactive {
        let answer = prompt_input(&format!("  Identifiant admin [{username}] : "));
        if !answer.trim().is_empty() {
            username = sanitize_setup_text(&answer, "admin", 64);
        }
    }
    username
}

fn setup_configured_admin_password(answers: Option<&toml::Value>) -> Option<String> {
    setup_env_or_answer_any(
        "CAPTAIN_ADMIN_PASSWORD",
        answers,
        &["auth.password", "web.password", "admin.password"],
    )
    .or_else(|| setup_env_or_answer_any("CAPTAIN_WEB_PASSWORD", answers, &["web_password"]))
}

fn setup_resolve_password_hash(
    existing_hash: Option<String>,
    provided_password: Option<String>,
) -> Result<(Option<String>, String, bool), String> {
    let generated_password = if existing_hash.is_none() && provided_password.is_none() {
        Some(setup_generate_secret("captain-"))
    } else {
        None
    };
    let candidate_password = provided_password
        .as_deref()
        .or(generated_password.as_deref());

    let Some(password) = candidate_password else {
        return Ok((
            generated_password,
            existing_hash.expect("an existing hash is present when no password is generated"),
            false,
        ));
    };

    if let Some(existing) = existing_hash.as_ref() {
        match captain_types::config::verify_web_password(password, existing) {
            captain_types::config::WebPasswordVerification::Argon2id => {
                return Ok((generated_password, existing.clone(), false));
            }
            captain_types::config::WebPasswordVerification::LegacySha256 => {
                let migrated = captain_types::config::hash_web_password(password)
                    .map_err(|error| format!("hash admin password: {error}"))?;
                return Ok((generated_password, migrated, false));
            }
            captain_types::config::WebPasswordVerification::Invalid => {}
        }
    }

    let password_hash = captain_types::config::hash_web_password(password)
        .map_err(|error| format!("hash admin password: {error}"))?;
    let password_changed = existing_hash.is_some();
    Ok((generated_password, password_hash, password_changed))
}

fn setup_resolve_session_secret(existing: Option<&str>) -> Result<String, String> {
    if let Some(existing) = existing {
        if captain_types::config::decode_session_secret(existing).is_none() {
            return Err(
                "auth.session_secret is invalid; remove it before rotating signing state"
                    .to_string(),
            );
        }
        return Ok(existing.to_string());
    }
    captain_types::config::generate_session_secret()
        .map_err(|error| format!("generate auth.session_secret: {error}"))
}

fn setup_resolve_daemon_api_key(
    answers: Option<&toml::Value>,
    existing: &ExistingAccessConfig,
) -> (String, bool) {
    let configured_api_key = setup_env_or_answer_any(
        "CAPTAIN_DAEMON_API_KEY",
        answers,
        &["auth.api_key", "daemon.api_key", "api.api_key", "api_key"],
    )
    .or_else(|| {
        setup_env_or_answer_any(
            "CAPTAIN_AUTH_API_KEY",
            answers,
            &["web.api_key", "access.api_key"],
        )
    });

    let generated_api_key = existing.config_api_key.is_none()
        && existing.secret_api_key.is_none()
        && configured_api_key.is_none();
    let api_key = existing
        .config_api_key
        .clone()
        .or_else(|| existing.secret_api_key.clone())
        .or(configured_api_key)
        .unwrap_or_else(|| setup_generate_secret("captain_api_"));

    (api_key, generated_api_key)
}

fn setup_apply_access_config(captain_dir: &Path, access: &ResolvedAccess) -> Result<(), String> {
    let patches = setup_access_config_patches(access);
    captain_runtime::integrations::apply_config_patch(&captain_dir.join("config.toml"), &patches)?;
    restrict_file_permissions(&captain_dir.join("config.toml"));
    Ok(())
}

fn setup_access_config_patches(access: &ResolvedAccess) -> Vec<ConfigPatch> {
    let patches = vec![
        ConfigPatch {
            path: vec![],
            key: "api_key".to_string(),
            value: toml_edit::value(""),
        },
        ConfigPatch {
            path: vec!["auth".to_string()],
            key: "enabled".to_string(),
            value: toml_edit::value(true),
        },
        ConfigPatch {
            path: vec!["auth".to_string()],
            key: "username".to_string(),
            value: toml_edit::value(access.username.as_str()),
        },
        ConfigPatch {
            path: vec!["auth".to_string()],
            key: "password_hash".to_string(),
            value: toml_edit::value(access.password_hash.as_str()),
        },
        ConfigPatch {
            path: vec!["auth".to_string()],
            key: "session_secret".to_string(),
            value: toml_edit::value(access.session_secret.as_str()),
        },
        ConfigPatch {
            path: vec!["auth".to_string()],
            key: "session_epoch".to_string(),
            value: toml_edit::value(access.session_epoch as i64),
        },
        ConfigPatch {
            path: vec!["auth".to_string()],
            key: "session_ttl_hours".to_string(),
            value: toml_edit::value(72),
        },
    ];
    patches
}

fn setup_write_initial_credentials(
    captain_dir: &Path,
    access: &ResolvedAccess,
    existing: &ExistingAccessConfig,
) -> Result<Option<PathBuf>, String> {
    if access.generated_password.is_none() && !access.generated_api_key && existing.auth_enabled {
        return Ok(None);
    }

    let path = captain_dir.join("initial-credentials.txt");
    let contents = setup_initial_credentials_contents(access);
    captain_types::durable_fs::atomic_write(&path, contents.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    restrict_file_permissions(&path);
    Ok(Some(path))
}

fn setup_initial_credentials_contents(access: &ResolvedAccess) -> String {
    let password_line = access
        .generated_password
        .as_deref()
        .map(|password| format!("Password: {password}"))
        .unwrap_or_else(|| "Password: provided during setup (not written)".to_string());
    let api_key_line = if access.generated_api_key {
        format!("API key: {}", access.api_key)
    } else {
        "API key: already configured or provided during setup".to_string()
    };
    format!(
        "# Captain initial access\n\
Generated by `captain setup`.\n\
Keep this file private; it is chmod 600 on Unix systems.\n\n\
Web terminal: http://127.0.0.1:50051/terminal\n\
Username:  {}\n\
{password_line}\n\
Web session: 72 hours\n\
API key storage: ~/.captain/secrets.env (CAPTAIN_DAEMON_API_KEY)\n\
{api_key_line}\n",
        access.username
    )
}

fn setup_print_access_summary(username: &str, credentials_path: Option<&PathBuf>) {
    ui::success("Web/API auth configurée");
    ui::kv("Admin", username);
    if let Some(path) = credentials_path {
        ui::kv("Accès initial", &path.display().to_string());
    }
}

fn setup_generate_secret(prefix: &str) -> String {
    format!("{prefix}{}", uuid::Uuid::new_v4().as_simple())
}

fn setup_read_secret(captain_dir: &Path, key: &str) -> Option<String> {
    for path in [captain_dir.join("secrets.env"), captain_dir.join(".env")] {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((existing_key, value)) = line.split_once('=') else {
                continue;
            };
            if existing_key.trim() == key {
                let value = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn setup_store_secret(captain_dir: &Path, key: &str, value: &str) -> Result<(), String> {
    if key.trim().is_empty()
        || key.contains('=')
        || key.contains('\n')
        || value.contains('\n')
        || value.trim() != value
    {
        return Err("secret key/value invalide".to_string());
    }

    let path = captain_dir.join("secrets.env");
    let original = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = original.lines().map(str::to_string).collect();
    let mut replaced = false;
    for line in &mut lines {
        if let Some((existing_key, _)) = line.split_once('=') {
            if existing_key.trim() == key {
                *line = format!("{key}={value}");
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        lines.push(format!("{key}={value}"));
    }
    let serialized = lines.join("\n") + "\n";
    captain_types::durable_fs::atomic_write(&path, serialized.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    restrict_file_permissions(&path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_changes_increment_the_session_epoch_once() {
        let existing = captain_api::session_auth::hash_password("old-password").unwrap();
        let (_, unchanged, changed) =
            setup_resolve_password_hash(Some(existing.clone()), Some("old-password".to_string()))
                .unwrap();
        assert_eq!(unchanged, existing);
        assert!(!changed);
        assert_eq!(setup_resolve_session_epoch(7, changed).unwrap(), 7);

        let (_, replacement, changed) =
            setup_resolve_password_hash(Some(existing), Some("new-password".to_string())).unwrap();
        assert!(changed);
        assert_eq!(setup_resolve_session_epoch(7, changed).unwrap(), 8);
        assert!(captain_api::session_auth::verify_password(
            "new-password",
            &replacement
        ));
    }

    #[test]
    fn legacy_password_hash_migrates_without_invalidating_sessions() {
        use sha2::{Digest, Sha256};
        let legacy = format!("{:x}", Sha256::digest(b"old-password"));
        let (_, migrated, changed) =
            setup_resolve_password_hash(Some(legacy), Some("old-password".to_string())).unwrap();

        assert!(migrated.starts_with("$argon2id$"));
        assert!(!changed);
        assert_eq!(setup_resolve_session_epoch(7, changed).unwrap(), 7);
    }

    #[test]
    fn setup_session_secret_is_random_and_valid() {
        let first = setup_resolve_session_secret(None).unwrap();
        let second = setup_resolve_session_secret(None).unwrap();

        assert_ne!(first, second);
        assert!(captain_types::config::decode_session_secret(&first).is_some());
        assert_eq!(setup_resolve_session_secret(Some(&first)).unwrap(), first);
    }

    #[test]
    fn setup_refuses_malformed_existing_session_secret() {
        assert!(setup_resolve_session_secret(Some("not-base64")).is_err());
    }
}
