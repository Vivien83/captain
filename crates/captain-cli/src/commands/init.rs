use std::io::IsTerminal;
use std::path::Path;

use crate::{
    cli_captain_home, find_daemon, open_in_browser, restrict_dir_permissions,
    restrict_file_permissions, ui,
};

pub(crate) fn cmd_init(quick: bool) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Could not determine home directory");
            std::process::exit(1);
        }
    };

    let captain_dir = cli_captain_home();

    if !captain_dir.exists() {
        std::fs::create_dir_all(&captain_dir).unwrap_or_else(|e| {
            ui::error_with_fix(
                &format!("Failed to create {}", captain_dir.display()),
                &format!("Check permissions on {}", home.display()),
            );
            eprintln!("  {e}");
            std::process::exit(1);
        });
        restrict_dir_permissions(&captain_dir);
    }

    for sub in ["data", "agents"] {
        let dir = captain_dir.join(sub);
        if !dir.exists() {
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
                eprintln!("Error creating {sub} dir: {e}");
                std::process::exit(1);
            });
        }
    }

    if quick {
        cmd_init_quick(&captain_dir);
    } else if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        ui::hint("Non-interactive terminal detected — running in quick mode");
        ui::hint("For the interactive wizard, run: captain init (in a terminal)");
        cmd_init_quick(&captain_dir);
    } else {
        cmd_init_interactive(&captain_dir);
    }
}

fn cmd_init_quick(captain_dir: &Path) {
    ui::banner();
    ui::blank();

    let (provider, api_key_env, model) = detect_best_provider();

    write_config_if_missing(captain_dir, provider, model, api_key_env);

    ui::blank();
    ui::success("Captain initialized (quick mode)");
    ui::kv("Provider", provider);
    ui::kv("Model", model);
    ui::blank();
    ui::next_steps(&[
        "Start the daemon:  captain start",
        "Chat:              captain chat",
    ]);
}

fn cmd_init_interactive(captain_dir: &Path) {
    use crate::tui::screens::init_wizard::{self, InitResult, LaunchChoice};

    match init_wizard::run() {
        InitResult::Completed {
            provider,
            model,
            daemon_started,
            launch,
        } => {
            ui::blank();
            ui::success("Captain initialized!");
            ui::kv("Provider", &provider);
            ui::kv("Model", &model);

            if daemon_started {
                ui::kv_ok("Daemon", "running");
            }
            ui::blank();

            match launch {
                LaunchChoice::Desktop => {
                    launch_desktop_app(captain_dir);
                }
                LaunchChoice::WebTerminal => {
                    if let Some(base) = find_daemon() {
                        let url = format!("{base}/terminal");
                        ui::success(&format!("Opening web terminal at {url}"));
                        if !open_in_browser(&url) {
                            ui::hint(&format!("Could not open browser. Visit: {url}"));
                        }
                    } else {
                        ui::error("Daemon is not running. Start it with: captain start");
                    }
                }
                LaunchChoice::Chat => {
                    ui::hint("Starting chat session...");
                    ui::blank();
                    super::chat::cmd_quick_chat(None, None, false);
                }
            }
        }
        InitResult::Cancelled => {
            println!("  Setup cancelled.");
        }
    }
}

fn launch_desktop_app(_captain_dir: &Path) {
    let desktop_bin = {
        let exe = std::env::current_exe().ok();
        let dir = exe.as_ref().and_then(|e| e.parent());

        #[cfg(windows)]
        let name = "captain-desktop.exe";
        #[cfg(not(windows))]
        let name = "captain-desktop";

        dir.map(|d| d.join(name))
    };

    match desktop_bin {
        Some(ref path) if path.exists() => {
            ui::success("Launching Captain Desktop...");
            match std::process::Command::new(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    ui::success("Desktop app started.");
                }
                Err(e) => {
                    ui::error(&format!("Failed to launch desktop app: {e}"));
                    ui::hint("Try: captain terminal");
                }
            }
        }
        _ => {
            ui::error("Desktop app not found.");
            ui::hint("Install it with: cargo install captain-desktop");
            ui::hint("Falling back to the web terminal...");
            ui::blank();
            if let Some(base) = find_daemon() {
                let url = format!("{base}/terminal");
                if !open_in_browser(&url) {
                    ui::hint("Could not open a browser automatically.");
                }
                ui::hint(&format!("Web terminal: {url}"));
            } else {
                ui::hint("Daemon is not running. Start it with: captain start");
                ui::hint("Then open: http://127.0.0.1:50051/terminal");
            }
        }
    }
}

