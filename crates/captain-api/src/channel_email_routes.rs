//! Named Email account configuration, removal, readiness, and live probes.

use crate::channel_config_store::{
    email_account_password_key, mutate_email_channel_account_transactional,
    parse_email_channel_config, remove_channel_config, with_email_config_lock,
};
use crate::channel_readiness_email::{email_channel_readiness, EmailChannelReadiness};
use crate::channel_test_delivery::test_email_account;
use crate::secret_env::remove_secret_env;
use crate::state::AppState;
use axum::http::StatusCode;
use axum::Json;
use captain_channels::email::email_allowlist_rule_is_valid;
use captain_types::config::{is_valid_email_account_alias, EmailAccountConfig, EmailConfig};
use serde::Deserialize;
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureEmailRequest {
    account: ConfigureEmailAccount,
    #[serde(default)]
    make_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureEmailAccount {
    alias: String,
    enabled: Option<bool>,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    username: Option<String>,
    password_env: Option<String>,
    password: Option<String>,
    poll_interval_secs: Option<u64>,
    folders: Option<Vec<String>>,
    allowed_senders: Option<Vec<String>>,
    default_agent: Option<String>,
}

pub(crate) async fn configure_email_channel_account(
    state: &Arc<AppState>,
    body: serde_json::Value,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut request = match serde_json::from_value::<ConfigureEmailRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return bad_request(&format!(
                "Invalid Email account request: {error}. Expected {{account: {{alias, imap_host, smtp_host, username, password, allowed_senders}}, make_default}}"
            ));
        }
    };
    request.account.alias = request.account.alias.trim().to_string();
    if !is_valid_email_account_alias(&request.account.alias) {
        return bad_request(
            "Email account alias must be 1-32 lowercase letters, digits, '.', '_' or '-', starting with a letter or digit",
        );
    }

    let password = request.account.password.take().map(Zeroizing::new);
    if password
        .as_deref()
        .is_some_and(|password| password.trim().is_empty())
    {
        return bad_request("Email password must not be empty");
    }

    let config_path = state.kernel.config.home_dir.join("config.toml");
    let kernel = state.kernel.clone();
    let account_request = request.account;
    let make_default = request.make_default;
    let (configured, account) = match tokio::task::spawn_blocking(move || {
        mutate_email_channel_account_transactional(
            &config_path,
            make_default,
            |current| {
                let account = email_account_from_request(&account_request, current)
                    .map_err(|error| format!("request:{error}"))?;
                let existing_credential = kernel
                    .resolve_credential(&account.password_env)
                    .is_some();
                if account.enabled && password.is_none() && !existing_credential {
                    return Err(format!(
                        "request:Email account '{}' needs a password or an already resolvable password_env before it can be enabled",
                        account.alias
                    ));
                }
                if password.is_some()
                    && kernel.credential_is_externally_managed(&account.password_env)
                {
                    return Err(format!(
                        "request:{} is managed by secret-sources.toml; rotate the external file instead",
                        account.password_env
                    ));
                }
                Ok(account)
            },
            |account| {
                let Some(password) = password.as_deref() else {
                    return Ok(());
                };
                captain_runtime::kernel_handle::KernelHandle::secret_write(
                    kernel.as_ref(),
                    &account.password_env,
                    password,
                )
                .map_err(|error| format!("Could not store {}: {error}", account.password_env))
            },
        )
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => match error.strip_prefix("request:") {
            Some(message) => return bad_request(message),
            None => return server_error(error),
        },
        Err(error) => return server_error(format!("Email configuration task failed: {error}")),
    };

    let readiness = email_channel_readiness(Some(&configured), &|key| {
        state.kernel.resolve_credential(key)
    });
    match crate::channel_bridge::reload_channels_from_disk(state).await {
        Ok(started) => email_configured_response(&readiness, &account.alias, &started, None),
        Err(error) => {
            tracing::warn!(error = %error, account = %account.alias, "Email hot-reload failed after configure");
            email_configured_response(&readiness, &account.alias, &[], Some(error))
        }
    }
}

