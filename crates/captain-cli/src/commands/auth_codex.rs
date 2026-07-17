use crate::{
    captain_home, restrict_dir_permissions, restrict_file_permissions, ui,
    write_default_model_config,
};

use captain_runtime::codex_oauth::{
    exchange_code, poll_authorization, request_device_code, CodexCredentials, CodexOAuthError,
    DeviceCodeResponse, PollOutcome, CODEX_DEVICE_VERIFICATION_URL,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";
const CODEX_LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Phase-k.3: drive the Codex device-code OAuth flow interactively.
/// Persists tokens to ~/.codex/auth.json so the official Codex CLI sees them
/// too. Optionally runs the model picker post-login (k.4).
pub(crate) fn cmd_login_codex(with_model: bool) {
    let rt = codex_runtime_or_exit();
    let creds = request_codex_credentials_or_exit(&rt);

    persist_codex_cli_auth_or_exit(&creds);
    refresh_codex_models_cache_or_hint(&rt, &creds);
    configure_codex_model_after_login(with_model);

    ui::success("Connexion Codex réussie.");
}

fn codex_runtime_or_exit() -> Runtime {
    match Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            ui::error_with_fix(&format!("Tokio runtime: {e}"), "Try restarting captain.");
            std::process::exit(1);
        }
    }
}

fn request_codex_credentials_or_exit(rt: &Runtime) -> CodexCredentials {
    rt.block_on(request_codex_credentials())
}

async fn request_codex_credentials() -> CodexCredentials {
    ui::step("Demande d'un code de connexion ChatGPT…");
    let dc = match request_device_code().await {
        Ok(d) => d,
        Err(e) => {
            ui::error_with_fix(
                &format!("Échec demande device code: {e}"),
                "Vérifie ta connexion à auth.openai.com",
            );
            std::process::exit(1);
        }
    };

    show_codex_device_instructions(&dc);
    open_codex_device_url_best_effort();
    wait_for_codex_authorization_or_exit(&dc).await
}

fn show_codex_device_instructions(dc: &DeviceCodeResponse) {
    println!();
    println!("  Pour continuer :");
    println!(
        "    1. Ouvre {} dans ton navigateur",
        CODEX_DEVICE_VERIFICATION_URL
    );
    println!("    2. Connecte-toi à ChatGPT (Plus / Pro / Pro+)");
    println!("    3. Saisis ce code : \x1b[1;94m{}\x1b[0m", dc.user_code);
    println!();
}

fn open_codex_device_url_best_effort() {
    let _ = std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "start"
    } else {
        "xdg-open"
    })
    .arg(CODEX_DEVICE_VERIFICATION_URL)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn();
}

async fn wait_for_codex_authorization_or_exit(dc: &DeviceCodeResponse) -> CodexCredentials {
    ui::step(&format!(
        "En attente de la connexion (poll toutes les {}s, Ctrl+C pour annuler)…",
        dc.interval
    ));
    let interval = codex_poll_interval(dc.interval);
    let deadline = codex_poll_deadline();
    let mut transient_poll_errors = 0usize;

    loop {
        if Instant::now() >= deadline {
            ui::error_with_fix("Délai dépassé (15 min).", "Relance `captain login codex`");
            std::process::exit(1);
        }
        tokio::time::sleep(interval).await;
        match poll_authorization(&dc.device_auth_id, &dc.user_code).await {
            Ok(PollOutcome::Pending) => continue,
            Ok(PollOutcome::Authorized {
                authorization_code,
                code_verifier,
            }) => {
                return exchange_codex_authorization_or_exit(&authorization_code, &code_verifier)
                    .await
            }
            Err(CodexOAuthError::Http(e)) => {
                if record_transient_poll_error(&mut transient_poll_errors) {
                    ui::hint(&format!(
                        "Connexion OpenAI temporairement instable, Captain continue d'attendre ({e})."
                    ));
                }
            }
            Err(CodexOAuthError::PollError(status)) if status == 429 || status >= 500 => {
                if record_transient_poll_error(&mut transient_poll_errors) {
                    ui::hint(&format!(
                        "Service OpenAI temporairement indisponible ({status}), Captain continue d'attendre."
                    ));
                }
            }
            Err(e) => {
                ui::error_with_fix(
                    &format!("Erreur de polling: {e}"),
                    "Relance `captain login codex`",
                );
                std::process::exit(1);
            }
        }
    }
}

