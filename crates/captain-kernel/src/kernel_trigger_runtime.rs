use crate::error::{KernelError, KernelResult};
use crate::triggers::{
    canonicalize_or_lazy, spawn_file_change_watcher, FileChangeTrigger, FileWatchSignal,
    TriggerPatch, TriggerPattern,
};

use super::CaptainKernel;
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use captain_types::event::{
    Event, EventPayload, EventTarget, FileEventKind, SystemEvent, TriggerId,
};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

impl CaptainKernel {
    /// Publish an event to the bus and evaluate triggers.
    ///
    /// Any matching triggers will dispatch messages to the subscribing agents.
    /// Returns the list of (agent_id, message) pairs that were triggered.
    pub async fn publish_event(&self, event: Event) -> Vec<(AgentId, String)> {
        let triggered = self.triggers.evaluate(&event);
        self.event_bus.publish(event).await;

        if !triggered.is_empty() {
            let graph = self.graph_memory.clone();
            let trigger_data: Vec<(String, String)> = triggered
                .iter()
                .map(|(aid, msg)| {
                    (
                        aid.to_string(),
                        if msg.len() > 100 {
                            format!("{}…", &msg[..97])
                        } else {
                            msg.clone()
                        },
                    )
                })
                .collect();
            tokio::spawn(async move {
                for (aid, msg) in &trigger_data {
                    let _ = graph.record_event(
                        "_sys::trigger_fire",
                        &format!("trigger→{}", aid),
                        vec![
                            ("agent_id", aid.as_str()),
                            ("message_preview", msg.as_str()),
                        ],
                        None,
                    );
                }
                let _ = graph.save();
            });
        }

        if let Some(weak) = self.self_handle.get() {
            if !triggered.is_empty() {
                info!(
                    count = triggered.len(),
                    "Dispatching trigger fires to agents"
                );
            }
            for (agent_id, message) in &triggered {
                if let Some(kernel) = weak.upgrade() {
                    let aid = *agent_id;
                    let msg = message.clone();
                    let preview: String = msg.chars().take(120).collect();
                    debug!(
                        agent = %aid,
                        preview = %preview,
                        "Dispatching trigger message"
                    );
                    tokio::spawn(async move {
                        if let Err(e) = kernel.send_message(aid, &msg).await {
                            warn!(agent = %aid, "Trigger dispatch failed: {e}");
                        }
                    });
                }
            }
        }

        triggered
    }

