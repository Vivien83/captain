//! Strict lifecycle for hidden sessions owned by background automations.

use crate::error::KernelError;
use crate::kernel::CaptainKernel;
use captain_types::agent::{AgentId, SessionId, AUTOMATION_SESSION_LABEL_PREFIX};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;

pub(crate) struct AutomationSessionGuard {
    kernel: Arc<CaptainKernel>,
    agent_id: AgentId,
    session_id: SessionId,
    label: String,
    cleaned: bool,
}

impl AutomationSessionGuard {
    pub(crate) fn create(
        kernel: Arc<CaptainKernel>,
        agent_id: AgentId,
    ) -> Result<Self, KernelError> {
        let label = format!("{AUTOMATION_SESSION_LABEL_PREFIX}{}", uuid::Uuid::new_v4());
        let session = kernel
            .memory
            .create_session_with_label(agent_id, Some(&label))
            .map_err(KernelError::Captain)?;
        Ok(Self {
            kernel,
            agent_id,
            session_id: session.id,
            label,
            cleaned: false,
        })
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.session_id
    }

    fn cleanup(&mut self) -> Result<bool, KernelError> {
        if self.cleaned {
            return Ok(false);
        }
        if self
            .kernel
            .registry
            .get(self.agent_id)
            .map(|entry| entry.session_id == self.session_id)
            .unwrap_or(false)
        {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                "Automation session became active; refusing automatic deletion"
            );
            return Ok(false);
        }
        if self.kernel.running_tasks.contains_key(&self.agent_id) {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                "Automation agent task is still running; deferring session cleanup"
            );
            return Ok(false);
        }
        let Some(session) = self
            .kernel
            .memory
            .get_session(self.session_id)
            .map_err(KernelError::Captain)?
        else {
            self.cleaned = true;
            return Ok(false);
        };
        if session.agent_id != self.agent_id
            || session.label.as_deref() != Some(self.label.as_str())
            || automation_label_run_id(&self.label).is_none()
        {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                "Automation session provenance changed; refusing automatic deletion"
            );
            return Ok(false);
        }
        self.kernel
            .memory
            .delete_session(self.session_id)
            .map_err(KernelError::Captain)?;
        self.cleaned = true;
        Ok(true)
    }
}

impl Drop for AutomationSessionGuard {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                error = %error,
                "Automation session cleanup deferred until restart"
            );
        }
    }
}

fn automation_label_run_id(label: &str) -> Option<uuid::Uuid> {
    label
        .strip_prefix(AUTOMATION_SESSION_LABEL_PREFIX)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
}

