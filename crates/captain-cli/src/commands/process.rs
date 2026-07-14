use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

pub(crate) fn cmd_process_list(json: bool) {
    let base = require_daemon("process list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/status")).send());
    let processes = body["active_processes"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(processes)).unwrap_or_default()
        );
        return;
    }

    if processes.is_empty() {
        ui::success("No managed background processes.");
        return;
    }

    ui::section("Managed Background Processes");
    ui::blank();
    for process in processes {
        println!("{}", process_summary_line(&process));
    }
    ui::blank();
    ui::hint("Stop intentionally with: captain process kill <process_id>");
}

pub(crate) fn cmd_process_kill(process_id: &str) {
    let base = require_daemon("process kill");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/processes/{process_id}"))
            .send(),
    );
    if let Some(error) = body["error"].as_str() {
        ui::error(&format!("Process stop failed: {error}"));
        return;
    }
    ui::success(&format!("Process {process_id} stopped."));
    ui::hint("Run `captain status` to verify shutdown drain can continue.");
}

fn process_summary_line(process: &serde_json::Value) -> String {
    let id = process["id"].as_str().unwrap_or("?");
    let alive = process["alive"].as_bool().unwrap_or(false);
    let attached = process["attached"].as_bool().unwrap_or(true);
    let state = process_status_marker(alive, attached);
    let agent = process["agent_name"].as_str().unwrap_or("?");
    let command = truncate_display(process["command"].as_str().unwrap_or(""), 96);
    format!("    {id} -- {state} -- {agent} -- {command}")
}

fn process_status_marker(alive: bool, attached: bool) -> &'static str {
    match (alive, attached) {
        (true, true) => "alive",
        (true, false) => "recovered",
        (false, _) => "exited",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_summary_line_is_operator_safe() {
        let process = serde_json::json!({
            "id": "proc_7",
            "alive": true,
            "attached": true,
            "agent_name": "captain",
            "command": "npm run dev",
            "prompt": "private"
        });
        let line = process_summary_line(&process);

        assert!(line.contains("proc_7"));
        assert!(line.contains("alive"));
        assert!(line.contains("npm run dev"));
        assert!(!line.contains("private"));
    }

    #[test]
    fn process_summary_line_marks_recovered_processes() {
        let process = serde_json::json!({
            "id": "proc_8",
            "alive": true,
            "attached": false,
            "agent_name": "captain",
            "command": "python server.py"
        });
        let line = process_summary_line(&process);

        assert!(line.contains("recovered"));
    }

    #[test]
    fn process_status_marker_distinguishes_recovered_processes() {
        assert_eq!(process_status_marker(true, true), "alive");
        assert_eq!(process_status_marker(true, false), "recovered");
        assert_eq!(process_status_marker(false, true), "exited");
    }
}
