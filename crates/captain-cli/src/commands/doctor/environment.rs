use crate::{cli_captain_home, production_credential_resolver_at, test_api_key_value, ui};

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

    let Some(resolver) = credential_resolver(report) else {
        return;
    };
    let mut any_key_set = false;
    for (env_var, name, provider_id) in &provider_keys {
        if let Some(key) = resolver.resolve(env_var) {
            let valid = test_api_key_value(provider_id, &key);
            if valid {
                if !report.json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !report.json {
                ui::check_warn(&format!("{name} ({env_var}) - key rejected (401/403)"));
            }
            any_key_set = true;
            report.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": if valid { "ok" } else { "warn" }, "live_test": true, "externally_managed": resolver.is_externally_managed(env_var)}));
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
    let Some(resolver) = credential_resolver(report) else {
        return;
    };
    for (env_var, name) in &channel_keys {
        if let Some(val) = resolver.resolve(env_var) {
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
            report.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": if format_ok { "ok" } else { "warn" }, "externally_managed": resolver.is_externally_managed(env_var)}));
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
    let Some(resolver) = credential_resolver(report) else {
        return;
    };
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
        if !val.is_empty() && !resolver.has_credential(val) {
            if !report.json {
                ui::check_warn(&format!(
                    "Config references {val} but no credential source can resolve it"
                ));
            }
            report.push(serde_json::json!({"check": "env_consistency", "status": "warn", "missing_var": val}));
        }
    }
}

pub(super) fn check_external_secret_sources(report: &mut DoctorReport) {
    let path = cli_captain_home()
        .join(captain_extensions::external_secret_sources::SECRET_SOURCES_FILENAME);
    let sources =
        match captain_extensions::external_secret_sources::ExternalSecretSources::load(&path) {
            Ok(sources) => sources,
            Err(error) => {
                if !report.json {
                    ui::check_fail(&format!("External secret registry is invalid: {error}"));
                }
                report.push(serde_json::json!({
                    "check": "external_secret_sources",
                    "status": "fail",
                    "error": error.to_string(),
                }));
                report.fail();
                return;
            }
        };
    let statuses = sources.statuses();
    if statuses.is_empty() {
        report.push(serde_json::json!({
            "check": "external_secret_sources",
            "status": "ok",
            "count": 0,
        }));
        return;
    }

    if !report.json {
        println!("\n  External Secret Sources:");
    }
    let mut all_ready = true;
    for status in &statuses {
        if status.ready {
            if !report.json {
                ui::check_ok(&format!("{} ({}) ready", status.key, status.source_type));
            }
        } else {
            all_ready = false;
            if !report.json {
                ui::check_fail(&format!(
                    "{} ({}) unavailable: {}",
                    status.key,
                    status.source_type,
                    status.error_code.as_deref().unwrap_or("source_unavailable")
                ));
            }
        }
    }
    report.push(serde_json::json!({
        "check": "external_secret_sources",
        "status": if all_ready { "ok" } else { "fail" },
        "count": statuses.len(),
        "sources": statuses,
    }));
    if !all_ready {
        report.fail();
    }
}

fn credential_resolver(
    report: &mut DoctorReport,
) -> Option<captain_extensions::credentials::CredentialResolver> {
    match production_credential_resolver_at(&cli_captain_home()) {
        Ok(resolver) => Some(resolver),
        Err(error) => {
            if !report.json {
                ui::check_fail(&format!("Secret sources unavailable: {error}"));
            }
            report.push(serde_json::json!({
                "check": "credential_resolution",
                "status": "fail",
                "error": error.to_string(),
            }));
            report.fail();
            None
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

    let posture = cfg.exec_policy.host_execution_posture();
    let mode = posture.policy_mode.as_str();
    let critical_mode = posture.critical_mode.as_str();
    let safe_bins_count = cfg.exec_policy.safe_bins.len();
    if !report.json {
        ui::check_ok(&format!(
            "Exec policy: {mode}/{critical_mode}, backend={}, isolation={}, os_isolation={}",
            posture.backend, posture.isolation_level, posture.os_isolation
        ));
        if posture.critical_mode == captain_types::config::CriticalMode::Open {
            ui::check_warn(
                "Critical mode is open: detected catastrophic commands may run after approval",
            );
        }
    }
    report.push(serde_json::json!({
        "check": "exec_policy",
        "status": "ok",
        "mode": mode,
        "critical_mode": critical_mode,
        "safe_bins": safe_bins_count,
        "backend": posture.backend,
        "isolation_level": posture.isolation_level,
        "os_isolation": posture.os_isolation,
        "dangerous_command_guard": posture.dangerous_command_guard,
    }));
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
                    captain_skills::verify::SkillVerifier::scan_prompt_content_advisory(prompt)
                        .findings
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
                "High-risk advisory phrase matches in {injection_warnings} skill(s); review required"
            ));
        }
        report.push(serde_json::json!({
            "check": "skill_prompt_advisory_scan",
            "status": "warn",
            "assurance": "advisory_heuristic",
            "warnings": injection_warnings
        }));
    } else {
        if !report.json {
            ui::check_ok(
                "Advisory skill phrase scan found no configured matches (not a security proof)",
            );
        }
        report.push(serde_json::json!({
            "check": "skill_prompt_advisory_scan",
            "status": "ok",
            "assurance": "advisory_heuristic"
        }));
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
