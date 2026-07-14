use captain_kernel::CaptainKernel;
use captain_types::agent::AgentEntry;

pub(crate) fn build_active_process_status(
    kernel: &CaptainKernel,
    registry_entries: &[AgentEntry],
) -> (Vec<serde_json::Value>, usize) {
    let mut active_processes: Vec<serde_json::Value> = kernel
        .process_manager
        .list_all()
        .into_iter()
        .map(|process| {
            let agent = registry_entries
                .iter()
                .find(|registered| registered.id.to_string() == process.agent_id);
            let operator_actions =
                process_operator_actions(&process.id, process.alive, process.attached);
            serde_json::json!({
                "id": process.id,
                "agent_id": process.agent_id,
                "agent_name": agent.map(|entry| entry.name.as_str()).unwrap_or("?"),
                "command": process.command,
                "alive": process.alive,
                "attached": process.attached,
                "pid": process.pid,
                "uptime_seconds": process.uptime_secs,
                "idle_seconds": process.idle_secs,
                "operator_actions": operator_actions,
            })
        })
        .collect();
    active_processes.sort_by(|a, b| {
        b["uptime_seconds"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["uptime_seconds"].as_u64().unwrap_or(0))
    });
    let active_process_count = active_processes
        .iter()
        .filter(|process| process["alive"].as_bool().unwrap_or(false))
        .count();
    (active_processes, active_process_count)
}

fn process_operator_actions(process_id: &str, alive: bool, attached: bool) -> Vec<String> {
    if alive && !attached {
        return vec![
            format!(
                "Process recovered after restart; stdout/stdin are detached. Inspect externally or stop intentionally with `captain process kill {process_id}`."
            ),
            "Retry `captain stop` after stopping the recovered process if shutdown is draining."
                .to_string(),
        ];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_process_operator_actions_are_actionable() {
        let actions = process_operator_actions("proc_9", true, false);

        assert_eq!(actions.len(), 2);
        assert!(actions[0].contains("captain process kill proc_9"));
        assert!(actions[0].contains("detached"));
    }

    #[test]
    fn attached_process_has_no_extra_operator_action() {
        assert!(process_operator_actions("proc_9", true, true).is_empty());
        assert!(process_operator_actions("proc_9", false, false).is_empty());
    }
}
