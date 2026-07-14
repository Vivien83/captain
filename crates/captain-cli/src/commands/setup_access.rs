use std::path::{Path, PathBuf};

use super::setup_profile::sanitize_setup_text;
use super::setup_support::{
    setup_config_bool, setup_config_string, setup_env_or_answer_any, setup_read_config_value,
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

#[derive(Debug, Clone)]
struct ExistingAccessConfig {
    config_api_key: Option<String>,
    secret_api_key: Option<String>,
    username: Option<String>,
    password_hash: Option<String>,
    auth_enabled: bool,
}

#[derive(Debug, Clone)]
struct ResolvedAccess {
    username: String,
    generated_password: Option<String>,
    password_hash: String,
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
    let (generated_password, password_hash) =
        setup_resolve_password_hash(existing.password_hash.clone(), provided_password);
    let (api_key, generated_api_key) = setup_resolve_daemon_api_key(answers, existing);

    Ok(ResolvedAccess {
        username,
        generated_password,
        password_hash,
        api_key,
        generated_api_key,
    })
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
) -> (Option<String>, String) {
    let generated_password = if existing_hash.is_none() && provided_password.is_none() {
        Some(setup_generate_secret("captain-"))
    } else {
        None
    };
    let password_hash = existing_hash.unwrap_or_else(|| {
        let password = provided_password
            .as_deref()
            .or(generated_password.as_deref())
            .expect("password is generated when not provided");
        captain_api::session_auth::hash_password(password)
    });
    (generated_password, password_hash)
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
    std::fs::write(&path, contents).map_err(|e| format!("write {}: {e}", path.display()))?;
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

    std::fs::create_dir_all(captain_dir)
        .map_err(|e| format!("create {}: {e}", captain_dir.display()))?;
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
    let tmp = path.with_extension("env.tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    restrict_file_permissions(&tmp);
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    restrict_file_permissions(&path);
    Ok(())
}
