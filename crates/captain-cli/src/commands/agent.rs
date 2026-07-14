use std::path::PathBuf;

use captain_types::agent::{AgentId, AgentManifest};

use crate::{
    agent_api_sheet::AgentApiSpawnSheet, boot_kernel, daemon_client, daemon_json, find_daemon,
    prompt_input, templates, tui, ui,
};

pub(crate) fn cmd_agent_spawn(config: Option<PathBuf>, manifest_path: PathBuf) {
    if !manifest_path.exists() {
        ui::error_with_fix(
            &format!("Manifest file not found: {}", manifest_path.display()),
            "Use `captain agent new` to spawn from a template instead",
        );
        std::process::exit(1);
    }

    let contents = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        eprintln!("Error reading manifest: {e}");
        std::process::exit(1);
    });

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": contents}))
                .send(),
        );
        if body.get("agent_id").is_some() {
            println!("Agent spawned successfully!");
            println!("  ID:   {}", body["agent_id"].as_str().unwrap_or("?"));
            println!("  Name: {}", body["name"].as_str().unwrap_or("?"));
            print_spawn_agent_api_provisioning(&body);
        } else {
            eprintln!(
                "Failed to spawn agent: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        let manifest: AgentManifest = toml::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("Error parsing manifest: {e}");
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(manifest) {
            Ok(id) => {
                println!("Agent spawned (in-process mode).");
                println!("  ID: {id}");
                println!("\n  Note: Agent will be lost when this process exits.");
                println!("  For persistent agents, use `captain start` first.");
            }
            Err(e) => {
                eprintln!("Failed to spawn agent: {e}");
                std::process::exit(1);
            }
        }
    }
}

pub(crate) fn cmd_agent_list(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/agents")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        match body.as_array() {
            Some(agents) if agents.is_empty() => println!("No agents running."),
            Some(agents) => {
                println!(
                    "{:<38} {:<16} {:<10} {:<12} MODEL",
                    "ID", "NAME", "STATE", "PROVIDER"
                );
                println!("{}", "-".repeat(95));
                for a in agents {
                    println!(
                        "{:<38} {:<16} {:<10} {:<12} {}",
                        a["id"].as_str().unwrap_or("?"),
                        a["name"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["model_provider"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    );
                }
            }
            None => println!("No agents running."),
        }
    } else {
        let kernel = boot_kernel(config);
        let agents = kernel.registry.list();

        if json {
            let list: Vec<serde_json::Value> = agents
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&list).unwrap_or_default()
            );
            return;
        }

        if agents.is_empty() {
            println!("No agents running.");
            return;
        }

        println!("{:<38} {:<20} {:<12} CREATED", "ID", "NAME", "STATE");
        println!("{}", "-".repeat(85));
        for entry in agents {
            println!(
                "{:<38} {:<20} {:<12} {}",
                entry.id,
                entry.name,
                format!("{:?}", entry.state),
                entry.created_at.format("%Y-%m-%d %H:%M")
            );
        }
    }
}

pub(crate) fn cmd_agent_chat(config: Option<PathBuf>, agent_id_str: &str) {
    tui::chat_runner::run_chat_tui(config, Some(agent_id_str.to_string()));
}

pub(crate) fn cmd_agent_kill(config: Option<PathBuf>, agent_id_str: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{agent_id_str}"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Agent {agent_id_str} killed.");
        } else {
            eprintln!(
                "Failed to kill agent: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        let agent_id: AgentId = agent_id_str.parse().unwrap_or_else(|_| {
            eprintln!("Invalid agent ID: {agent_id_str}");
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.kill_agent(agent_id) {
            Ok(()) => println!("Agent {agent_id} killed."),
            Err(e) => {
                eprintln!("Failed to kill agent: {e}");
                std::process::exit(1);
            }
        }
    }
}

