//! Persistent process runtime handlers.

use crate::process_manager::ProcessManager;

use super::ensure_no_secret_literal;
use std::path::Path;

pub(crate) async fn tool_process_start(
    input: &serde_json::Value,
    pm: Option<&ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    ensure_no_secret_literal("process_start", "command", command)?;
    let args: Vec<String> = input["args"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for (idx, arg) in args.iter().enumerate() {
        ensure_no_secret_literal("process_start", &format!("args[{idx}]"), arg)?;
    }
    let cwd = input["cwd"]
        .as_str()
        .filter(|value| !value.trim().is_empty());
    if let Some(cwd) = cwd {
        ensure_no_secret_literal("process_start", "cwd", cwd)?;
    }

    let proc_id = pm
        .start_in_dir(agent_id, command, &args, cwd.map(Path::new))
        .await?;
    Ok(serde_json::json!({
        "process_id": proc_id,
        "status": "started",
        "cwd": cwd
    })
    .to_string())
}

pub(crate) async fn tool_process_poll(
    input: &serde_json::Value,
    pm: Option<&ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let (stdout, stderr) = pm.read(proc_id).await?;
    Ok(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
    })
    .to_string())
}

pub(crate) async fn tool_process_write(
    input: &serde_json::Value,
    pm: Option<&ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let data = input["data"].as_str().ok_or("Missing 'data' parameter")?;
    ensure_no_secret_literal("process_write", "data", data)?;
    let data = if data.ends_with('\n') {
        data.to_string()
    } else {
        format!("{data}\n")
    };
    pm.write(proc_id, &data).await?;
    Ok(r#"{"status": "written"}"#.to_string())
}

pub(crate) async fn tool_process_kill(
    input: &serde_json::Value,
    pm: Option<&ProcessManager>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    pm.kill(proc_id).await?;
    Ok(r#"{"status": "killed"}"#.to_string())
}

pub(crate) async fn tool_process_list(
    pm: Option<&ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let list: Vec<serde_json::Value> = pm
        .list(agent_id)
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "command": p.command,
                "alive": p.alive,
                "attached": p.attached,
                "pid": p.pid,
                "uptime_secs": p.uptime_secs,
                "idle_secs": p.idle_secs,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(list).to_string())
}