    /// Register a trigger for an agent.
    pub fn register_trigger(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> KernelResult<TriggerId> {
        if self.registry.get(agent_id).is_none() {
            return Err(KernelError::Captain(CaptainError::AgentNotFound(
                agent_id.to_string(),
            )));
        }
        Ok(self
            .triggers
            .register(agent_id, pattern, prompt_template, max_fires))
    }

    /// Remove a trigger by ID.
    pub fn remove_trigger(&self, trigger_id: TriggerId) -> bool {
        self.triggers.remove(trigger_id)
    }

    /// Enable or disable a trigger. Returns true if found.
    pub fn set_trigger_enabled(&self, trigger_id: TriggerId, enabled: bool) -> bool {
        self.triggers.set_enabled(trigger_id, enabled)
    }

    /// Apply a partial trigger update and return the refreshed trigger.
    pub fn update_trigger(
        &self,
        trigger_id: TriggerId,
        patch: TriggerPatch,
    ) -> Option<crate::triggers::Trigger> {
        self.triggers.update(trigger_id, patch)
    }

    /// List all triggers, optionally filtered by agent.
    pub fn list_triggers(&self, agent_id: Option<AgentId>) -> Vec<crate::triggers::Trigger> {
        match agent_id {
            Some(id) => self.triggers.list_agent_triggers(id),
            None => self.triggers.list_all(),
        }
    }

    /// Register and arm a file-change trigger for an agent.
    pub fn register_file_change_trigger(
        &self,
        trigger: FileChangeTrigger,
    ) -> KernelResult<TriggerId> {
        if self.registry.get(trigger.agent_id).is_none() {
            return Err(KernelError::Captain(CaptainError::AgentNotFound(
                trigger.agent_id.to_string(),
            )));
        }

        let trigger = self.prepare_file_change_trigger(trigger)?;
        let id = trigger.id;
        self.triggers
            .register_file_trigger(trigger.clone())
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e)))?;

        if let Err(e) = self.arm_file_change_trigger(trigger) {
            let _ = self.triggers.remove_file_trigger(id);
            return Err(KernelError::Captain(CaptainError::Internal(e)));
        }

        Ok(id)
    }

    /// Remove a file-change trigger by ID.
    pub fn remove_file_change_trigger(&self, trigger_id: TriggerId) -> KernelResult<bool> {
        self.drop_file_watcher(trigger_id);
        self.triggers
            .remove_file_trigger(trigger_id)
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e)))
    }

    /// Enable or disable a file-change trigger.
    pub fn set_file_change_trigger_enabled(
        &self,
        trigger_id: TriggerId,
        enabled: bool,
    ) -> KernelResult<bool> {
        let updated = self
            .triggers
            .set_file_trigger_enabled(trigger_id, enabled)
            .map_err(|e| KernelError::Captain(CaptainError::Internal(e)))?;
        if !updated {
            return Ok(false);
        }

        if enabled {
            if let Some(trigger) = self.triggers.get_file_trigger(trigger_id) {
                if let Err(e) = self.arm_file_change_trigger(trigger) {
                    let _ = self.triggers.set_file_trigger_enabled(trigger_id, false);
                    self.drop_file_watcher(trigger_id);
                    return Err(KernelError::Captain(CaptainError::Internal(e)));
                }
            }
        } else {
            self.drop_file_watcher(trigger_id);
        }

        Ok(true)
    }

    /// List all file-change triggers, optionally filtered by agent.
    pub fn list_file_change_triggers(&self, agent_id: Option<AgentId>) -> Vec<FileChangeTrigger> {
        match agent_id {
            Some(id) => self.triggers.list_agent_file_triggers(id),
            None => self.triggers.list_file_triggers(),
        }
    }

    /// Publish a file-change signal into the normal event/trigger dispatch path.
    pub async fn publish_file_change_signal(&self, signal: FileWatchSignal) {
        let owner_agent = self
            .triggers
            .get_file_trigger(signal.trigger_id)
            .map(|trigger| trigger.agent_id)
            .unwrap_or_default();
        let target = if owner_agent == AgentId::default() {
            EventTarget::Broadcast
        } else {
            EventTarget::Agent(owner_agent)
        };
        let event = Event::new(
            owner_agent,
            target,
            EventPayload::System(SystemEvent::FileChanged {
                trigger_id: signal.trigger_id,
                path: signal.path,
                kind: signal.kind,
                previous_path: signal.previous_path,
            }),
        );
        self.publish_event(event).await;
    }

    fn prepare_file_change_trigger(
        &self,
        mut trigger: FileChangeTrigger,
    ) -> KernelResult<FileChangeTrigger> {
        trigger.debounce_ms = crate::triggers::clamp_file_watch_debounce_ms(trigger.debounce_ms);
        if trigger.paths.is_empty() {
            return Err(KernelError::Captain(CaptainError::Internal(
                "file-change trigger requires at least one path".to_string(),
            )));
        }
        if trigger.events.is_empty() {
            trigger.events.push(FileEventKind::Any);
        }

        let blocked_canon: Vec<PathBuf> = self
            .blocked_workspace_paths()
            .into_iter()
            .map(|blocked| blocked.canonicalize().unwrap_or(blocked))
            .collect();

        let mut paths = Vec::with_capacity(trigger.paths.len());
        for path in trigger.paths {
            let canonical = canonicalize_or_lazy(&path).map_err(|e| {
                KernelError::Captain(CaptainError::Internal(format!(
                    "invalid file-change trigger path {}: {e}",
                    path.display()
                )))
            })?;
            for blocked in &blocked_canon {
                if canonical == *blocked || canonical.starts_with(blocked) {
                    return Err(KernelError::Captain(CaptainError::Internal(format!(
                        "refused file-change trigger path {}: inside protected zone {}",
                        canonical.display(),
                        blocked.display()
                    ))));
                }
            }
            paths.push(canonical);
        }
        trigger.paths = paths;
        Ok(trigger)
    }

    pub(super) fn arm_persisted_file_watchers(self: &Arc<Self>) {
        let all = self.triggers.list_file_triggers();
        let mut armed = 0usize;
        let mut skipped_disabled = 0usize;
        let mut failed = 0usize;
        for trigger in all.iter() {
            if !trigger.enabled {
                skipped_disabled += 1;
                continue;
            }
            match self.arm_file_change_trigger(trigger.clone()) {
                Ok(()) => armed += 1,
                Err(e) => {
                    failed += 1;
                    warn!(
                        trigger_id = %trigger.id,
                        agent_id = %trigger.agent_id,
                        paths = ?trigger.paths,
                        error = %e,
                        "Failed to arm persisted file-change trigger; auto-disabling"
                    );
                    let _ = self.triggers.set_file_trigger_enabled(trigger.id, false);
                }
            }
        }
        if !all.is_empty() {
            info!(
                total = all.len(),
                armed, skipped_disabled, failed, "Armed persisted file-change triggers at boot"
            );
        }
    }

    pub(super) fn arm_file_change_trigger(&self, trigger: FileChangeTrigger) -> Result<(), String> {
        if !trigger.enabled {
            return Ok(());
        }
        let weak = self
            .self_handle
            .get()
            .cloned()
            .ok_or_else(|| "kernel self handle is not initialized".to_string())?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|e| format!("file-change trigger needs an active tokio runtime: {e}"))?;
        let trigger_id = trigger.id;
        let dispatch: Arc<dyn Fn(FileWatchSignal) + Send + Sync + 'static> =
            Arc::new(move |signal| {
                if let Some(kernel) = weak.upgrade() {
                    runtime.spawn(async move {
                        kernel.publish_file_change_signal(signal).await;
                    });
                }
            });
        let guard = spawn_file_change_watcher(trigger, dispatch)?;
        let mut watchers = self.file_watchers.lock().unwrap_or_else(|e| e.into_inner());
        watchers.insert(trigger_id, guard);
        Ok(())
    }

    pub(super) fn drop_file_watcher(&self, trigger_id: TriggerId) {
        let mut watchers = self.file_watchers.lock().unwrap_or_else(|e| e.into_inner());
        watchers.remove(&trigger_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;

    fn file_trigger(path: PathBuf, debounce_ms: u64) -> FileChangeTrigger {
        FileChangeTrigger {
            id: TriggerId::new(),
            paths: vec![path],
            recursive: false,
            events: Vec::new(),
            agent_id: AgentId::new(),
            prompt_template: "changed {path}".to_string(),
            debounce_ms,
            enabled: false,
        }
    }

    #[test]
    fn prepare_file_change_trigger_defaults_event_and_clamps_debounce() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("home");
        let watched = tmp.path().join("watched");
        std::fs::create_dir_all(&home_dir).unwrap();
        std::fs::create_dir_all(&watched).unwrap();
        let config = KernelConfig {
            home_dir,
            data_dir: tmp.path().join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

        let prepared = kernel
            .prepare_file_change_trigger(file_trigger(watched.clone(), 1))
            .expect("file trigger prepared");

        assert_eq!(prepared.events, vec![FileEventKind::Any]);
        assert_eq!(
            prepared.debounce_ms,
            crate::triggers::MIN_FILE_WATCH_DEBOUNCE_MS
        );
        assert_eq!(prepared.paths, vec![watched.canonicalize().unwrap()]);

        kernel.shutdown();
    }

    #[test]
    fn prepare_file_change_trigger_rejects_protected_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("home");
        std::fs::create_dir_all(&home_dir).unwrap();
        let protected = home_dir.join("secrets.env");
        std::fs::write(&protected, "SECRET=value\n").unwrap();
        let config = KernelConfig {
            home_dir,
            data_dir: tmp.path().join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

        let err = kernel
            .prepare_file_change_trigger(file_trigger(protected, 800))
            .expect_err("protected path rejected");
        assert!(
            err.to_string().contains("protected zone"),
            "unexpected error: {err}"
        );

        kernel.shutdown();
    }
}