fn codex_poll_interval(interval_seconds: u64) -> Duration {
    Duration::from_secs(interval_seconds.clamp(1, 30))
}

fn codex_poll_deadline() -> Instant {
    Instant::now() + CODEX_LOGIN_TIMEOUT
}

async fn exchange_codex_authorization_or_exit(
    authorization_code: &str,
    code_verifier: &str,
) -> CodexCredentials {
    ui::step("Connexion détectée, échange du token…");
    match exchange_code(authorization_code, code_verifier).await {
        Ok(c) => c,
        Err(e) => {
            ui::error_with_fix(
                &format!("Échec exchange: {e}"),
                "Relance `captain login codex`",
            );
            std::process::exit(1);
        }
    }
}

fn record_transient_poll_error(transient_poll_errors: &mut usize) -> bool {
    *transient_poll_errors += 1;
    should_report_transient_poll_error(*transient_poll_errors)
}

fn should_report_transient_poll_error(transient_poll_errors: usize) -> bool {
    transient_poll_errors == 1 || transient_poll_errors.is_multiple_of(6)
}

fn persist_codex_cli_auth_or_exit(creds: &CodexCredentials) {
    if let Some(home) = dirs::home_dir() {
        let codex_dir = codex_auth_dir(home);
        if let Err(e) = std::fs::create_dir_all(&codex_dir) {
            ui::error_with_fix(
                &format!("Création {} : {e}", codex_dir.display()),
                "Vérifie les permissions du dossier home",
            );
            std::process::exit(1);
        }
        let auth_path = codex_dir.join("auth.json");
        write_codex_auth_file_or_exit(&auth_path, &codex_auth_payload(creds));
    }
}

fn codex_auth_dir(home: PathBuf) -> PathBuf {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"))
}

fn codex_auth_payload(creds: &CodexCredentials) -> serde_json::Value {
    serde_json::json!({
        "tokens": {
            "access_token": &creds.access_token,
            "refresh_token": &creds.refresh_token,
        },
        "api_key": &creds.access_token,
        "expires_at": creds.expires_at,
        "last_refresh": &creds.last_refresh,
        "auth_mode": &creds.auth_mode,
        "source": &creds.source,
    })
}

fn write_codex_auth_file_or_exit(auth_path: &Path, payload: &serde_json::Value) {
    match captain_types::durable_fs::atomic_write(
        auth_path,
        serde_json::to_string_pretty(payload)
            .unwrap_or_default()
            .as_bytes(),
    ) {
        Ok(()) => {
            restrict_file_permissions(auth_path);
            ui::success(&format!("Tokens persistés dans {}", auth_path.display()));
        }
        Err(e) => {
            ui::error_with_fix(
                &format!("Écriture {} : {e}", auth_path.display()),
                "Vérifie les permissions",
            );
            std::process::exit(1);
        }
    }
}

fn refresh_codex_models_cache_or_hint(rt: &Runtime, creds: &CodexCredentials) {
    match rt.block_on(
        captain_runtime::model_catalog::refresh_codex_models_cache_with_token(
            &creds.access_token,
            captain_types::model_catalog::CODEX_BASE_URL,
        ),
    ) {
        Ok(count) => ui::success(&format!("Catalogue Codex rafraîchi ({count} modèles).")),
        Err(_) => ui::hint(
            "Catalogue Codex indisponible temporairement ; modèle par défaut gpt-5.5 conservé.",
        ),
    }
}

fn configure_codex_model_after_login(with_model: bool) {
    if with_model {
        cmd_login_codex_pick_model();
        return;
    }
    match write_codex_default_model(DEFAULT_CODEX_MODEL) {
        Ok(()) => ui::success("Codex configuré par défaut : gpt-5.5"),
        Err(e) => ui::warn_with_fix(
            &format!("Codex connecté, mais config par défaut non écrite : {e}"),
            "Relance `captain login codex --with-model` ou vérifie ~/.captain/config.toml",
        ),
    }
}

