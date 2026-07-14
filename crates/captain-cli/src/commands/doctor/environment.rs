use crate::{cli_captain_home, test_api_key, ui};

use super::DoctorReport;

pub(super) fn check_providers(report: &mut DoctorReport) {
    if !report.json {
        println!("\n  LLM Providers:");
    }
    let provider_keys = [
        ("GROQ_API_KEY", "Groq", "groq"),
        ("OPENROUTER_API_KEY", "OpenRouter", "openrouter"),
        ("ANTHROPIC_API_KEY", "Anthropic", "anthropic"),
        ("OPENAI_API_KEY", "OpenAI", "openai"),
        ("DEEPSEEK_API_KEY", "DeepSeek", "deepseek"),
        ("GEMINI_API_KEY", "Gemini", "gemini"),
        ("GOOGLE_API_KEY", "Google", "google"),
        ("TOGETHER_API_KEY", "Together", "together"),
        ("MISTRAL_API_KEY", "Mistral", "mistral"),
        ("FIREWORKS_API_KEY", "Fireworks", "fireworks"),
    ];

    let mut any_key_set = false;
    for (env_var, name, provider_id) in &provider_keys {
        if std::env::var(env_var).is_ok() {
            let valid = test_api_key(provider_id, env_var);
            if valid {
                if !report.json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !report.json {
                ui::check_warn(&format!("{name} ({env_var}) - key rejected (401/403)"));
            }
            any_key_set = true;
            report.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": if valid { "ok" } else { "warn" }, "live_test": !valid}));
        } else {
            if !report.json {
                ui::provider_status(name, env_var, false);
            }
            report.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    if !any_key_set {
        if !report.json {
            println!();
            ui::check_fail("No LLM provider API keys found!");
            ui::blank();
            ui::section("Getting an API key (free tiers)");
            ui::suggest_cmd("Groq:", "https://console.groq.com       (free, fast)");
            ui::suggest_cmd("Gemini:", "https://aistudio.google.com    (free tier)");
            ui::suggest_cmd("DeepSeek:", "https://platform.deepseek.com  (low cost)");
            ui::blank();
            ui::hint("Or run: captain config set-key groq");
        }
        report.fail();
    }
}

pub(super) fn check_channels(report: &mut DoctorReport) {
    if !report.json {
        println!("\n  Channel Integrations:");
    }
    let channel_keys = [
        ("TELEGRAM_BOT_TOKEN", "Telegram"),
        ("DISCORD_BOT_TOKEN", "Discord"),
        ("EMAIL_PASSWORD", "Email"),
    ];
    for (env_var, name) in &channel_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            let val = std::env::var(env_var).unwrap_or_default();
            let format_ok = match *env_var {
                "TELEGRAM_BOT_TOKEN" => val.contains(':'),
                "DISCORD_BOT_TOKEN" => val.len() > 50,
                _ => true,
            };
            if format_ok {
                if !report.json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !report.json {
                ui::check_warn(&format!("{name} ({env_var}) - unexpected token format"));
            }
            report.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": if format_ok { "ok" } else { "warn" }}));
        } else {
            if !report.json {
                ui::provider_status(name, env_var, false);
            }
            report.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }
}

pub(super) fn check_env_consistency(report: &mut DoctorReport) {
    let config_path = cli_captain_home().join("config.toml");
    if !config_path.exists() {
        return;
    }
    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
    for line in config_str.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("api_key_env") else {
            continue;
        };
        let Some(val_part) = rest.strip_prefix('=') else {
            continue;
        };
        let val = val_part.trim().trim_matches('"');
        if !val.is_empty() && std::env::var(val).is_err() {
            if !report.json {
                ui::check_warn(&format!(
                    "Config references {val} but it is not set in env or .env"
                ));
            }
            report.push(serde_json::json!({"check": "env_consistency", "status": "warn", "missing_var": val}));
        }
    }
}

pub(super) fn check_config_deserialization(report: &mut DoctorReport) {
    let config_path = cli_captain_home().join("config.toml");
    if !config_path.exists() {
        return;
    }
    if !report.json {
        println!("\n  Config Validation:");
    }
    let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
    match toml::from_str::<captain_types::config::KernelConfig>(&config_content) {
        Ok(cfg) => check_kernel_config(report, cfg),
        Err(e) => {
            if !report.json {
                ui::check_fail(&format!("Config fails KernelConfig deserialization: {e}"));
            }
            report.push(serde_json::json!({"check": "config_deser", "status": "fail", "error": e.to_string()}));
            report.fail();
        }
    }
}

fn check_kernel_config(report: &mut DoctorReport, cfg: captain_types::config::KernelConfig) {
    if !report.json {
        ui::check_ok("Config deserializes into KernelConfig");
    }
    report.push(serde_json::json!({"check": "config_deser", "status": "ok"}));

    let mode = format!("{:?}", cfg.exec_policy.mode);
    let safe_bins_count = cfg.exec_policy.safe_bins.len();
    if !report.json {
        ui::check_ok(&format!(
            "Exec policy: mode={mode}, safe_bins={safe_bins_count}"
        ));
    }
    report.push(serde_json::json!({"check": "exec_policy", "status": "ok", "mode": mode, "safe_bins": safe_bins_count}));
    check_includes(report, &cfg);
    check_mcp_servers(report, &cfg);
}

fn check_includes(report: &mut DoctorReport, cfg: &captain_types::config::KernelConfig) {
    if cfg.include.is_empty() {
        return;
    }
    let captain_dir = cli_captain_home();
    let mut include_ok = true;
    for inc in &cfg.include {
        let inc_path = captain_dir.join(inc);
        if inc_path.exists() {
            if !report.json {
                ui::check_ok(&format!("Include file: {inc}"));
            }
        } else if report.repair {
            if !report.json {
                ui::check_warn(&format!("Include file missing: {inc}"));
            }
            include_ok = false;
        } else {
            if !report.json {
                ui::check_fail(&format!("Include file not found: {inc}"));
            }
            include_ok = false;
            report.fail();
        }
    }
    report.push(serde_json::json!({"check": "config_includes", "status": if include_ok { "ok" } else { "fail" }, "count": cfg.include.len()}));
}

fn check_mcp_servers(report: &mut DoctorReport, cfg: &captain_types::config::KernelConfig) {
    if cfg.mcp_servers.is_empty() {
        return;
    }
    let mcp_count = cfg.mcp_servers.len();
    if !report.json {
        ui::check_ok(&format!("MCP servers configured: {mcp_count}"));
    }
    for server in &cfg.mcp_servers {
        match &server.transport {
            captain_types::config::McpTransportEntry::Stdio { command, .. } => {
                if command.is_empty() {
                    warn_mcp_server(report, &server.name);
                }
            }
            captain_types::config::McpTransportEntry::Sse { url } => {
                if url.is_empty() {
                    warn_mcp_server(report, &server.name);
                }
            }
        }
    }
    report.push(serde_json::json!({"check": "mcp_servers", "status": "ok", "count": mcp_count}));
}

fn warn_mcp_server(report: &mut DoctorReport, name: &str) {
    if !report.json {
        ui::check_warn(&format!("MCP server '{name}' has empty command or URL"));
    }
    report.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": name}));
}

pub(super) fn check_skills(report: &mut DoctorReport) {
    if !report.json {
        println!("\n  Skills:");
    }
    let skills_dir = cli_captain_home().join("skills");
    let mut skill_reg = captain_skills::registry::SkillRegistry::new(skills_dir.clone());
    skill_reg.load_bundled();
    let bundled_count = skill_reg.count();
    if !report.json {
        ui::check_ok(&format!("Bundled skills loaded: {bundled_count}"));
    }
    report.push(
        serde_json::json!({"check": "bundled_skills", "status": "ok", "count": bundled_count}),
    );

    if skills_dir.exists() {
        match skill_reg.load_workspace_skills(&skills_dir) {
            Ok(_) => {
                let ws_count = skill_reg.count().saturating_sub(bundled_count);
                if ws_count > 0 {
                    if !report.json {
                        ui::check_ok(&format!("Workspace skills loaded: {ws_count}"));
                    }
                    report.push(serde_json::json!({"check": "workspace_skills", "status": "ok", "count": ws_count}));
                }
            }
            Err(e) => {
                if !report.json {
                    ui::check_warn(&format!("Failed to load workspace skills: {e}"));
                }
                report.push(serde_json::json!({"check": "workspace_skills", "status": "warn", "error": e.to_string()}));
            }
        }
    }

    let injection_warnings = skill_reg
        .list()
        .iter()
        .filter(|skill| {
            skill
                .manifest
                .prompt_context
                .as_ref()
                .map(|prompt| {
                    captain_skills::verify::SkillVerifier::scan_prompt_content(prompt)
                        .iter()
                        .any(|w| {
                            matches!(
                                w.severity,
                                captain_skills::verify::WarningSeverity::Critical
                            )
                        })
                })
                .unwrap_or(false)
        })
        .count();
    if injection_warnings > 0 {
        if !report.json {
            ui::check_warn(&format!(
                "Prompt injection warnings in {injection_warnings} skill(s)"
            ));
        }
        report.push(serde_json::json!({"check": "skill_injection_scan", "status": "warn", "warnings": injection_warnings}));
    } else {
        if !report.json {
            ui::check_ok("All skills pass prompt injection scan");
        }
        report.push(serde_json::json!({"check": "skill_injection_scan", "status": "ok"}));
    }
}

pub(super) fn check_extensions(report: &mut DoctorReport) {
    if !report.json {
        println!("\n  Extensions:");
    }
    let captain_dir = cli_captain_home();
    let mut ext_registry = captain_extensions::registry::IntegrationRegistry::new(&captain_dir);
    ext_registry.load_bundled();
    let _ = ext_registry.load_installed();
    let template_count = ext_registry.template_count();
    let installed_count = ext_registry.installed_count();
    if !report.json {
        ui::check_ok(&format!(
            "Available integration templates: {template_count}"
        ));
        ui::check_ok(&format!("Installed integrations: {installed_count}"));
    }
    report.push(serde_json::json!({"check": "extensions_available", "status": "ok", "count": template_count}));
    report.push(serde_json::json!({"check": "extensions_installed", "status": "ok", "count": installed_count}));
}