pub(crate) fn cmd_agent_set(agent_id_str: &str, field: &str, value: &str) {
    match field {
        "model" => {
            if let Some(base) = find_daemon() {
                let client = daemon_client();
                let body = daemon_json(
                    client
                        .put(format!("{base}/api/agents/{agent_id_str}/model"))
                        .json(&serde_json::json!({"model": value}))
                        .send(),
                );
                if body.get("status").is_some() {
                    println!("Agent {agent_id_str} model set to {value}.");
                } else {
                    eprintln!(
                        "Failed to set model: {}",
                        body["error"].as_str().unwrap_or("Unknown error")
                    );
                    std::process::exit(1);
                }
            } else {
                eprintln!("No running daemon found. Start one with: captain start");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Unknown field: {field}. Supported fields: model");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_agent_new(config: Option<PathBuf>, template_name: Option<String>) {
    let all_templates = templates::load_all_templates();
    if all_templates.is_empty() {
        ui::error_with_fix(
            "No agent templates found",
            "Run `captain init` to set up the agents directory",
        );
        std::process::exit(1);
    }

    let chosen = match template_name {
        Some(ref name) => match all_templates.iter().find(|t| t.name == *name) {
            Some(t) => t,
            None => {
                ui::error_with_fix(
                    &format!("Template '{name}' not found"),
                    "Run `captain agent new` to see available templates",
                );
                std::process::exit(1);
            }
        },
        None => {
            ui::section("Available Agent Templates");
            ui::blank();
            for (i, t) in all_templates.iter().enumerate() {
                let desc = if t.description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", t.description)
                };
                println!(
                    "    {:>2}. {:<22}{}",
                    i + 1,
                    t.name,
                    colored::Colorize::dimmed(desc.as_str())
                );
            }
            ui::blank();
            let choice = prompt_input("  Choose template [1]: ");
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(all_templates.len() - 1)
            };
            &all_templates[idx]
        }
    };

    spawn_template_agent(config, chosen);
}

fn spawn_template_agent(config: Option<PathBuf>, template: &templates::AgentTemplate) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": template.content}))
                .send(),
        );
        if let Some(id) = body["agent_id"].as_str() {
            ui::blank();
            ui::success(&format!("Agent '{}' spawned", template.name));
            ui::kv("ID", id);
            if let Some(model) = body["model_name"].as_str() {
                let provider = body["model_provider"].as_str().unwrap_or("?");
                ui::kv("Model", &format!("{provider}/{model}"));
            }
            print_spawn_agent_api_provisioning(&body);
            ui::blank();
            ui::hint(&format!("Chat: captain chat {}", template.name));
        } else {
            ui::error(&format!(
                "Failed to spawn: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            ));
            std::process::exit(1);
        }
    } else {
        let manifest: AgentManifest = toml::from_str(&template.content).unwrap_or_else(|e| {
            ui::error_with_fix(
                &format!("Failed to parse template '{}': {e}", template.name),
                "The template manifest may be corrupted",
            );
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(manifest) {
            Ok(id) => {
                ui::blank();
                ui::success(&format!("Agent '{}' spawned (in-process)", template.name));
                ui::kv("ID", &id.to_string());
                ui::blank();
                ui::hint(&format!("Chat: captain chat {}", template.name));
                ui::hint("Note: Agent will be lost when this process exits");
                ui::hint("For persistent agents, use `captain start` first");
            }
            Err(e) => {
                ui::error(&format!("Failed to spawn agent: {e}"));
                std::process::exit(1);
            }
        }
    }
}

fn print_spawn_agent_api_provisioning(body: &serde_json::Value) {
    let Some(sheet) = AgentApiSpawnSheet::from_spawn_body(body) else {
        return;
    };
    ui::blank();
    ui::section("Agent API");
    for line in sheet.cli_lines() {
        if line.is_empty() {
            ui::blank();
        } else if let Some((key, value)) = line.split_once(": ") {
            ui::kv(key, value);
        } else {
            println!("  {line}");
        }
    }
}
