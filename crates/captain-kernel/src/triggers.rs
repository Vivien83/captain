//! Event-driven agent triggers — agents auto-activate when events match patterns.
//!
//! Agents register triggers that describe which events should wake them.
//! When a matching event arrives on the EventBus, the trigger system
//! sends the event content as a message to the subscribing agent.

use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, LifecycleEvent, SystemEvent};
pub use captain_types::event::{FileEventKind, TriggerId};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use notify_debouncer_mini::notify::{
    event::{ModifyKind, RenameMode},
    Config as NotifyConfig, Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode,
    Watcher,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// What kind of events a trigger matches on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPattern {
    /// Match any lifecycle event (agent spawned, started, terminated, etc.).
    Lifecycle,
    /// Match when a specific agent is spawned.
    AgentSpawned { name_pattern: String },
    /// Match when any agent is terminated.
    AgentTerminated,
    /// Match any system event.
    System,
    /// Match a specific system event by keyword.
    SystemKeyword { keyword: String },
    /// Match any memory update event.
    MemoryUpdate,
    /// Match memory updates for a specific key pattern.
    MemoryKeyPattern { key_pattern: String },
    /// Match all events (wildcard).
    All,
    /// Match custom events by content substring.
    ContentMatch { substring: String },
    /// Match messages from a specific channel (e.g., telegram, discord).
    ChannelMessage {
        /// Channel name (e.g., "telegram", "discord"). Empty = any channel.
        channel: String,
        /// Optional content filter. Empty = any message.
        #[serde(default)]
        contains: String,
    },
}

/// A registered trigger definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    /// Unique trigger ID.
    pub id: TriggerId,
    /// Which agent owns this trigger.
    pub agent_id: AgentId,
    /// The event pattern to match.
    pub pattern: TriggerPattern,
    /// Prompt template to send when triggered. Use `{{event}}` for event description.
    pub prompt_template: String,
    /// Whether this trigger is currently active.
    pub enabled: bool,
    /// When this trigger was created.
    pub created_at: DateTime<Utc>,
    /// How many times this trigger has fired.
    pub fire_count: u64,
    /// Maximum number of times this trigger can fire (0 = unlimited).
    pub max_fires: u64,
}

/// Partial update for a registered event trigger.
#[derive(Debug, Clone, Default)]
pub struct TriggerPatch {
    pub pattern: Option<TriggerPattern>,
    pub prompt_template: Option<String>,
    pub enabled: Option<bool>,
    pub max_fires: Option<u64>,
}

/// Default debounce for file-change triggers.
pub const DEFAULT_FILE_WATCH_DEBOUNCE_MS: u64 = 800;
/// Hard lower bound for file-change trigger debounce.
pub const MIN_FILE_WATCH_DEBOUNCE_MS: u64 = 200;
/// Hard upper bound for file-change trigger debounce.
pub const MAX_FILE_WATCH_DEBOUNCE_MS: u64 = 60_000;
/// Maximum trigger fires allowed in the rate window before auto-pause.
pub const FILE_WATCH_RATE_LIMIT_MAX_FIRES: usize = 10;
/// File-trigger rate-limit window in seconds.
pub const FILE_WATCH_RATE_LIMIT_WINDOW_SECS: u64 = 60;

/// A registered file-change trigger definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeTrigger {
    /// Unique trigger ID.
    pub id: TriggerId,
    /// Watched paths, canonicalized at creation time.
    pub paths: Vec<PathBuf>,
    /// Whether directories are watched recursively.
    pub recursive: bool,
    /// Which file event kinds should fire this trigger.
    pub events: Vec<FileEventKind>,
    /// Agent that receives the rendered prompt.
    pub agent_id: AgentId,
    /// Prompt template. Supports `{path}`, `{kind}`, and `{previous_path}`.
    pub prompt_template: String,
    /// Debounce window in milliseconds. Clamped to 200ms..60s.
    pub debounce_ms: u64,
    /// Whether this trigger is currently active.
    pub enabled: bool,
}

impl FileChangeTrigger {
    /// Return a copy with debounce clamped to the supported bounds.
    pub fn with_clamped_debounce(mut self) -> Self {
        self.debounce_ms = clamp_file_watch_debounce_ms(self.debounce_ms);
        self
    }

    /// True when this trigger should react to the observed file event kind.
    pub fn matches_kind(&self, kind: FileEventKind) -> bool {
        self.events.is_empty()
            || self.events.contains(&FileEventKind::Any)
            || self.events.contains(&kind)
    }

    /// True when the observed path is inside one of the watched roots.
    pub fn matches_path(&self, path: &Path) -> bool {
        self.paths
            .iter()
            .any(|root| path == root || path.starts_with(root))
    }
}

/// Clamp file-watch debounce to the supported guardrail range.
pub fn clamp_file_watch_debounce_ms(value: u64) -> u64 {
    value.clamp(MIN_FILE_WATCH_DEBOUNCE_MS, MAX_FILE_WATCH_DEBOUNCE_MS)
}

/// Canonicalize a watcher path, allowing the leaf to be missing.
///
/// `notify` accepts a path before its file exists (the OS watch is on the
/// parent directory anyway). When the leaf is missing, walk up to the first
/// existing ancestor, canonicalize that, and re-append the relative tail so
/// the resulting path is canonical for sandbox comparisons but still names
/// the not-yet-existing target.
pub fn canonicalize_or_lazy(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|e| format!("canonicalize failed for {}: {e}", path.display()));
    }
    let mut ancestor = path.parent();
    while let Some(parent) = ancestor {
        if parent.as_os_str().is_empty() {
            break;
        }
        if parent.exists() {
            let canon_parent = parent
                .canonicalize()
                .map_err(|e| format!("canonicalize failed for {}: {e}", parent.display()))?;
            let tail = path
                .strip_prefix(parent)
                .map_err(|_| "internal: lazy canonicalize tail extraction failed".to_string())?;
            return Ok(canon_parent.join(tail));
        }
        ancestor = parent.parent();
    }
    Err(format!(
        "no existing ancestor for {} (cannot canonicalize)",
        path.display()
    ))
}