fn email_account_from_request(
    request: &ConfigureEmailAccount,
    current: Option<&EmailConfig>,
) -> Result<EmailAccountConfig, String> {
    let existing = current.and_then(|config| {
        config
            .effective_accounts()
            .into_iter()
            .find(|account| account.alias == request.alias)
    });
    let mut account = existing.unwrap_or_else(|| EmailAccountConfig {
        alias: request.alias.clone(),
        password_env: email_account_password_key(&request.alias),
        ..EmailAccountConfig::default()
    });
    account.alias = request.alias.clone();
    if let Some(enabled) = request.enabled {
        account.enabled = enabled;
    }
    assign_trimmed(&mut account.imap_host, request.imap_host.as_deref());
    assign_trimmed(&mut account.smtp_host, request.smtp_host.as_deref());
    assign_trimmed(&mut account.username, request.username.as_deref());
    assign_trimmed(&mut account.password_env, request.password_env.as_deref());
    if let Some(port) = request.imap_port {
        account.imap_port = port;
    }
    if let Some(port) = request.smtp_port {
        account.smtp_port = port;
    }
    if let Some(interval) = request.poll_interval_secs {
        account.poll_interval_secs = interval;
    }
    if let Some(folders) = &request.folders {
        account.folders = normalized_nonempty_values(folders);
    }
    if let Some(allowed_senders) = &request.allowed_senders {
        account.allowed_senders = normalized_nonempty_values(allowed_senders);
    }
    if let Some(default_agent) = &request.default_agent {
        account.default_agent =
            (!default_agent.trim().is_empty()).then(|| default_agent.trim().to_string());
    }

    let errors = account.validation_errors();
    if !errors.is_empty() {
        return Err(format!(
            "Invalid Email account '{}': {}",
            account.alias,
            errors.join("; ")
        ));
    }
    if account.enabled && account.allowed_senders.is_empty() {
        return Err(format!(
            "Email account '{}' needs at least one allowed_senders entry; use '*' only when open inbound access is intentional",
            account.alias
        ));
    }
    let invalid_allowlist = account
        .allowed_senders
        .iter()
        .filter(|rule| !email_allowlist_rule_is_valid(rule))
        .cloned()
        .collect::<Vec<_>>();
    if !invalid_allowlist.is_empty() {
        return Err(format!(
            "Email account '{}' has invalid allowed_senders entries: {}. Use an exact address, @domain, or *",
            account.alias,
            invalid_allowlist.join(", ")
        ));
    }
    Ok(account)
}

pub(crate) fn email_account_fields_json() -> serde_json::Value {
    serde_json::json!([
        {"key": "alias", "label": "Account name", "type": "text", "required": true, "placeholder": "work", "advanced": false, "secret": false},
        {"key": "enabled", "label": "Enabled", "type": "boolean", "required": false, "default": true, "advanced": true, "secret": false},
        {"key": "username", "label": "Mailbox address", "type": "text", "required": true, "placeholder": "captain@example.com", "advanced": false, "secret": false},
        {"key": "password", "label": "App password", "type": "secret", "required_for_new_enabled_account": true, "placeholder": "provider app password", "stored": false, "advanced": false, "secret": true},
        {"key": "password_env", "label": "Credential key", "type": "text", "required": false, "placeholder": "generated automatically", "advanced": true, "secret": false},
        {"key": "imap_host", "label": "IMAP host", "type": "text", "required": true, "placeholder": "imap.example.com", "advanced": false, "secret": false},
        {"key": "imap_port", "label": "IMAP port", "type": "number", "required": false, "default": 993, "advanced": true, "secret": false},
        {"key": "smtp_host", "label": "SMTP host", "type": "text", "required": true, "placeholder": "smtp.example.com", "advanced": false, "secret": false},
        {"key": "smtp_port", "label": "SMTP port", "type": "number", "required": false, "default": 587, "advanced": true, "secret": false},
        {"key": "poll_interval_secs", "label": "Poll interval (seconds)", "type": "number", "required": false, "default": 30, "advanced": true, "secret": false},
        {"key": "folders", "label": "Folders", "type": "list", "required": false, "default": ["INBOX"], "advanced": true, "secret": false},
        {"key": "allowed_senders", "label": "Allowed senders", "type": "list", "required": true, "placeholder": "me@example.com, @company.com", "advanced": false, "secret": false},
        {"key": "default_agent", "label": "Default agent", "type": "text", "required": false, "placeholder": "captain", "advanced": true, "secret": false},
        {"key": "make_default", "label": "Make default", "type": "boolean", "required": false, "default": false, "placeholder": "yes or no", "advanced": false, "scope": "request", "secret": false}
    ])
}