pub(crate) fn detect_best_provider() -> (&'static str, &'static str, &'static str) {
    if codex_auth_available() {
        ui::success("Detected Codex (~/.codex/auth.json — abonnement ChatGPT)");
        return ("codex", "", "gpt-5.5");
    }

    let providers = provider_list();
    for (p, env_var, m, display) in &providers {
        if !env_var.is_empty() && std::env::var(env_var).is_ok() {
            ui::success(&format!("Detected {display} ({env_var})"));
            return (p, env_var, m);
        }
    }
    if std::env::var("GOOGLE_API_KEY").is_ok() {
        ui::success("Detected Gemini (GOOGLE_API_KEY)");
        return ("gemini", "GOOGLE_API_KEY", "gemini-2.5-flash");
    }
    if check_ollama_available() {
        ui::success("Detected Ollama running locally (no API key needed)");
        return ("ollama", "OLLAMA_API_KEY", "llama3.2");
    }
    ui::hint("Aucun provider LLM configuré");
    ui::hint("`captain login codex` pour utiliser ton abonnement ChatGPT (sans API key)");
    ui::hint("Codex reste le défaut Captain; l'auth sera demandée avant le premier appel LLM.");
    ("codex", "", "gpt-5.5")
}

pub(crate) fn codex_auth_available() -> bool {
    captain_runtime::model_catalog::read_codex_credential().is_some()
}

pub(crate) fn provider_list() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("codex", "", "gpt-5.5", "Codex (ChatGPT)"),
        ("groq", "GROQ_API_KEY", "llama-3.3-70b-versatile", "Groq"),
        ("gemini", "GEMINI_API_KEY", "gemini-2.5-flash", "Gemini"),
        ("deepseek", "DEEPSEEK_API_KEY", "deepseek-chat", "DeepSeek"),
        (
            "anthropic",
            "ANTHROPIC_API_KEY",
            "claude-sonnet-4-20250514",
            "Anthropic",
        ),
        ("openai", "OPENAI_API_KEY", "gpt-4o", "OpenAI"),
        (
            "openrouter",
            "OPENROUTER_API_KEY",
            "openrouter/google/gemini-2.5-flash",
            "OpenRouter",
        ),
    ]
}

pub(crate) fn check_ollama_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 11434)),
        std::time::Duration::from_millis(500),
    )
    .is_ok()
}

pub(crate) fn write_config_if_missing(
    captain_dir: &Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) {
    let config_path = captain_dir.join("config.toml");
    if config_path.exists() {
        ui::check_ok(&format!("Config already exists: {}", config_path.display()));
    } else {
        let default_config = format!(
            r#"# Captain Agent OS configuration
# See https://captain.sh/docs for documentation

# For VPS/container installs, change to "0.0.0.0:50051" or set CAPTAIN_LISTEN.
api_listen = "127.0.0.1:50051"

[default_model]
provider = "{provider}"
model = "{model}"
api_key_env = "{api_key_env}"

[memory]
decay_rate = 0.05
"#
        );
        std::fs::write(&config_path, &default_config).unwrap_or_else(|e| {
            ui::error_with_fix("Failed to write config", &e.to_string());
            std::process::exit(1);
        });
        restrict_file_permissions(&config_path);
        ui::success(&format!("Created: {}", config_path.display()));
    }
}

pub(crate) fn write_default_model_config(
    captain_dir: &Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> Result<(), String> {
    let config_path = captain_dir.join("config.toml");
    let patches = vec![
        captain_runtime::integrations::ConfigPatch {
            path: vec!["default_model".to_string()],
            key: "provider".to_string(),
            value: toml_edit::value(provider),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["default_model".to_string()],
            key: "model".to_string(),
            value: toml_edit::value(model),
        },
        captain_runtime::integrations::ConfigPatch {
            path: vec!["default_model".to_string()],
            key: "api_key_env".to_string(),
            value: toml_edit::value(api_key_env),
        },
    ];
    captain_runtime::integrations::apply_config_patch(&config_path, &patches)?;
    restrict_file_permissions(&config_path);
    Ok(())
}