/// A debounced file-system change emitted by a watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWatchSignal {
    pub trigger_id: TriggerId,
    pub path: PathBuf,
    pub kind: FileEventKind,
    pub previous_path: Option<PathBuf>,
}

/// Guard that keeps a file-change watcher alive until dropped.
pub struct FileChangeWatchGuard {
    _watcher: RecommendedWatcher,
    stop_tx: mpsc::Sender<()>,
    _worker: Option<JoinHandle<()>>,
}

impl Drop for FileChangeWatchGuard {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

/// The trigger engine manages event-to-agent routing.
pub struct TriggerEngine {
    /// All registered triggers.
    triggers: DashMap<TriggerId, Trigger>,
    /// Index: agent_id → list of trigger IDs belonging to that agent.
    agent_triggers: DashMap<AgentId, Vec<TriggerId>>,
    /// All registered file-change triggers.
    file_triggers: DashMap<TriggerId, FileChangeTrigger>,
    /// Index: agent_id → list of file trigger IDs belonging to that agent.
    file_agent_triggers: DashMap<AgentId, Vec<TriggerId>>,
    /// Persistence file for file-change triggers.
    file_trigger_persist_path: Option<PathBuf>,
    /// Per-trigger fire timestamps used for rate-limiting file-change bursts.
    file_fire_windows: DashMap<TriggerId, VecDeque<Instant>>,
}

impl TriggerEngine {
    /// Create a new trigger engine.
    pub fn new() -> Self {
        Self {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            file_triggers: DashMap::new(),
            file_agent_triggers: DashMap::new(),
            file_trigger_persist_path: None,
            file_fire_windows: DashMap::new(),
        }
    }

    /// Create a trigger engine with file-change trigger persistence enabled.
    pub fn with_file_trigger_persistence(home_dir: &Path) -> Self {
        let engine = Self {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            file_triggers: DashMap::new(),
            file_agent_triggers: DashMap::new(),
            file_trigger_persist_path: Some(home_dir.join("file_change_triggers.json")),
            file_fire_windows: DashMap::new(),
        };
        match engine.load_file_triggers() {
            Ok(count) if count > 0 => info!(count, "Loaded file-change triggers from disk"),
            Ok(_) => {}
            Err(e) => warn!("Failed to load file-change triggers: {e}"),
        }
        engine
    }