pub(crate) fn reconcile_abandoned_automation_sessions(
    kernel: &CaptainKernel,
) -> Result<usize, KernelError> {
    let active_sessions = kernel
        .registry
        .list()
        .into_iter()
        .map(|entry| entry.session_id)
        .collect::<HashSet<_>>();
    let mut removed = 0;

    for row in kernel
        .memory
        .list_sessions_including_internal()
        .map_err(KernelError::Captain)?
    {
        let Some(label) = row.get("label").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if automation_label_run_id(label).is_none() {
            continue;
        }
        let Some(session_id) = row
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(SessionId)
        else {
            continue;
        };
        if active_sessions.contains(&session_id) {
            continue;
        }
        let Some(session) = kernel
            .memory
            .get_session(session_id)
            .map_err(KernelError::Captain)?
        else {
            continue;
        };
        if session.label.as_deref() != Some(label)
            || kernel.running_tasks.contains_key(&session.agent_id)
        {
            continue;
        }
        kernel
            .memory
            .delete_session(session_id)
            .map_err(KernelError::Captain)?;
        removed += 1;
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;

    fn boot_test_kernel(name: &str) -> (tempfile::TempDir, Arc<CaptainKernel>, AgentId) {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join(name);
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).expect("kernel boot"));
        let agent_id = kernel
            .registry
            .list()
            .into_iter()
            .next()
            .expect("principal agent")
            .id;
        (tmp, kernel, agent_id)
    }

    #[test]
    fn automation_guard_deletes_only_its_created_session() {
        let (_tmp, kernel, agent_id) = boot_test_kernel("automation-guard-cleanup");
        let user_session = kernel
            .memory
            .create_session_with_label(agent_id, Some("Session utilisateur"))
            .unwrap();
        let automation_session_id = {
            let guard = AutomationSessionGuard::create(Arc::clone(&kernel), agent_id).unwrap();
            let session_id = guard.session_id();
            assert!(kernel.memory.get_session(session_id).unwrap().is_some());
            session_id
        };

        assert!(kernel
            .memory
            .get_session(automation_session_id)
            .unwrap()
            .is_none());
        assert!(kernel
            .memory
            .get_session(user_session.id)
            .unwrap()
            .is_some());
        kernel.shutdown();
    }

    #[test]
    fn automation_guard_refuses_session_with_changed_provenance() {
        let (_tmp, kernel, agent_id) = boot_test_kernel("automation-guard-provenance");
        let session_id = {
            let guard = AutomationSessionGuard::create(Arc::clone(&kernel), agent_id).unwrap();
            let session_id = guard.session_id();
            kernel
                .memory
                .set_session_label(session_id, Some("Session reprise par utilisateur"))
                .unwrap();
            session_id
        };

        assert!(kernel.memory.get_session(session_id).unwrap().is_some());
        kernel.shutdown();
    }

    #[tokio::test]
    async fn automation_guard_defers_cleanup_while_agent_task_is_running() {
        let (_tmp, kernel, agent_id) = boot_test_kernel("automation-guard-running");
        let guard = AutomationSessionGuard::create(Arc::clone(&kernel), agent_id).unwrap();
        let session_id = guard.session_id();
        let task = tokio::spawn(std::future::pending::<()>());
        kernel.running_tasks.insert(
            agent_id,
            crate::kernel::RunningTaskHandle {
                run_id: uuid::Uuid::new_v4(),
                abort_handle: task.abort_handle(),
                started_at: chrono::Utc::now(),
            },
        );

        drop(guard);
        assert!(kernel.memory.get_session(session_id).unwrap().is_some());

        kernel.running_tasks.remove(&agent_id);
        task.abort();
        let _ = task.await;
        assert_eq!(reconcile_abandoned_automation_sessions(&kernel).unwrap(), 1);
        assert!(kernel.memory.get_session(session_id).unwrap().is_none());
        kernel.shutdown();
    }

    #[test]
    fn restart_cleanup_removes_only_abandoned_valid_automation_sessions() {
        let (_tmp, kernel, agent_id) = boot_test_kernel("automation-restart-cleanup");
        let user_session = kernel
            .memory
            .create_session_with_label(agent_id, Some("Projet utilisateur"))
            .unwrap();
        let stale_label = format!("{AUTOMATION_SESSION_LABEL_PREFIX}{}", uuid::Uuid::new_v4());
        let stale = kernel
            .memory
            .create_session_with_label(agent_id, Some(&stale_label))
            .unwrap();
        let malformed = kernel
            .memory
            .create_session_with_label(agent_id, Some(".captain-internal/automation/not-a-run-id"))
            .unwrap();
        let active_label = format!("{AUTOMATION_SESSION_LABEL_PREFIX}{}", uuid::Uuid::new_v4());
        let active = kernel
            .memory
            .create_session_with_label(agent_id, Some(&active_label))
            .unwrap();
        kernel
            .switch_agent_session(agent_id, active.id)
            .expect("internal session can be made active only by trusted code");

        assert_eq!(reconcile_abandoned_automation_sessions(&kernel).unwrap(), 1);
        assert!(kernel.memory.get_session(stale.id).unwrap().is_none());
        assert!(kernel
            .memory
            .get_session(user_session.id)
            .unwrap()
            .is_some());
        assert!(kernel.memory.get_session(malformed.id).unwrap().is_some());
        assert!(kernel.memory.get_session(active.id).unwrap().is_some());
        kernel.shutdown();
    }
}