fn write_codex_default_model(model_id: &str) -> Result<(), String> {
    let home = captain_home();
    std::fs::create_dir_all(&home).map_err(|e| format!("create {}: {e}", home.display()))?;
    restrict_dir_permissions(&home);
    write_default_model_config(&home, "codex", model_id, "")
}

fn cmd_login_codex_pick_model() {
    let models =
        codex_model_choices_with_fallback(captain_runtime::model_catalog::codex_model_choices());
    println!();
    println!("  Modèles Codex disponibles :");
    for (i, (id, label)) in models.iter().enumerate() {
        println!("    {}. {:<18}  {}", i + 1, id, label);
    }
    print!("  Choix [1-{}, Entrée=1] : ", models.len());
    use std::io::{self, Write};
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    let choice = buf
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=models.len()).contains(n))
        .unwrap_or(1);
    let model_id = models[choice - 1].0.as_str();

    if let Err(e) = write_codex_default_model(model_id) {
        ui::error_with_fix(
            &format!("Écriture config Codex : {e}"),
            "Vérifie les permissions",
        );
        return;
    }
    ui::success(&format!("Modèle par défaut : {model_id} (provider=codex)"));
}

fn codex_model_choices_with_fallback(models: Vec<(String, String)>) -> Vec<(String, String)> {
    if models.is_empty() {
        return codex_fallback_model_choices();
    }
    models
}

fn codex_fallback_model_choices() -> Vec<(String, String)> {
    vec![
        (
            "gpt-5.5".to_string(),
            "GPT-5.5 (Codex fallback)".to_string(),
        ),
        (
            "gpt-5.4".to_string(),
            "GPT-5.4 (Codex fallback)".to_string(),
        ),
        (
            "gpt-5.3-codex".to_string(),
            "GPT-5.3 Codex (Codex fallback)".to_string(),
        ),
        (
            "gpt-5.3-codex-spark".to_string(),
            "GPT-5.3 Codex Spark (Codex fallback)".to_string(),
        ),
        (
            "gpt-5.2".to_string(),
            "GPT-5.2 (Codex fallback)".to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_creds() -> CodexCredentials {
        CodexCredentials {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: 123456,
            last_refresh: "123Z".to_string(),
            auth_mode: "chatgpt".to_string(),
            source: "device-code".to_string(),
        }
    }

    #[test]
    fn codex_poll_interval_is_clamped() {
        assert_eq!(codex_poll_interval(0), Duration::from_secs(1));
        assert_eq!(codex_poll_interval(5), Duration::from_secs(5));
        assert_eq!(codex_poll_interval(99), Duration::from_secs(30));
    }

    #[test]
    fn transient_poll_error_reporting_is_throttled() {
        assert!(should_report_transient_poll_error(1));
        assert!(!should_report_transient_poll_error(2));
        assert!(!should_report_transient_poll_error(5));
        assert!(should_report_transient_poll_error(6));
        assert!(!should_report_transient_poll_error(7));
        assert!(should_report_transient_poll_error(12));
    }

    #[test]
    fn codex_auth_payload_matches_cli_shape() {
        let payload = codex_auth_payload(&sample_creds());
        assert_eq!(payload["tokens"]["access_token"], "access");
        assert_eq!(payload["tokens"]["refresh_token"], "refresh");
        assert_eq!(payload["api_key"], "access");
        assert_eq!(payload["expires_at"], 123456);
        assert_eq!(payload["auth_mode"], "chatgpt");
        assert_eq!(payload["source"], "device-code");
    }

    #[test]
    fn codex_model_choices_preserve_catalog_or_fallback() {
        let catalog = vec![("codex-x".to_string(), "Codex X".to_string())];
        assert_eq!(codex_model_choices_with_fallback(catalog.clone()), catalog);

        let fallback = codex_model_choices_with_fallback(Vec::new());
        assert_eq!(fallback[0].0, DEFAULT_CODEX_MODEL);
        assert!(fallback.iter().any(|(id, _)| id == "gpt-5.3-codex"));
    }
}