    /// Register a new trigger.
    pub fn register(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> TriggerId {
        let trigger = Trigger {
            id: TriggerId::new(),
            agent_id,
            pattern,
            prompt_template,
            enabled: true,
            created_at: Utc::now(),
            fire_count: 0,
            max_fires,
        };
        let id = trigger.id;
        self.triggers.insert(id, trigger);
        self.agent_triggers.entry(agent_id).or_default().push(id);

        info!(trigger_id = %id, agent_id = %agent_id, "Trigger registered");
        id
    }

    /// Remove a trigger.
    pub fn remove(&self, trigger_id: TriggerId) -> bool {
        if let Some((_, trigger)) = self.triggers.remove(&trigger_id) {
            if let Some(mut list) = self.agent_triggers.get_mut(&trigger.agent_id) {
                list.retain(|id| *id != trigger_id);
            }
            true
        } else {
            false
        }
    }

    /// Remove all triggers for an agent.
    pub fn remove_agent_triggers(&self, agent_id: AgentId) {
        if let Some((_, trigger_ids)) = self.agent_triggers.remove(&agent_id) {
            for id in trigger_ids {
                self.triggers.remove(&id);
            }
        }
        if let Some((_, trigger_ids)) = self.file_agent_triggers.remove(&agent_id) {
            for id in trigger_ids {
                self.file_triggers.remove(&id);
                self.file_fire_windows.remove(&id);
            }
            if let Err(e) = self.persist_file_triggers() {
                warn!("Failed to persist file triggers after agent removal: {e}");
            }
        }
    }

    /// Take all triggers for an agent, removing them from the engine.
    ///
    /// Returns the extracted triggers so they can be restored under a
    /// different agent ID via [`restore_triggers`]. This is used during
    /// hand reactivation: triggers must be saved before `kill_agent`
    /// destroys them, then restored with the new agent ID after spawn.
    pub fn take_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger> {
        let trigger_ids = self
            .agent_triggers
            .remove(&agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let mut taken = Vec::with_capacity(trigger_ids.len());
        for id in trigger_ids {
            if let Some((_, t)) = self.triggers.remove(&id) {
                taken.push(t);
            }
        }
        if !taken.is_empty() {
            info!(
                agent = %agent_id,
                count = taken.len(),
                "Took triggers for agent (pending reassignment)"
            );
        }
        taken
    }

    /// Take all file-change triggers for an agent, removing them from the engine.
    pub fn take_agent_file_triggers(&self, agent_id: AgentId) -> Vec<FileChangeTrigger> {
        let trigger_ids = self
            .file_agent_triggers
            .remove(&agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let mut taken = Vec::with_capacity(trigger_ids.len());
        for id in trigger_ids {
            self.file_fire_windows.remove(&id);
            if let Some((_, t)) = self.file_triggers.remove(&id) {
                taken.push(t);
            }
        }
        if !taken.is_empty() {
            if let Err(e) = self.persist_file_triggers() {
                warn!("Failed to persist file triggers after take: {e}");
            }
            info!(
                agent = %agent_id,
                count = taken.len(),
                "Took file-change triggers for agent"
            );
        }
        taken
    }

    /// Restore previously taken triggers under a new agent ID.
    ///
    /// Each trigger keeps its original pattern, prompt template, fire count,
    /// and max_fires, but is re-keyed to `new_agent_id`. New trigger IDs are
    /// generated so there are no stale references.
    ///
    /// Returns the number of triggers restored.
    pub fn restore_triggers(&self, new_agent_id: AgentId, triggers: Vec<Trigger>) -> usize {
        let count = triggers.len();
        for old in triggers {
            let new_id = TriggerId::new();
            let trigger = Trigger {
                id: new_id,
                agent_id: new_agent_id,
                pattern: old.pattern,
                prompt_template: old.prompt_template,
                enabled: old.enabled,
                created_at: old.created_at,
                fire_count: old.fire_count,
                max_fires: old.max_fires,
            };
            self.triggers.insert(new_id, trigger);
            self.agent_triggers
                .entry(new_agent_id)
                .or_default()
                .push(new_id);
        }
        if count > 0 {
            info!(
                agent = %new_agent_id,
                count,
                "Restored triggers under new agent"
            );
        }
        count
    }

    /// Restore previously taken file-change triggers under a new agent ID.
    pub fn restore_file_triggers(
        &self,
        new_agent_id: AgentId,
        triggers: Vec<FileChangeTrigger>,
    ) -> Result<Vec<FileChangeTrigger>, String> {
        let mut restored = Vec::with_capacity(triggers.len());
        for old in triggers {
            let mut trigger = old;
            trigger.id = TriggerId::new();
            trigger.agent_id = new_agent_id;
            trigger.debounce_ms = clamp_file_watch_debounce_ms(trigger.debounce_ms);
            let id = trigger.id;
            self.file_triggers.insert(id, trigger.clone());
            self.file_agent_triggers
                .entry(new_agent_id)
                .or_default()
                .push(id);
            restored.push(trigger);
        }
        if !restored.is_empty() {
            self.persist_file_triggers()?;
            info!(
                agent = %new_agent_id,
                count = restored.len(),
                "Restored file-change triggers under new agent"
            );
        }
        Ok(restored)
    }

    /// Reassign all triggers from one agent to another in place.
    ///
    /// Used during cold boot when the old agent ID (from persisted state) no
    /// longer exists and a new agent was spawned. Updates the `agent_id` field
    /// on each trigger and moves the index entry.
    ///
    /// Returns the number of triggers reassigned.
    pub fn reassign_agent_triggers(&self, old_agent_id: AgentId, new_agent_id: AgentId) -> usize {
        let trigger_ids = self
            .agent_triggers
            .remove(&old_agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let count = trigger_ids.len();
        for id in &trigger_ids {
            if let Some(mut t) = self.triggers.get_mut(id) {
                t.agent_id = new_agent_id;
            }
        }
        if !trigger_ids.is_empty() {
            self.agent_triggers
                .entry(new_agent_id)
                .or_default()
                .extend(trigger_ids);
            info!(
                old_agent = %old_agent_id,
                new_agent = %new_agent_id,
                count,
                "Reassigned triggers to new agent"
            );
        }
        let file_trigger_ids = self
            .file_agent_triggers
            .remove(&old_agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let file_count = file_trigger_ids.len();
        for id in &file_trigger_ids {
            if let Some(mut t) = self.file_triggers.get_mut(id) {
                t.agent_id = new_agent_id;
            }
            self.file_fire_windows.remove(id);
        }
        if !file_trigger_ids.is_empty() {
            self.file_agent_triggers
                .entry(new_agent_id)
                .or_default()
                .extend(file_trigger_ids);
            if let Err(e) = self.persist_file_triggers() {
                warn!("Failed to persist file triggers after agent reassignment: {e}");
            }
            info!(
                old_agent = %old_agent_id,
                new_agent = %new_agent_id,
                count = file_count,
                "Reassigned file-change triggers to new agent"
            );
        }
        count + file_count
    }

    /// Enable or disable a trigger. Returns true if the trigger was found.
    pub fn set_enabled(&self, trigger_id: TriggerId, enabled: bool) -> bool {
        if let Some(mut t) = self.triggers.get_mut(&trigger_id) {
            t.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Apply a partial update and return the refreshed trigger.
    pub fn update(&self, trigger_id: TriggerId, patch: TriggerPatch) -> Option<Trigger> {
        let mut trigger = self.triggers.get_mut(&trigger_id)?;
        if let Some(pattern) = patch.pattern {
            trigger.pattern = pattern;
        }
        if let Some(prompt_template) = patch.prompt_template {
            trigger.prompt_template = prompt_template;
        }
        if let Some(enabled) = patch.enabled {
            trigger.enabled = enabled;
        }
        if let Some(max_fires) = patch.max_fires {
            trigger.max_fires = max_fires;
        }
        Some(trigger.clone())
    }

    /// List all triggers for an agent.
    pub fn list_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger> {
        self.agent_triggers
            .get(&agent_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.triggers.get(id).map(|t| t.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all registered triggers.
    pub fn list_all(&self) -> Vec<Trigger> {
        self.triggers.iter().map(|e| e.value().clone()).collect()
    }

    /// Register a new file-change trigger.
    pub fn register_file_trigger(&self, trigger: FileChangeTrigger) -> Result<TriggerId, String> {
        let trigger = trigger.with_clamped_debounce();
        let id = trigger.id;
        if let Some((_, previous)) = self.file_triggers.remove(&id) {
            if let Some(mut list) = self.file_agent_triggers.get_mut(&previous.agent_id) {
                list.retain(|existing| *existing != id);
            }
        }
        self.file_triggers.insert(id, trigger.clone());
        let mut ids = self
            .file_agent_triggers
            .entry(trigger.agent_id)
            .or_default();
        if !ids.contains(&id) {
            ids.push(id);
        }
        drop(ids);
        self.persist_file_triggers()?;

        info!(
            trigger_id = %id,
            agent_id = %trigger.agent_id,
            paths = trigger.paths.len(),
            "File-change trigger registered"
        );
        Ok(id)
    }

    /// Remove a file-change trigger.
    pub fn remove_file_trigger(&self, trigger_id: TriggerId) -> Result<bool, String> {
        if let Some((_, trigger)) = self.file_triggers.remove(&trigger_id) {
            if let Some(mut list) = self.file_agent_triggers.get_mut(&trigger.agent_id) {
                list.retain(|id| *id != trigger_id);
            }
            self.file_fire_windows.remove(&trigger_id);
            self.persist_file_triggers()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Enable or disable a file-change trigger. Returns true if found.
    pub fn set_file_trigger_enabled(
        &self,
        trigger_id: TriggerId,
        enabled: bool,
    ) -> Result<bool, String> {
        if let Some(mut t) = self.file_triggers.get_mut(&trigger_id) {
            t.enabled = enabled;
            drop(t);
            self.persist_file_triggers()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List file-change triggers for an agent.
    pub fn list_agent_file_triggers(&self, agent_id: AgentId) -> Vec<FileChangeTrigger> {
        self.file_agent_triggers
            .get(&agent_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.file_triggers.get(id).map(|t| t.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all registered file-change triggers.
    pub fn list_file_triggers(&self) -> Vec<FileChangeTrigger> {
        self.file_triggers
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    /// Get a file-change trigger by ID.
    pub fn get_file_trigger(&self, trigger_id: TriggerId) -> Option<FileChangeTrigger> {
        self.file_triggers.get(&trigger_id).map(|t| t.clone())
    }

    /// Evaluate an event against all triggers. Returns a list of
    /// (agent_id, message_to_send) pairs for matching triggers.
    pub fn evaluate(&self, event: &Event) -> Vec<(AgentId, String)> {
        let event_description = describe_event(event);
        let mut matches = Vec::new();

        for mut entry in self.triggers.iter_mut() {
            let trigger = entry.value_mut();

            if !trigger.enabled {
                continue;
            }

            // Check max fires
            if trigger.max_fires > 0 && trigger.fire_count >= trigger.max_fires {
                trigger.enabled = false;
                continue;
            }

            if matches_pattern(&trigger.pattern, event, &event_description) {
                let message = trigger
                    .prompt_template
                    .replace("{{event}}", &event_description);
                matches.push((trigger.agent_id, message));
                trigger.fire_count += 1;

                info!(
                    trigger_id = %trigger.id,
                    agent_id = %trigger.agent_id,
                    fire_count = trigger.fire_count,
                    pattern = ?trigger.pattern,
                    "Event trigger fired"
                );
            }
        }

        if let EventPayload::System(SystemEvent::FileChanged {
            trigger_id,
            path,
            kind,
            previous_path,
        }) = &event.payload
        {
            if let Some((agent_id, message)) =
                self.evaluate_file_trigger_event(*trigger_id, path, *kind, previous_path.as_deref())
            {
                matches.push((agent_id, message));
            }
        }

        matches
    }

    /// Get a trigger by ID.
    pub fn get(&self, trigger_id: TriggerId) -> Option<Trigger> {
        self.triggers.get(&trigger_id).map(|t| t.clone())
    }

    fn evaluate_file_trigger_event(
        &self,
        trigger_id: TriggerId,
        path: &Path,
        kind: FileEventKind,
        previous_path: Option<&Path>,
    ) -> Option<(AgentId, String)> {
        let mut trigger = self.file_triggers.get_mut(&trigger_id)?;
        if !trigger.enabled || !trigger.matches_kind(kind) || !trigger.matches_path(path) {
            return None;
        }

        if self.file_trigger_rate_limited(trigger_id) {
            trigger.enabled = false;
            drop(trigger);
            if let Err(e) = self.persist_file_triggers() {
                warn!("Failed to persist file trigger throttle state: {e}");
            }
            warn!(
                trigger_id = %trigger_id,
                "File-change trigger auto-paused after rate-limit"
            );
            return None;
        }

        let previous = previous_path
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let path_display = path.display().to_string();
        let message = render_file_trigger_prompt(
            &trigger.prompt_template,
            &path_display,
            kind.as_str(),
            &previous,
        );
        let agent_id = trigger.agent_id;
        info!(
            trigger_id = %trigger_id,
            agent_id = %agent_id,
            path = %path_display,
            kind = kind.as_str(),
            "File-change trigger fired"
        );
        Some((agent_id, message))
    }

    fn file_trigger_rate_limited(&self, trigger_id: TriggerId) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(FILE_WATCH_RATE_LIMIT_WINDOW_SECS);
        let mut entry = self.file_fire_windows.entry(trigger_id).or_default();
        while entry
            .front()
            .is_some_and(|oldest| now.duration_since(*oldest) > window)
        {
            entry.pop_front();
        }
        if entry.len() >= FILE_WATCH_RATE_LIMIT_MAX_FIRES {
            true
        } else {
            entry.push_back(now);
            false
        }
    }

    fn load_file_triggers(&self) -> Result<usize, String> {
        let Some(path) = &self.file_trigger_persist_path else {
            return Ok(0);
        };
        if !path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file triggers: {e}"))?;
        let triggers: Vec<FileChangeTrigger> = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse file triggers: {e}"))?;
        let count = triggers.len();
        self.file_triggers.clear();
        self.file_agent_triggers.clear();
        let mut had_disable = false;
        for trigger in triggers {
            let mut trigger = trigger.with_clamped_debounce();
            // If any persisted path no longer exists, force-disable the trigger.
            // Notify cannot watch a vanished path; better surface the dead state
            // than leave the user thinking the watcher is armed.
            let mut missing: Vec<String> = Vec::new();
            for p in &trigger.paths {
                if !p.exists() {
                    missing.push(p.display().to_string());
                }
            }
            if !missing.is_empty() && trigger.enabled {
                warn!(
                    trigger_id = %trigger.id,
                    agent_id = %trigger.agent_id,
                    missing = ?missing,
                    "Persisted file-change trigger has vanished paths; disabling"
                );
                trigger.enabled = false;
                had_disable = true;
            }
            let id = trigger.id;
            self.file_agent_triggers
                .entry(trigger.agent_id)
                .or_default()
                .push(id);
            self.file_triggers.insert(id, trigger);
        }
        if had_disable {
            // Persist the corrected state so a manual edit of the JSON cannot
            // resurrect a dead trigger by accident.
            self.persist_file_triggers()?;
        }
        Ok(count)
    }

    fn persist_file_triggers(&self) -> Result<(), String> {
        let Some(path) = &self.file_trigger_persist_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create trigger directory: {e}"))?;
        }
        let triggers = self.list_file_triggers();
        let data = serde_json::to_string_pretty(&triggers)
            .map_err(|e| format!("Failed to serialize file triggers: {e}"))?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, data.as_bytes())
            .map_err(|e| format!("Failed to write file triggers temp file: {e}"))?;
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("Failed to rename file triggers file: {e}"))?;
        debug!(count = triggers.len(), "Persisted file-change triggers");
        Ok(())
    }
}

impl Default for TriggerEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn one OS watcher for a file-change trigger.
pub fn spawn_file_change_watcher(
    trigger: FileChangeTrigger,
    dispatch: Arc<dyn Fn(FileWatchSignal) + Send + Sync + 'static>,
) -> Result<FileChangeWatchGuard, String> {
    let trigger = trigger.with_clamped_debounce();
    if !trigger.enabled {
        return Err("file-change trigger is disabled".to_string());
    }
    if trigger.paths.is_empty() {
        return Err("file-change trigger has no watched paths".to_string());
    }

    let trigger_id = trigger.id;
    let debounce = Duration::from_millis(trigger.debounce_ms);
    let (event_tx, event_rx) =
        mpsc::channel::<Result<NotifyEvent, notify_debouncer_mini::notify::Error>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = event_tx.send(res);
        },
        NotifyConfig::default(),
    )
    .map_err(|e| format!("notify watcher init failed: {e}"))?;

    let mode = if trigger.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    for path in &trigger.paths {
        watcher
            .watch(path, mode)
            .map_err(|e| format!("notify watch failed for {}: {e}", path.display()))?;
    }

    let worker = std::thread::Builder::new()
        .name(format!("captain-file-trigger-{trigger_id}"))
        .spawn(move || {
            file_watch_worker(trigger_id, debounce, event_rx, stop_rx, dispatch);
        })
        .map_err(|e| format!("file watcher worker spawn failed: {e}"))?;

    Ok(FileChangeWatchGuard {
        _watcher: watcher,
        stop_tx,
        _worker: Some(worker),
    })
}

fn file_watch_worker(
    trigger_id: TriggerId,
    debounce: Duration,
    event_rx: mpsc::Receiver<Result<NotifyEvent, notify_debouncer_mini::notify::Error>>,
    stop_rx: mpsc::Receiver<()>,
    dispatch: Arc<dyn Fn(FileWatchSignal) + Send + Sync + 'static>,
) {
    let mut pending = Vec::new();
    loop {
        if stop_rx.try_recv().is_ok() {
            return;
        }

        match event_rx.recv_timeout(debounce) {
            Ok(Ok(event)) => {
                collect_file_watch_signal(trigger_id, event, &mut pending);
                drain_until_quiet(trigger_id, debounce, &event_rx, &stop_rx, &mut pending);
                for signal in dedupe_file_watch_signals(std::mem::take(&mut pending)) {
                    dispatch(signal);
                }
            }
            Ok(Err(e)) => warn!(error = ?e, "file-change watcher event error"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn drain_until_quiet(
    trigger_id: TriggerId,
    debounce: Duration,
    event_rx: &mpsc::Receiver<Result<NotifyEvent, notify_debouncer_mini::notify::Error>>,
    stop_rx: &mpsc::Receiver<()>,
    pending: &mut Vec<FileWatchSignal>,
) {
    loop {
        if stop_rx.try_recv().is_ok() {
            return;
        }
        match event_rx.recv_timeout(debounce) {
            Ok(Ok(event)) => collect_file_watch_signal(trigger_id, event, pending),
            Ok(Err(e)) => warn!(error = ?e, "file-change watcher event error"),
            Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return;
            }
        }
    }
}

fn collect_file_watch_signal(
    trigger_id: TriggerId,
    event: NotifyEvent,
    pending: &mut Vec<FileWatchSignal>,
) {
    if let Some(signal) = notify_event_to_file_signal(trigger_id, event) {
        pending.push(signal);
    }
}

fn notify_event_to_file_signal(
    trigger_id: TriggerId,
    event: NotifyEvent,
) -> Option<FileWatchSignal> {
    let (kind, path, previous_path) = match event.kind {
        EventKind::Access(_) => return None,
        EventKind::Create(_) => (FileEventKind::Create, event.paths.first()?.clone(), None),
        EventKind::Remove(_) => (FileEventKind::Remove, event.paths.first()?.clone(), None),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() >= 2 => (
            FileEventKind::Rename,
            event.paths[1].clone(),
            Some(event.paths[0].clone()),
        ),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            (FileEventKind::Remove, event.paths.first()?.clone(), None)
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            (FileEventKind::Create, event.paths.first()?.clone(), None)
        }
        EventKind::Modify(ModifyKind::Name(_)) => {
            (FileEventKind::Rename, event.paths.first()?.clone(), None)
        }
        EventKind::Modify(_) => (FileEventKind::Modify, event.paths.first()?.clone(), None),
        EventKind::Any | EventKind::Other => {
            (FileEventKind::Any, event.paths.first()?.clone(), None)
        }
    };
    Some(FileWatchSignal {
        trigger_id,
        path,
        kind,
        previous_path,
    })
}

fn dedupe_file_watch_signals(signals: Vec<FileWatchSignal>) -> Vec<FileWatchSignal> {
    let mut seen = HashSet::new();
    signals
        .into_iter()
        .filter(|signal| {
            seen.insert((
                signal.trigger_id,
                signal.path.clone(),
                signal.kind,
                signal.previous_path.clone(),
            ))
        })
        .collect()
}

fn render_file_trigger_prompt(
    template: &str,
    path: &str,
    kind: &str,
    previous_path: &str,
) -> String {
    template
        .replace("{{path}}", path)
        .replace("{path}", path)
        .replace("{{kind}}", kind)
        .replace("{kind}", kind)
        .replace("{{previous_path}}", previous_path)
        .replace("{previous_path}", previous_path)
}

/// Check if an event matches a trigger pattern.
fn matches_pattern(pattern: &TriggerPattern, event: &Event, description: &str) -> bool {
    match pattern {
        TriggerPattern::All => true,
        TriggerPattern::Lifecycle => {
            matches!(event.payload, EventPayload::Lifecycle(_))
        }
        TriggerPattern::AgentSpawned { name_pattern } => {
            if let EventPayload::Lifecycle(LifecycleEvent::Spawned { name, .. }) = &event.payload {
                name.contains(name_pattern.as_str()) || name_pattern == "*"
            } else {
                false
            }
        }
        TriggerPattern::AgentTerminated => matches!(
            event.payload,
            EventPayload::Lifecycle(LifecycleEvent::Terminated { .. })
                | EventPayload::Lifecycle(LifecycleEvent::Crashed { .. })
        ),
        TriggerPattern::System => {
            matches!(event.payload, EventPayload::System(_))
        }
        TriggerPattern::SystemKeyword { keyword } => {
            if let EventPayload::System(se) = &event.payload {
                let se_str = format!("{:?}", se).to_lowercase();
                se_str.contains(&keyword.to_lowercase())
            } else {
                false
            }
        }
        TriggerPattern::MemoryUpdate => {
            matches!(event.payload, EventPayload::MemoryUpdate(_))
        }
        TriggerPattern::MemoryKeyPattern { key_pattern } => {
            if let EventPayload::MemoryUpdate(delta) = &event.payload {
                delta.key.contains(key_pattern.as_str()) || key_pattern == "*"
            } else {
                false
            }
        }
        TriggerPattern::ContentMatch { substring } => description
            .to_lowercase()
            .contains(&substring.to_lowercase()),
        TriggerPattern::ChannelMessage { channel, contains } => {
            if let EventPayload::Message(msg) = &event.payload {
                let msg_channel = msg
                    .metadata
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let channel_match = channel.is_empty() || msg_channel.eq_ignore_ascii_case(channel);
                let content_match = contains.is_empty()
                    || msg
                        .content
                        .to_lowercase()
                        .contains(&contains.to_lowercase());
                channel_match && content_match
            } else {
                false
            }
        }
    }
}

/// Create a human-readable description of an event for use in prompts.
fn describe_event(event: &Event) -> String {
    match &event.payload {
        EventPayload::Message(msg) => {
            format!("Message from {:?}: {}", msg.role, msg.content)
        }
        EventPayload::ToolResult(tr) => {
            format!(
                "Tool '{}' {} ({}ms): {}",
                tr.tool_id,
                if tr.success { "succeeded" } else { "failed" },
                tr.execution_time_ms,
                captain_types::truncate_str(&tr.content, 200)
            )
        }
        EventPayload::MemoryUpdate(delta) => {
            format!(
                "Memory {:?} on key '{}' for agent {}",
                delta.operation, delta.key, delta.agent_id
            )
        }
        EventPayload::Lifecycle(le) => match le {
            LifecycleEvent::Spawned { agent_id, name } => {
                format!("Agent '{name}' (id: {agent_id}) was spawned")
            }
            LifecycleEvent::Started { agent_id } => {
                format!("Agent {agent_id} started")
            }
            LifecycleEvent::Suspended { agent_id } => {
                format!("Agent {agent_id} suspended")
            }
            LifecycleEvent::Resumed { agent_id } => {
                format!("Agent {agent_id} resumed")
            }
            LifecycleEvent::Terminated { agent_id, reason } => {
                format!("Agent {agent_id} terminated: {reason}")
            }
            LifecycleEvent::Crashed { agent_id, error } => {
                format!("Agent {agent_id} crashed: {error}")
            }
        },
        EventPayload::Network(ne) => {
            format!("Network event: {:?}", ne)
        }
        EventPayload::System(se) => match se {
            SystemEvent::KernelStarted => "Kernel started".to_string(),
            SystemEvent::KernelStopping => "Kernel stopping".to_string(),
            SystemEvent::QuotaWarning {
                agent_id,
                resource,
                usage_percent,
            } => format!("Quota warning: agent {agent_id}, {resource} at {usage_percent:.1}%"),
            SystemEvent::HealthCheck { status } => {
                format!("Health check: {status}")
            }
            SystemEvent::QuotaEnforced {
                agent_id,
                spent,
                limit,
            } => {
                format!("Quota enforced: agent {agent_id}, spent ${spent:.4} / ${limit:.4}")
            }
            SystemEvent::ModelRouted {
                agent_id,
                complexity,
                model,
            } => {
                format!("Model routed: agent {agent_id}, complexity={complexity}, model={model}")
            }
            SystemEvent::UserAction {
                user_id,
                action,
                result,
            } => {
                format!("User action: {user_id} {action} -> {result}")
            }
            SystemEvent::HealthCheckFailed {
                agent_id,
                unresponsive_secs,
            } => {
                format!(
                    "Health check failed: agent {agent_id}, unresponsive for {unresponsive_secs}s"
                )
            }
            SystemEvent::IntegrationConfigured { name } => {
                format!("Integration configured: {name}")
            }
            SystemEvent::FileChanged {
                trigger_id,
                path,
                kind,
                previous_path,
            } => {
                let previous = previous_path
                    .as_ref()
                    .map(|p| format!(", previous={}", p.display()))
                    .unwrap_or_default();
                format!(
                    "File changed: trigger={trigger_id}, kind={}, path={}{}",
                    kind.as_str(),
                    path.display(),
                    previous
                )
            }
            SystemEvent::TriggerThrottled { trigger_id, reason } => {
                format!("Trigger throttled: trigger={trigger_id}, reason={reason}")
            }
        },
        EventPayload::Custom(data) => {
            format!("Custom event ({} bytes)", data.len())
        }
        EventPayload::ChatStream(cs) => {
            format!("Chat stream event: {:?}", cs)
        }
        EventPayload::ToolRun(run) => {
            format!(
                "Tool run '{}' ({}) status={}",
                run.run_id, run.tool_name, run.status
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::event::*;

    #[test]
    fn test_register_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event occurred: {{event}}".to_string(),
            0,
        );
        assert!(engine.get(id).is_some());
    }

    #[test]
    fn test_evaluate_lifecycle() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine.register(
            watcher,
            TriggerPattern::Lifecycle,
            "Lifecycle: {{event}}".to_string(),
            0,
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "new-agent".to_string(),
            }),
        );

        let matches = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, watcher);
        assert!(matches[0].1.contains("new-agent"));
    }

    #[test]
    fn test_evaluate_agent_spawned_pattern() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine.register(
            watcher,
            TriggerPattern::AgentSpawned {
                name_pattern: "coder".to_string(),
            },
            "Coder spawned: {{event}}".to_string(),
            0,
        );

        // This should match
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "coder".to_string(),
            }),
        );
        assert_eq!(engine.evaluate(&event).len(), 1);

        // This should NOT match
        let event2 = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "researcher".to_string(),
            }),
        );
        assert_eq!(engine.evaluate(&event2).len(), 0);
    }

    #[test]
    fn test_max_fires() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            2, // max 2 fires
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // First two should match
        assert_eq!(engine.evaluate(&event).len(), 1);
        assert_eq!(engine.evaluate(&event).len(), 1);
        // Third should not
        assert_eq!(engine.evaluate(&event).len(), 0);
    }

    #[test]
    fn test_remove_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine.register(agent_id, TriggerPattern::All, "msg".to_string(), 0);
        assert!(engine.remove(id));
        assert!(engine.get(id).is_none());
    }

    #[test]
    fn test_update_trigger_patch() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine.register(agent_id, TriggerPattern::All, "old".to_string(), 0);
        let updated = engine
            .update(
                id,
                TriggerPatch {
                    pattern: Some(TriggerPattern::System),
                    prompt_template: Some("new".to_string()),
                    enabled: Some(false),
                    max_fires: Some(3),
                },
            )
            .unwrap();

        assert_eq!(updated.prompt_template, "new");
        assert!(!updated.enabled);
        assert_eq!(updated.max_fires, 3);
        assert!(matches!(updated.pattern, TriggerPattern::System));
    }

    #[test]
    fn test_remove_agent_triggers() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(agent_id, TriggerPattern::All, "a".to_string(), 0);
        engine.register(agent_id, TriggerPattern::System, "b".to_string(), 0);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 2);