fn assign_trimmed(target: &mut String, supplied: Option<&str>) {
    if let Some(supplied) = supplied {
        *target = supplied.trim().to_string();
    }
}

fn normalized_nonempty_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn email_configured_response(
    readiness: &EmailChannelReadiness,
    alias: &str,
    started: &[String],
    reload_error: Option<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let account = readiness
        .accounts
        .iter()
        .find(|account| account.alias == alias);
    let adapter = account
        .map(|account| account.adapter.clone())
        .unwrap_or_else(|| format!("email:{alias}"));
    let activated = started.iter().any(|started| started == &adapter);
    let mut body = serde_json::json!({
        "status": if reload_error.is_some() { "configured_reload_failed" } else { "configured" },
        "channel": "email",
        "account": alias,
        "adapter": adapter,
        "activated": activated,
        "started_channels": started,
        "account_summary": account,
        "operational_state": readiness.operational_state,
        "account_count": readiness.total_accounts,
    });
    if let Some(error) = reload_error {
        body["note"] = serde_json::Value::String(format!(
            "Configured, but hot-reload failed: {error}. Restart daemon to activate."
        ));
    }
    (StatusCode::OK, Json(body))
}

pub(crate) fn remove_email_channel_persisted(
    kernel: &captain_kernel::CaptainKernel,
    config_path: &std::path::Path,
    secrets_path: &std::path::Path,
) -> Result<usize, String> {
    with_email_config_lock(config_path, || {
        let raw = if config_path.exists() {
            std::fs::read_to_string(config_path)
                .map_err(|error| format!("Could not read {}: {error}", config_path.display()))?
        } else {
            String::new()
        };
        let mut secret_keys = parse_email_channel_config(&raw)?
            .into_iter()
            .flat_map(|config| config.effective_accounts())
            .map(|account| account.password_env)
            .collect::<Vec<_>>();
        secret_keys.push("EMAIL_PASSWORD".to_string());
        secret_keys.sort();
        secret_keys.dedup();

        remove_channel_config(config_path, "email")
            .map_err(|error| format!("Failed to remove Email config: {error}"))?;
        let mut warning_count = 0;
        for key in secret_keys {
            if kernel.credential_is_externally_managed(&key) {
                continue;
            }
            if let Err(error) = remove_secret_env(secrets_path, &key) {
                warning_count += 1;
                tracing::warn!(credential = %key, error = %error, "Could not remove local Email credential after config removal");
                continue;
            }
            unsafe {
                std::env::remove_var(&key);
            }
        }
        Ok(warning_count)
    })
}

