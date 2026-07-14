use captain_kernel::CaptainKernel;

pub(crate) fn active_work_deferred_text(
    kernel: &CaptainKernel,
    action_label: &str,
    retry_command: &str,
) -> Option<String> {
    active_work_deferred_text_for_count(
        crate::shutdown_guard::active_shutdown_work(kernel),
        action_label,
        retry_command,
    )
}

fn active_work_deferred_text_for_count(
    work: crate::shutdown_guard::ActiveShutdownWork,
    action_label: &str,
    retry_command: &str,
) -> Option<String> {
    if work.is_empty() {
        return None;
    }
    let active_runs = work.active_run_count;
    let active_processes = work.active_process_count;
    let total = work.total_count();
    Some(format!(
        "⏳ {action_label} différé: {total} travail/travaux actif(s) ({active_runs} run(s), {active_processes} process).\n\
         Captain ne coupe pas un travail actif sain.\n\
         Actions: lance `captain status`, attends la fin du travail, puis relance {retry_command}."
    ))
}

pub(crate) fn record_control_deferred(
    kernel: &CaptainKernel,
    command: &str,
    work: crate::shutdown_guard::ActiveShutdownWork,
) {
    kernel.audit_log.record(
        "system",
        captain_runtime::audit::AuditAction::ConfigChange,
        format!("{command} deferred because active work is running"),
        format!(
            "active_work_count={}, active_run_count={}, active_process_count={}",
            work.total_count(),
            work.active_run_count,
            work.active_process_count
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_control_defer_text_is_operator_safe() {
        let text = active_work_deferred_text_for_count(
            crate::shutdown_guard::ActiveShutdownWork::new(2, 1),
            "Restart",
            "`/restart`",
        )
        .unwrap();

        assert!(text.contains("3 travail/travaux actif(s)"));
        assert!(text.contains("2 run(s), 1 process"));
        assert!(text.contains("captain status"));
        assert!(text.contains("Captain ne coupe pas"));
        assert!(!text.contains("agent_id"));
        assert!(!text.contains("prompt"));
    }
}
