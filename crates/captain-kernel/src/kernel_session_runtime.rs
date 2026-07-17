use crate::error::{KernelError, KernelResult};

use super::CaptainKernel;
use captain_types::agent::{AgentEntry, AgentId, SessionId};
use captain_types::error::CaptainError;
use captain_types::message::{Message, MessageContent, Role};
use tracing::{debug, info};

impl CaptainKernel {
    fn update_agent_session_id(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<()> {
        self.registry
            .update_session_id(agent_id, session_id)
            .map_err(KernelError::Captain)?;

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        self.memory
            .save_agent(&entry)
            .map_err(KernelError::Captain)?;
        Ok(())
    }

    /// Reset an agent's session by saving a short memory summary and switching
    /// the registry to a fresh session ID. The previous session stays durable
    /// and can be reopened explicitly.
    pub fn reset_session(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        if let Ok(Some(old_session)) = self.memory.get_session(entry.session_id) {
            if old_session.messages.len() >= 2 {
                self.save_session_summary(agent_id, &entry, &old_session);
            }
        }

        let new_session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::Captain)?;

        self.update_agent_session_id(agent_id, new_session.id)?;
        self.scheduler.reset_usage(agent_id);

        info!(agent_id = %agent_id, "Session reset (previous session preserved)");
        Ok(())
    }

    /// Clear all conversation history for an agent and create a fresh active
    /// session so the agent remains usable.
    pub fn clear_agent_history(&self, agent_id: AgentId) -> KernelResult<()> {
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let _ = self.memory.delete_agent_sessions(agent_id);
        let _ = self.memory.delete_canonical_session(agent_id);
        let new_session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::Captain)?;

        self.update_agent_session_id(agent_id, new_session.id)?;