pub(crate) async fn test_email_channel_account(
    state: &Arc<AppState>,
    path_alias: Option<String>,
    body: &serde_json::Value,
) -> (StatusCode, Json<serde_json::Value>) {
    let body_alias = body
        .get("account_alias")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if path_alias.is_some() && body_alias.is_some() && path_alias != body_alias {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "Email account alias in the path and request body must match."
            })),
        );
    }
    let requested_alias = path_alias.or(body_alias);
    if requested_alias
        .as_deref()
        .is_some_and(|alias| !is_valid_email_account_alias(alias))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "Invalid Email account alias."
            })),
        );
    }

    let live_channels = state.channels_config.read().await;
    let Some(config) = live_channels.email.clone() else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": "Email channel is not configured."
            })),
        );
    };
    drop(live_channels);
    let resolve = |key: &str| state.kernel.resolve_credential(key);
    let readiness = email_channel_readiness(Some(&config), &resolve);
    if readiness.operational_state == "invalid" {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": "Email configuration is invalid.",
                "missing_required_fields": readiness.missing_required_fields,
                "operator_actions": readiness.operator_actions,
            })),
        );
    }
    let alias = requested_alias
        .or_else(|| config.effective_default_account())
        .unwrap_or_default();
    let accounts = config.effective_accounts();
    let Some(account) = accounts.iter().find(|account| account.alias == alias) else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Email account '{alias}' was not found."),
                "available_accounts": accounts.iter().map(|account| account.alias.as_str()).collect::<Vec<_>>(),
            })),
        );
    };
    let Some(account_readiness) = readiness
        .accounts
        .iter()
        .find(|candidate| candidate.alias == alias)
    else {
        tracing::error!(account = %alias, "Email readiness omitted a configured account");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": "Email readiness could not inspect the requested account."
            })),
        );
    };
    if !account_readiness.ready {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "account": alias,
                "message": "Email account is not ready for a live test.",
                "missing_required_fields": account_readiness.missing_required_fields,
                "operator_actions": account_readiness.operator_actions,
            })),
        );
    }
    let Some(password) = resolve(&account.password_env) else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "account": alias,
                "message": "Email account credential became unavailable before the live test."
            })),
        );
    };
    let recipient = body
        .get("recipient")
        .or_else(|| body.get("channel_id"))
        .and_then(serde_json::Value::as_str);
    match test_email_account(
        account,
        password,
        recipient,
        "Captain test message - your Email account is connected.",
    )
    .await
    {
        Ok(outcome) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "message": if outcome.message_sent {
                    "IMAP, SMTP authentication, and test delivery succeeded."
                } else {
                    "IMAP and SMTP authentication succeeded."
                },
                "account": alias,
                "adapter": account_readiness.adapter,
                "connectivity": outcome.connectivity,
                "message_sent": outcome.message_sent,
            })),
        ),
        Err(error) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "account": alias,
                "message": error,
            })),
        ),
    }
}

fn bad_request(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": message})),
    )
}

