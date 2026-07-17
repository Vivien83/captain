use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) async fn tool_hand_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hands = kh.hand_list().await?;

    if hands.is_empty() {
        return Ok(
            "No Hands available. Install hands to enable curated autonomous packages.".to_string(),
        );
    }

    let mut lines = vec!["Available Hands:".to_string(), String::new()];
    for h in &hands {
        let icon = h["icon"].as_str().unwrap_or("");
        let name = h["name"].as_str().unwrap_or("?");
        let id = h["id"].as_str().unwrap_or("?");
        let status = h["status"].as_str().unwrap_or("unknown");
        let desc = h["description"].as_str().unwrap_or("");

        let status_marker = match status {
            "Active" => "[ACTIVE]",
            "Paused" => "[PAUSED]",
            _ => "[available]",
        };

        lines.push(format!("{} {} ({}) {}", icon, name, id, status_marker));
        if !desc.is_empty() {
            lines.push(format!("  {}", desc));
        }
        if let Some(iid) = h["instance_id"].as_str() {
            lines.push(format!("  Instance: {}", iid));
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

pub(crate) async fn tool_hand_activate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;
    let config: std::collections::HashMap<String, serde_json::Value> =
        if let Some(obj) = input["config"].as_object() {
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            std::collections::HashMap::new()
        };

    let result = kh.hand_activate(hand_id, config).await?;
    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let status = result["status"].as_str().unwrap_or("?");

    Ok(format!(
        "Hand '{}' activated!\n  Instance: {}\n  Agent: {} ({})",
        hand_id, instance_id, agent_name, status
    ))
}

pub(crate) async fn tool_hand_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;

    let result = kh.hand_status(hand_id).await?;
    let icon = result["icon"].as_str().unwrap_or("");
    let name = result["name"].as_str().unwrap_or(hand_id);
    let status = result["status"].as_str().unwrap_or("unknown");
    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let activated = result["activated_at"].as_str().unwrap_or("?");

    Ok(format!(
        "{} {} — {}\n  Instance: {}\n  Agent: {}\n  Activated: {}",
        icon, name, status, instance_id, agent_name, activated
    ))
}

pub(crate) async fn tool_hand_deactivate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let instance_id = input["instance_id"]
        .as_str()
        .ok_or("Missing 'instance_id' parameter")?;
    kh.hand_deactivate(instance_id).await?;
    Ok(format!("Hand instance '{}' deactivated.", instance_id))
}

pub(crate) async fn tool_scaffold_hand(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let id = input["id"].as_str().ok_or("Missing 'id'")?;
    let name = input["name"].as_str().ok_or("Missing 'name'")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description'")?;
    let category = input["category"].as_str().unwrap_or("personal");
    let icon = input["icon"].as_str().unwrap_or("🤖");
    let tools: Vec<String> = input["tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "shell_exec".into(),
                "file_read".into(),
                "file_write".into(),
                "memory_store".into(),
                "memory_recall".into(),
                "channel_send".into(),
            ]
        });

    let home = kernel
        .ok_or("No kernel handle")?
        .memory_recall("__home_dir")
        .unwrap_or(None)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".captain").to_string_lossy().to_string())
                .unwrap_or_else(|| "/tmp/.captain".to_string())
        });
    let home = std::path::Path::new(&home);

    let hand_dir = home
        .join("hands")
        .join(id.replace("-hand", "").replace("_hand", ""));
    captain_types::durable_fs::create_dir_all(&hand_dir)
        .map_err(|e| format!("mkdir hands: {e}"))?;

    let tools_toml = tools
        .iter()
        .map(|t| format!("\"{}\"", t))
        .collect::<Vec<_>>()
        .join(", ");
    let hand_toml = format!(
        "[hand]\nid = \"{id}\"\nname = \"{name}\"\ndescription = \"{description}\"\ncategory = \"{category}\"\nicon = \"{icon}\"\ntools = [{tools_toml}]\n"
    );
    captain_types::durable_fs::atomic_write(&hand_dir.join("HAND.toml"), hand_toml.as_bytes())
        .map_err(|e| format!("persist HAND.toml: {e}"))?;

    let ws_dir = home.join("workspaces").join(name);
    captain_types::durable_fs::create_dir_all(&ws_dir)
        .map_err(|e| format!("mkdir workspace: {e}"))?;
    for sub in &["data", "logs", "skills", "sessions", "output", "memory"] {
        let _ = captain_types::durable_fs::create_dir_all(&ws_dir.join(sub));
    }

    let soul = format!(
        "# Soul\nYou are **{name}**, an autonomous agent.\n{description}\n\n## Personality\n- Be helpful, proactive, and organized.\n- Respond in French by default.\n\n## Capabilities\n- Use your tools to accomplish tasks autonomously.\n- Persist durable facts through the memory tools, not workspace markdown snapshots.\n"
    );
    let _ = captain_types::durable_fs::atomic_write(&ws_dir.join("SOUL.md"), soul.as_bytes());

    let identity = format!(
        "---\nname: {name}\narchetype: {category}\nvibe: helpful\nemoji: {icon}\navatar_url:\ngreeting_style: warm\ncolor: #D4A853\n---\n# Identity\n{description}\n"
    );
    let _ =
        captain_types::durable_fs::atomic_write(&ws_dir.join("IDENTITY.md"), identity.as_bytes());

    Ok(format!(
        "Hand '{name}' scaffolded:\n- HAND.toml: {}\n- Workspace: {}\n\nFiles created: SOUL.md, IDENTITY.md\nSubdirs: data/, logs/, skills/, sessions/, output/, memory/\n\nNext: customize SOUL.md/IDENTITY.md if needed, then activate via the Hands page.",
        hand_dir.display(),
        ws_dir.display()
    ))
}