        info!(agent_id = %agent_id, "All agent history cleared");
        Ok(())
    }

    /// List all sessions for a specific agent and mark the active one.
    pub fn list_agent_sessions(&self, agent_id: AgentId) -> KernelResult<Vec<serde_json::Value>> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut sessions = self
            .memory
            .list_agent_sessions(agent_id)
            .map_err(KernelError::Captain)?;

        for session in &mut sessions {
            if let Some(obj) = session.as_object_mut() {
                let is_active = obj
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|sid| sid == entry.session_id.0.to_string())
                    .unwrap_or(false);
                obj.insert("active".to_string(), serde_json::json!(is_active));
            }
        }

        Ok(sessions)
    }

    /// Create a new named session for an agent and switch the agent onto it.
    pub fn create_agent_session(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> KernelResult<serde_json::Value> {
        self.create_agent_session_with_activation(agent_id, label, true)
    }

    /// Create a persisted session without changing the agent's global active
    /// session. This is the safe primitive for independent Web/API clients.
    pub fn create_agent_session_detached(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> KernelResult<serde_json::Value> {
        self.create_agent_session_with_activation(agent_id, label, false)
    }

    fn create_agent_session_with_activation(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
        activate: bool,
    ) -> KernelResult<serde_json::Value> {
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .create_session_with_label(agent_id, label)
            .map_err(KernelError::Captain)?;

        if activate {
            self.update_agent_session_id(agent_id, session.id)?;
        }

        info!(agent_id = %agent_id, label = ?label, active = activate, "Created new session");

        Ok(serde_json::json!({
            "session_id": session.id.0.to_string(),
            "label": session.label,
            "active": activate,
        }))
    }

    pub(super) fn resolve_agent_session_entry(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
    ) -> KernelResult<AgentEntry> {
        let mut entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        let Some(session_id) = session_id else {
            return Ok(entry);
        };

        let session = self
            .memory
            .get_session(session_id)
            .map_err(KernelError::Captain)?
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::InvalidInput(
                    "Requested session was not found".to_string(),
                ))
            })?;
        if session.agent_id != agent_id {
            return Err(KernelError::Captain(CaptainError::InvalidInput(
                "Requested session belongs to a different agent".to_string(),
            )));
        }

        entry.session_id = session_id;
        Ok(entry)
    }

    /// Resume an agent's conversation from an externally stored history.
    pub fn restore_agent_session(
        &self,
        agent_id: AgentId,
        messages: Vec<Message>,
    ) -> KernelResult<SessionId> {
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::Captain)?;
        let restored_count = messages.len();
        session.messages = messages;
        self.memory
            .save_session(&session)
            .map_err(KernelError::Captain)?;

        self.update_agent_session_id(agent_id, session.id)?;

        info!(
            agent_id = %agent_id,
            session_id = %session.id.0,
            restored_messages = restored_count,
            "Session restored from external snapshot"
        );
        Ok(session.id)
    }

    /// Switch an agent to an existing session by session ID.
    pub fn switch_agent_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<()> {
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(session_id)
            .map_err(KernelError::Captain)?
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::Internal("Session not found".to_string()))
            })?;

        if session.agent_id != agent_id {
            return Err(KernelError::Captain(CaptainError::Internal(
                "Session belongs to a different agent".to_string(),
            )));
        }

        self.update_agent_session_id(agent_id, session_id)?;

        info!(agent_id = %agent_id, session_id = %session_id.0, "Switched session");
        Ok(())
    }

    fn save_session_summary(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session: &captain_memory::session::Session,
    ) {
        let recent = &session.messages[session.messages.len().saturating_sub(10)..];
        let topics: Vec<&str> = recent
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| match &m.content {
                MessageContent::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();

        if topics.is_empty() {
            return;
        }

        let slug: String = topics[0]
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .take(60)
            .collect();

        let date = chrono::Utc::now().format("%Y-%m-%d");
        let summary = format!(
            "Session on {date}: {slug}\n\nKey exchanges:\n{}",
            topics
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, topic)| {
                    let truncated = captain_types::truncate_str(topic, 200);
                    format!("{}. {}", i + 1, truncated)
                })
                .collect::<Vec<_>>()
                .join("\n")
        );

        let key = format!("session_{date}_{slug}");
        let _ =
            self.memory
                .structured_set(agent_id, &key, serde_json::Value::String(summary.clone()));

        if let Some(ref workspace) = entry.manifest.workspace {
            let mem_dir = workspace.join("memory");
            let filename = format!("{date}-{slug}.md");
            let _ = captain_types::durable_fs::atomic_write(
                &mem_dir.join(&filename),
                summary.as_bytes(),
            );
        }

        debug!(
            agent_id = %agent_id,
            key = %key,
            "Saved session summary to memory before reset"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;
    use std::collections::HashMap;

    #[test]
    fn test_reset_session_persists_new_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-reset-session-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");

        let original_session = kernel
            .registry
            .get(agent_id)
            .expect("agent entry")
            .session_id;

        kernel.reset_session(agent_id).expect("reset succeeds");

        let entry_after = kernel.registry.get(agent_id).expect("agent entry");
        assert_ne!(
            entry_after.session_id, original_session,
            "reset must switch to a fresh session"
        );

        let persisted_after = kernel
            .memory
            .load_agent(agent_id)
            .expect("agent load")
            .expect("agent persisted");
        assert_eq!(
            persisted_after.session_id, entry_after.session_id,
            "persisted agent row must match registry after reset"
        );
        assert!(
            kernel
                .memory
                .get_session(original_session)
                .expect("old session lookup")
                .is_some(),
            "reset must preserve the previous session so it can be reopened"
        );
        assert!(
            kernel
                .memory
                .get_session(entry_after.session_id)
                .expect("new session lookup")
                .is_some(),
            "reset must create the new active session"
        );

        kernel.shutdown();
    }

    #[test]
    fn detached_session_does_not_switch_the_agent_and_can_be_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-detached-session-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");
        let active_before = kernel.registry.get(agent_id).unwrap().session_id;

        let created = kernel
            .create_agent_session_detached(agent_id, Some("Web isolated"))
            .expect("detached session");
        let detached_id = created["session_id"]
            .as_str()
            .unwrap()
            .parse::<uuid::Uuid>()
            .map(SessionId)
            .unwrap();

        assert_eq!(created["active"], false);
        assert_eq!(
            kernel.registry.get(agent_id).unwrap().session_id,
            active_before
        );
        let scoped = kernel
            .resolve_agent_session_entry(agent_id, Some(detached_id))
            .expect("session scope resolves");
        assert_eq!(scoped.session_id, detached_id);
        assert_eq!(
            kernel.registry.get(agent_id).unwrap().session_id,
            active_before
        );

        kernel.shutdown();
    }

    #[test]
    fn scoped_session_rejects_another_agent_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-session-owner-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let first = kernel
            .activate_hand("browser", HashMap::new())
            .expect("first hand activates")
            .agent_id
            .unwrap();
        let second_session = kernel
            .memory
            .create_session(AgentId::new())
            .expect("foreign session persists")
            .id;

        let error = kernel
            .resolve_agent_session_entry(first, Some(second_session))
            .expect_err("cross-agent session must fail");
        assert!(error.to_string().contains("different agent"));

        kernel.shutdown();
    }

    #[test]
    fn test_restore_agent_session_writes_messages_and_switches() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-restore-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");

        let original_session = kernel
            .registry
            .get(agent_id)
            .expect("agent entry")
            .session_id;

        let history = vec![
            Message::user("hier on parlait du projet alpha"),
            Message::assistant("oui - j'avais retenu PostgreSQL 16"),
            Message::user("on continue dessus"),
        ];

        let new_session_id = kernel
            .restore_agent_session(agent_id, history.clone())
            .expect("restore succeeds");

        assert_ne!(
            new_session_id, original_session,
            "restore must switch to a fresh session, not mutate the previous one"
        );

        let entry_after = kernel.registry.get(agent_id).expect("agent entry");
        assert_eq!(
            entry_after.session_id, new_session_id,
            "registry must point at the restored session"
        );

        let session = kernel
            .memory
            .get_session(new_session_id)
            .expect("session lookup")
            .expect("session exists");
        assert_eq!(session.messages.len(), history.len());
        assert_eq!(session.messages[0].role, Role::User);
        assert_eq!(session.messages[1].role, Role::Assistant);

        let persisted_after = kernel
            .memory
            .load_agent(agent_id)
            .expect("agent load")
            .expect("agent persisted");
        assert_eq!(
            persisted_after.session_id, new_session_id,
            "restored session switch must survive daemon restart"
        );

        kernel.shutdown();
    }

    #[test]
    fn test_restore_agent_session_rejects_unknown_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-restore-unknown");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

        let bogus = AgentId(uuid::Uuid::new_v4());
        let res = kernel.restore_agent_session(bogus, vec![]);
        assert!(res.is_err(), "restoring on a missing agent must fail");

        kernel.shutdown();
    }
}