fn server_error(message: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": message})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_kernel(home: &std::path::Path) -> captain_kernel::CaptainKernel {
        captain_kernel::CaptainKernel::boot_with_config(captain_types::config::KernelConfig {
            home_dir: home.to_path_buf(),
            data_dir: home.join("data"),
            ..Default::default()
        })
        .expect("test kernel")
    }

    fn test_state(home: &std::path::Path) -> Arc<AppState> {
        let kernel = Arc::new(test_kernel(home));
        kernel.set_self_handle();
        Arc::new(AppState {
            kernel,
            started_at: std::time::Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        })
    }

    fn email_request(body: serde_json::Value) -> ConfigureEmailRequest {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn email_account_schema_is_complete_for_guided_clients() {
        let schema = email_account_fields_json();
        let fields = schema.as_array().unwrap();
        let password = fields
            .iter()
            .find(|field| field["key"] == "password")
            .unwrap();
        let make_default = fields
            .iter()
            .find(|field| field["key"] == "make_default")
            .unwrap();

        assert!(fields.iter().all(|field| field["label"].is_string()));
        assert!(fields.iter().all(|field| field["type"].is_string()));
        assert_eq!(password["secret"], true);
        assert_eq!(password["stored"], false);
        assert_eq!(password["required_for_new_enabled_account"], true);
        assert_eq!(make_default["scope"], "request");
        assert_eq!(make_default["type"], "boolean");
    }

    #[test]
    fn email_account_request_uses_stable_secret_and_safe_defaults() {
        let request = email_request(serde_json::json!({
            "account": {
                "alias": "work",
                "username": " captain@example.com ",
                "imap_host": " imap.example.com ",
                "smtp_host": " smtp.example.com ",
                "allowed_senders": [" operator@example.com "]
            }
        }));

        let account = email_account_from_request(&request.account, None).unwrap();

        assert_eq!(account.username, "captain@example.com");
        assert_eq!(account.allowed_senders, ["operator@example.com"]);
        assert_eq!(account.password_env, email_account_password_key("work"));
        assert_eq!(account.folders, ["INBOX"]);
        assert!(account.enabled);
    }

    #[test]
    fn email_account_patch_preserves_existing_credential_reference() {
        let current = EmailConfig {
            accounts: vec![EmailAccountConfig {
                alias: "work".to_string(),
                imap_host: "imap.example.com".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                username: "old@example.com".to_string(),
                password_env: "EXTERNAL_WORK_PASSWORD".to_string(),
                allowed_senders: vec!["operator@example.com".to_string()],
                ..EmailAccountConfig::default()
            }],
            default_account: Some("work".to_string()),
            ..EmailConfig::default()
        };
        let request = email_request(serde_json::json!({
            "account": {
                "alias": "work",
                "enabled": false,
                "username": "new@example.com"
            }
        }));

        let account = email_account_from_request(&request.account, Some(&current)).unwrap();

        assert_eq!(account.password_env, "EXTERNAL_WORK_PASSWORD");
        assert_eq!(account.username, "new@example.com");
        assert!(!account.enabled);
    }

    #[test]
    fn email_configured_response_exposes_readiness_but_no_secret_metadata() {
        let config = EmailConfig {
            accounts: vec![EmailAccountConfig {
                alias: "work".to_string(),
                imap_host: "imap.example.com".to_string(),
                smtp_host: "smtp.example.com".to_string(),
                username: "work@example.com".to_string(),
                password_env: "PRIVATE_WORK_PASSWORD".to_string(),
                allowed_senders: vec!["operator@example.com".to_string()],
                ..EmailAccountConfig::default()
            }],
            default_account: Some("work".to_string()),
            ..EmailConfig::default()
        };
        let readiness =
            email_channel_readiness(Some(&config), &|_| Some("super-secret-value".to_string()));

        let (_, Json(response)) =
            email_configured_response(&readiness, "work", &["email".to_string()], None);
        let serialized = serde_json::to_string(&response).unwrap();

        assert_eq!(response["activated"], true);
        assert_eq!(response["adapter"], "email");
        assert!(!serialized.contains("PRIVATE_WORK_PASSWORD"));
        assert!(!serialized.contains("super-secret-value"));
    }

    #[tokio::test]
    async fn email_api_can_stage_a_disabled_account_without_network_or_secret_leak() {
        let temp = tempfile::tempdir().unwrap();
        let state = test_state(temp.path());
        let request = serde_json::json!({
            "account": {
                "alias": "work",
                "enabled": false,
                "username": "work@example.com",
                "imap_host": "imap.example.com",
                "smtp_host": "smtp.example.com",
                "allowed_senders": []
            }
        });

        let (status, Json(response)) = configure_email_channel_account(&state, request).await;
        let serialized = serde_json::to_string(&response).unwrap();
        let raw = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        let config = crate::parse_email_channel_config(&raw).unwrap().unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["status"], "configured");
        assert_eq!(response["operational_state"], "disabled");
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.default_account, None);
        assert!(!serialized.contains("CAPTAIN_EMAIL_WORK"));
        state.kernel.shutdown();
    }

    #[test]
    fn email_removal_cleans_every_local_account_secret_under_the_config_lock() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = test_kernel(temp.path());
        let config_path = temp.path().join("config.toml");
        let secrets_path = temp.path().join("secrets.env");
        let secret_key = email_account_password_key("work");
        let mut account = EmailAccountConfig {
            alias: "work".to_string(),
            imap_host: "imap.example.com".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            username: "work@example.com".to_string(),
            password_env: secret_key.clone(),
            allowed_senders: vec!["operator@example.com".to_string()],
            ..EmailAccountConfig::default()
        };
        account.enabled = false;
        crate::upsert_email_channel_account(&config_path, account, false).unwrap();
        captain_runtime::kernel_handle::KernelHandle::secret_write(
            &kernel,
            &secret_key,
            "private-value",
        )
        .unwrap();

        let warning_count =
            remove_email_channel_persisted(&kernel, &config_path, &secrets_path).unwrap();
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let secrets = std::fs::read_to_string(&secrets_path).unwrap_or_default();

        assert_eq!(warning_count, 0);
        assert!(crate::parse_email_channel_config(&raw).unwrap().is_none());
        assert!(!secrets.contains(&secret_key));
        assert!(!secrets.contains("private-value"));
        unsafe {
            std::env::remove_var(&secret_key);
        }
        kernel.shutdown();
    }
}