        engine.remove_agent_triggers(agent_id);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 0);
    }

    #[test]
    fn test_content_match() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "quota".to_string(),
            },
            "Alert: {{event}}".to_string(),
            0,
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::System,
            EventPayload::System(SystemEvent::QuotaWarning {
                agent_id: AgentId::new(),
                resource: "tokens".to_string(),
                usage_percent: 85.0,
            }),
        );
        assert_eq!(engine.evaluate(&event).len(), 1);
    }

    // -- reassign_agent_triggers (#519) ------------------------------------

    #[test]
    fn test_reassign_agent_triggers_basic() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
        engine.register(old_agent, TriggerPattern::System, "b".to_string(), 0);

        let count = engine.reassign_agent_triggers(old_agent, new_agent);
        assert_eq!(count, 2);
        assert_eq!(engine.list_agent_triggers(old_agent).len(), 0);
        assert_eq!(engine.list_agent_triggers(new_agent).len(), 2);

        // Verify triggers actually fire for the new agent
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let matches = engine.evaluate(&event);
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|(id, _)| *id == new_agent));
    }

    #[test]
    fn test_reassign_agent_triggers_no_match_returns_zero() {
        let engine = TriggerEngine::new();
        let agent_a = AgentId::new();
        engine.register(agent_a, TriggerPattern::All, "a".to_string(), 0);

        let count = engine.reassign_agent_triggers(AgentId::new(), AgentId::new());
        assert_eq!(count, 0);
        // Original triggers untouched
        assert_eq!(engine.list_agent_triggers(agent_a).len(), 1);
    }

    #[test]
    fn test_reassign_does_not_touch_other_agents() {
        let engine = TriggerEngine::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let agent_c = AgentId::new();
        engine.register(agent_a, TriggerPattern::All, "a".to_string(), 0);
        engine.register(agent_b, TriggerPattern::System, "b".to_string(), 0);

        let count = engine.reassign_agent_triggers(agent_a, agent_c);
        assert_eq!(count, 1);
        // agent_b untouched
        assert_eq!(engine.list_agent_triggers(agent_b).len(), 1);
        assert_eq!(engine.list_agent_triggers(agent_c).len(), 1);
    }

    // -- take / restore triggers (#519) ------------------------------------

    #[test]
    fn test_take_and_restore_triggers() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        engine.register(
            old_agent,
            TriggerPattern::ContentMatch {
                substring: "deploy".to_string(),
            },
            "Deploy alert: {{event}}".to_string(),
            5,
        );
        engine.register(old_agent, TriggerPattern::Lifecycle, "lc".to_string(), 0);

        // Take triggers — engine should be empty for old agent
        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken.len(), 2);
        assert_eq!(engine.list_agent_triggers(old_agent).len(), 0);
        assert_eq!(engine.list_all().len(), 0);

        // Restore under new agent
        let restored = engine.restore_triggers(new_agent, taken);
        assert_eq!(restored, 2);
        assert_eq!(engine.list_agent_triggers(new_agent).len(), 2);

        // Verify patterns and max_fires are preserved
        let triggers = engine.list_agent_triggers(new_agent);
        let has_content_match = triggers.iter().any(|t| {
            matches!(&t.pattern, TriggerPattern::ContentMatch { substring } if substring == "deploy")
                && t.max_fires == 5
        });
        assert!(
            has_content_match,
            "ContentMatch trigger with max_fires=5 should be preserved"
        );
    }

    #[test]
    fn test_take_empty_returns_empty() {
        let engine = TriggerEngine::new();
        let taken = engine.take_agent_triggers(AgentId::new());
        assert!(taken.is_empty());
    }

    #[test]
    fn test_restore_preserves_enabled_state() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        let tid = engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
        engine.set_enabled(tid, false);

        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken.len(), 1);
        assert!(!taken[0].enabled);

        engine.restore_triggers(new_agent, taken);
        let restored = engine.list_agent_triggers(new_agent);
        assert_eq!(restored.len(), 1);
        assert!(
            !restored[0].enabled,
            "Disabled state should survive take/restore"
        );
    }

    #[test]
    fn test_file_change_trigger_dispatches_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let trigger = FileChangeTrigger {
            id: TriggerId::new(),
            paths: vec![dir.path().to_path_buf()],
            recursive: true,
            events: vec![FileEventKind::Modify],
            agent_id,
            prompt_template: "File {kind}: {path}".to_string(),
            debounce_ms: DEFAULT_FILE_WATCH_DEBOUNCE_MS,
            enabled: true,
        };
        let trigger_id = engine.register_file_trigger(trigger).unwrap();
        let changed_path = dir.path().join("agent.toml");
        let event = Event::new(
            AgentId::default(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::FileChanged {
                trigger_id,
                path: changed_path.clone(),
                kind: FileEventKind::Modify,
                previous_path: None,
            }),
        );

        let matches = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, agent_id);
        assert!(matches[0].1.contains("modify"));
        assert!(matches[0].1.contains(&changed_path.display().to_string()));
    }

    #[test]
    fn test_file_change_trigger_rate_limit_disables_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let engine = TriggerEngine::new();
        let trigger = FileChangeTrigger {
            id: TriggerId::new(),
            paths: vec![dir.path().to_path_buf()],
            recursive: true,
            events: vec![FileEventKind::Any],
            agent_id: AgentId::new(),
            prompt_template: "File {kind}: {path}".to_string(),
            debounce_ms: DEFAULT_FILE_WATCH_DEBOUNCE_MS,
            enabled: true,
        };
        let trigger_id = engine.register_file_trigger(trigger).unwrap();
        let event = Event::new(
            AgentId::default(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::FileChanged {
                trigger_id,
                path: dir.path().join("hot.txt"),
                kind: FileEventKind::Modify,
                previous_path: None,
            }),
        );

        for _ in 0..FILE_WATCH_RATE_LIMIT_MAX_FIRES {
            assert_eq!(engine.evaluate(&event).len(), 1);
        }
        assert_eq!(engine.evaluate(&event).len(), 0);
        assert!(!engine.get_file_trigger(trigger_id).unwrap().enabled);
    }

    #[test]
    fn test_file_trigger_prompt_supports_double_braces() {
        let rendered = render_file_trigger_prompt(
            "Changed {{kind}} {{path}} from {{previous_path}}",
            "/tmp/new",
            "rename",
            "/tmp/old",
        );
        assert_eq!(rendered, "Changed rename /tmp/new from /tmp/old");
    }

    #[test]
    fn test_canonicalize_or_lazy_resolves_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = canonicalize_or_lazy(dir.path()).unwrap();
        assert_eq!(resolved, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_canonicalize_or_lazy_supports_missing_leaf() {
        let dir = tempfile::tempdir().unwrap();
        let leaf = dir.path().join("not-yet-here.txt");
        assert!(!leaf.exists());
        let resolved = canonicalize_or_lazy(&leaf).unwrap();
        let expected = dir.path().canonicalize().unwrap().join("not-yet-here.txt");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_load_file_triggers_disables_vanished_paths() {
        let home = tempfile::tempdir().unwrap();
        let watch_dir = tempfile::tempdir().unwrap();
        let live_path = watch_dir.path().to_path_buf();
        let dead_path = std::path::PathBuf::from("/captain/test/this/path/does/not/exist");
        let agent_id = AgentId::new();

        let trigger_live = FileChangeTrigger {
            id: TriggerId::new(),
            paths: vec![live_path.clone()],
            recursive: true,
            events: vec![FileEventKind::Any],
            agent_id,
            prompt_template: "File {kind}: {path}".to_string(),
            debounce_ms: DEFAULT_FILE_WATCH_DEBOUNCE_MS,
            enabled: true,
        };
        let trigger_dead = FileChangeTrigger {
            id: TriggerId::new(),
            paths: vec![dead_path.clone()],
            recursive: true,
            events: vec![FileEventKind::Any],
            agent_id,
            prompt_template: "File {kind}: {path}".to_string(),
            debounce_ms: DEFAULT_FILE_WATCH_DEBOUNCE_MS,
            enabled: true,
        };

        // Persist both triggers via a primed engine so the JSON file exists.
        let engine = TriggerEngine::with_file_trigger_persistence(home.path());
        engine.register_file_trigger(trigger_live.clone()).unwrap();
        engine.register_file_trigger(trigger_dead.clone()).unwrap();

        // Build a fresh engine pointing at the same persistence file: the dead
        // path must be auto-disabled at load time, the live one must stay armed.
        let reloaded = TriggerEngine::with_file_trigger_persistence(home.path());
        let live = reloaded.get_file_trigger(trigger_live.id).unwrap();
        let dead = reloaded.get_file_trigger(trigger_dead.id).unwrap();
        assert!(live.enabled, "live trigger should remain enabled");
        assert!(!dead.enabled, "dead trigger must be auto-disabled at load");
    }
}
