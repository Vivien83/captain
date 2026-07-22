//! Ratatui TUI for Captain interactive mode.
//!
//! Two-level navigation: Phase::Boot (Welcome/Wizard) → Phase::Main with focused hubs.

mod agent_status;
mod automation_status;
pub mod branding;
pub mod chat_runner;
mod chrome;
mod command_args;
mod copy_slash_state;
mod decision_status;
mod default_agent;
pub mod diff_render;
mod error_route;
pub mod event;
mod event_memory;
mod event_messages;
mod event_native_capabilities;
mod event_stream;
mod file_upload;
mod hub_key_state;
mod hub_nav;
mod hub_slash_state;
mod hub_view_state;
pub mod image_preview;
mod input_state;
mod list_state;
pub mod markdown;
mod navigation_state;
pub(crate) mod provider_quota;
mod refresh_state;
mod render_state;
mod resource_status;
mod resume_prompt;
pub mod screens;
pub(crate) mod session_runtime;
pub mod session_store;
mod slash_attachment;
mod slash_command;
mod slash_daemon;
mod slash_exit;
mod slash_export;
mod slash_feedback;
mod slash_fortune;
mod slash_help;
mod slash_info;
mod slash_kill;
mod slash_local;
mod slash_model;
mod slash_project;
mod slash_reload;
mod slash_retry;
mod slash_scroll;
mod slash_session;
mod slash_standalone;
mod slash_think;
mod stream_lifecycle;
mod stream_route;
mod surface_slash_state;
mod tab_bar;
mod tab_state;
pub mod theme;
mod tick_state;
mod usage_slash_state;
mod welcome_route;

use captain_kernel::CaptainKernel;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use copy_slash_state::{copy_slash_target_for_arg, CopySlashTarget};
use event::{AppEvent, BackendRef};
use hub_key_state::{
    automation_key_route_for_view, automation_view_after_shortcut, capabilities_key_route_for_view,
    capabilities_view_after_shortcut, connections_key_route_for_view,
    connections_view_after_shortcut, learning_key_route_for_view, learning_view_after_shortcut,
    AutomationKeyRoute, CapabilitiesKeyRoute, ConnectionsKeyRoute, LearningKeyRoute,
};
use hub_slash_state::{hub_slash_route_for_command, HubSlashRoute};
use hub_view_state::{
    automation_view_state_after_open, automation_view_state_after_switch,
    capabilities_view_state_after_open, capabilities_view_state_after_switch,
    connections_view_state_after_open, connections_view_state_after_switch,
    learning_view_state_after_open, learning_view_state_after_switch, HubViewEffect,
};
use input_state::{
    chat_mouse_effect_for_action, chat_scroll_offset_after_wheel, non_key_input_route_for_state,
    paste_effect_for_state, ChatMouseEffect, NonKeyInputRoute, PasteEffect,
};
use list_state::{select_first_if_non_empty, select_first_if_unselected};
use navigation_state::{
    ctrl_c_action_for_key, file_picker_key_action_for_key, hub_shortcut_route_for_key,
    main_global_key_action_for_key, main_phase_entry_plan, overlay_state_after_close,
    overlay_state_after_open, resume_prompt_action_for_key, screen_key_route_for_state,
    startup_phase_for_state, AutomationView, BootScreen, CapabilitiesView, ConnectionsView,
    CtrlCAction, FilePickerKeyAction, HubShortcutRoute, LearningView, MainGlobalKeyAction,
    MainPhaseEntryRoute, OverlayKeyAction, OverlayState, Phase, ResumePromptAction, ScreenKeyRoute,
    Tab, TabCycle, AUTOMATION_VIEWS, CAPABILITIES_VIEWS, CONNECTIONS_VIEWS, LEARNING_VIEWS, TABS,
};
use refresh_state::{
    automation_refresh_route_for_view, capabilities_refresh_route_for_view,
    connections_refresh_route_for_view, learning_refresh_route_for_view, tab_refresh_route_for_tab,
    AutomationRefreshRoute, CapabilitiesRefreshRoute, ConnectionsRefreshRoute,
    LearningRefreshRoute, TabRefreshRoute,
};
use render_state::{
    automation_hub_draw_route_for_view, capabilities_hub_draw_route_for_view,
    connections_hub_draw_route_for_view, frame_draw_route_for_state, hub_draw_composition_for_area,
    learning_hub_draw_route_for_view, main_draw_composition_for_state, main_draw_route_for_tab,
    overlay_draw_route_for_tab, AutomationHubDrawRoute, CapabilitiesHubDrawRoute,
    ConnectionsHubDrawRoute, FrameDrawRoute, LearningHubDrawRoute, MainDrawLayerRoute,
    MainDrawRoute, OverlayDrawRoute,
};
use screens::{
    agents, approvals, audit, budget, channels, chat, comms, cron, dashboard, extensions, graph,
    hands, learning, logs, memory, native_capabilities, peers, projects, security, sessions,
    settings, skills, skills_proposed, templates, triggers, usage, welcome, wizard, workflows,
};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use surface_slash_state::{surface_slash_route_for_command, SurfaceSlashRoute};
use tab_state::{next_primary_tab, previous_primary_tab, tab_switch_state_after_switch};

use ratatui::layout::Rect;
use tick_state::{
    auto_poll_route_for_state, next_tick_count, screen_tick_routes, should_clear_ctrl_c_pending,
    should_poll_pending_approval, AutoPollRoute, ScreenTickRoute,
};
use usage_slash_state::{
    cost_usage_message, token_usage_message, UsageSlashSnapshot, UsageSlashSurface,
};

// ─── Core runtime handles ───────────────────────────────────────────────────

enum Backend {
    Daemon { base_url: String },
    InProcess { kernel: Arc<CaptainKernel> },
    None,
}

impl Backend {
    fn to_ref(&self) -> Option<BackendRef> {
        match self {
            Backend::Daemon { base_url } => Some(BackendRef::Daemon(base_url.clone())),
            Backend::InProcess { kernel } => Some(BackendRef::InProcess(kernel.clone())),
            Backend::None => None,
        }
    }
}

struct ChatTarget {
    agent_id_daemon: Option<String>,
    agent_id_inprocess: Option<AgentId>,
    agent_name: String,
    session_id: Option<String>,
}

fn resume_owner_matches(
    owner_id: Option<&str>,
    owner_name: &str,
    current_id: &str,
    current_name: &str,
) -> bool {
    owner_id.is_some_and(|owner| owner == current_id)
        || (!owner_name.trim().is_empty() && owner_name.eq_ignore_ascii_case(current_name))
}

struct App {
    phase: Phase,
    active_tab: Tab,
    automation_view: AutomationView,
    learning_view: LearningView,
    capabilities_view: CapabilitiesView,
    connections_view: ConnectionsView,
    /// Latest persisted session offered by the boot ResumePrompt. Cleared
    /// once the user makes a choice (Y → load, N → drop).
    pending_resume: Option<session_store::SessionSummary>,
    /// Authoritative session and owner selected by the boot resume prompt.
    pending_resume_target: Option<(String, Option<String>, String)>,
    /// Kernel-format messages waiting to be POSTed (daemon) / inserted
    /// (in-process) once the agent_id is resolved. Set by accept_resume,
    /// consumed by enter_chat_{daemon,inprocess}.
    pending_restore_messages: Option<Vec<captain_types::message::Message>>,
    /// (#182) Visual replay info — `(agent_key, session_path)` — stashed
    /// by `accept_resume` so `enter_chat_{daemon,inprocess}` can rehydrate
    /// the chat *after* its mandatory `reset()` instead of having the
    /// rehydrated history wiped a few frames later.
    pending_chat_replay: Option<(String, std::path::PathBuf)>,
    /// Phase-f.13: modal overlay. When `Some(tab)`, the chat stays
    /// rendered underneath and the named tab's screen is drawn on top. Esc pops.
    overlay_tab: Option<Tab>,
    tab_scroll_offset: usize,
    config_path: Option<PathBuf>,
    should_quit: bool,
    event_tx: mpsc::Sender<AppEvent>,
    /// Double Ctrl+C quit: true after first Ctrl+C press.
    ctrl_c_pending: bool,
    /// Tick counter when first Ctrl+C was pressed (auto-resets after ~2s).
    ctrl_c_tick: usize,
    /// Global tick counter for Ctrl+C timeout tracking.
    tick_count: usize,
    /// One shared five-second observer feeds the chat quota status line.
    provider_quota_watch_started: bool,

    backend: Backend,
    chat_target: Option<ChatTarget>,

    // Screen states
    welcome: welcome::WelcomeState,
    wizard: wizard::WizardState,
    agents: agents::AgentSelectState,
    chat: chat::ChatState,
    projects: projects::ProjectState,
    dashboard: dashboard::DashboardState,
    channels: channels::ChannelState,
    workflows: workflows::WorkflowState,
    triggers: triggers::TriggerState,
    sessions: sessions::SessionsState,
    memory: memory::MemoryState,
    learning: learning::LearningState,
    skills_proposed: skills_proposed::SkillsProposedState,
    cron: cron::CronState,
    approvals: approvals::ApprovalsState,
    budget: budget::BudgetState,
    graph: graph::GraphState,
    native_capabilities: native_capabilities::NativeCapabilitiesState,
    skills: skills::SkillsState,
    hands: hands::HandsState,
    extensions: extensions::ExtensionsState,
    templates: templates::TemplatesState,
    security: security::SecurityState,
    audit: audit::AuditState,
    usage: usage::UsageState,
    settings: settings::SettingsState,
    peers: peers::PeersState,
    comms: comms::CommsState,
    logs: logs::LogsState,

    kernel_booting: bool,
    kernel_boot_error: Option<String>,

    /// TUI polish — decode-once-render-many cache for inline image previews
    /// of staged attachments. Initialised lazily on the first `/image` upload.
    image_cache: image_preview::ImagePreviewCache,
    /// TUI polish — interactive file picker overlay opened by `/image`
    /// or `/file` typed without a path. `None` when no picker is open.
    file_picker: Option<screens::file_picker::FilePickerState>,
    /// Whether terminal mouse capture is enabled. Disabled by default so
    /// native terminal selection + right-click copy keep working.
    mouse_capture_enabled: bool,
    /// IJ.4 — live user-input tx for the currently running streaming agent
    /// loop (in-process backend only). `Some` between `StreamStarted` and
    /// `StreamDone`. While `Some`, messages typed during the stream are
    /// forwarded to the running agent loop instead of being staged for
    /// after-stream send. Daemon mode keeps the legacy stage-and-flush
    /// path because the tx lives server-side.
    current_stream_input_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// `.captain.toml` discovered by walking up from the launch cwd.
    /// `Some` when a project has opted into auto-bind; consumed by
    /// `auto_select_default_agent` to override the default
    /// "first agent named captain" pick.
    workspace: Option<crate::workspace_config::DiscoveredWorkspace>,
}

// ─── App construction ────────────────────────────────────────────────────────

impl App {
    fn new(config_path: Option<PathBuf>, event_tx: mpsc::Sender<AppEvent>) -> Self {
        let workspace = Self::discover_workspace_config();
        if let Some(ref ws) = workspace {
            tracing::info!(path = %ws.config_path.display(), "loaded workspace config");
        }
        Self {
            phase: Phase::Boot(BootScreen::Welcome),
            active_tab: Tab::Dashboard,
            automation_view: AutomationView::Workflows,
            learning_view: LearningView::Review,
            capabilities_view: CapabilitiesView::Native,
            connections_view: ConnectionsView::Channels,
            pending_resume: None,
            pending_resume_target: None,
            pending_restore_messages: None,
            pending_chat_replay: None,
            overlay_tab: None,
            tab_scroll_offset: 0,
            config_path,
            should_quit: false,
            event_tx,
            backend: Backend::None,
            chat_target: None,
            welcome: welcome::WelcomeState::new(),
            wizard: wizard::WizardState::new(),
            agents: agents::AgentSelectState::new(),
            chat: chat::ChatState::new(),
            projects: projects::ProjectState::new(),
            dashboard: dashboard::DashboardState::new(),
            channels: channels::ChannelState::new(),
            workflows: workflows::WorkflowState::new(),
            triggers: triggers::TriggerState::new(),
            sessions: sessions::SessionsState::new(),
            memory: memory::MemoryState::new(),
            learning: learning::LearningState::new(),
            skills_proposed: skills_proposed::SkillsProposedState::new(),
            cron: cron::CronState::new(),
            approvals: approvals::ApprovalsState::new(),
            budget: budget::BudgetState::new(),
            graph: graph::GraphState::new(),
            native_capabilities: native_capabilities::NativeCapabilitiesState::new(),
            skills: skills::SkillsState::new(),
            hands: hands::HandsState::new(),
            extensions: extensions::ExtensionsState::new(),
            templates: templates::TemplatesState::new(),
            security: security::SecurityState::new(),
            audit: audit::AuditState::new(),
            usage: usage::UsageState::new(),
            settings: settings::SettingsState::new(),
            peers: peers::PeersState::new(),
            comms: comms::CommsState::new(),
            logs: logs::LogsState::new(),
            kernel_booting: false,
            kernel_boot_error: None,
            image_cache: image_preview::ImagePreviewCache::new(),
            file_picker: None,
            mouse_capture_enabled: false,
            current_stream_input_tx: None,
            ctrl_c_pending: false,
            ctrl_c_tick: 0,
            tick_count: 0,
            provider_quota_watch_started: false,
            workspace,
        }
    }

    fn discover_workspace_config() -> Option<crate::workspace_config::DiscoveredWorkspace> {
        // Workspace lookup: walks up from cwd to $HOME looking for
        // `.captain.toml`. A parse error is logged but never fatal: a
        // malformed file just falls back to the welcome menu so the user can
        // still launch the TUI manually.
        let workspace = match std::env::current_dir() {
            Ok(cwd) => match crate::workspace_config::discover(&cwd) {
                Ok(opt) => opt,
                Err(e) => {
                    tracing::warn!(error = %e, "ignoring malformed .captain.toml");
                    None
                }
            },
            Err(_) => None,
        };
        // Validate `extra_paths` against the credential blocklist before we
        // hand the config to the rest of the TUI. A malicious or misguided
        // entry that would grant access to ~/.ssh / secrets.env / vault.enc
        // is removed up front, with a `warn!` so the operator notices.
        workspace.map(|mut ws| {
            if !ws.config.captain.extra_paths.is_empty() {
                let captain_home = crate::cli_captain_home();
                match crate::workspace_config::validate_extra_paths(
                    &ws.config.captain.extra_paths,
                    &captain_home,
                ) {
                    Ok(canon) => {
                        ws.config.captain.extra_paths = canon;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            path = %ws.config_path.display(),
                            "dropping unsafe extra_paths from .captain.toml"
                        );
                        ws.config.captain.extra_paths.clear();
                    }
                }
            }
            ws
        })
    }

    // ─── Event dispatch ──────────────────────────────────────────────────────

    fn handle_event(&mut self, ev: AppEvent) {
        // IJ.4 — local user input that may have just landed in
        // `staged_messages` is forwarded to the live agent loop after each
        // event tick. No-op when no stream is running or no message is
        // staged. Cheap (single Option check + Vec::is_empty).
        let drain_after = matches!(ev, AppEvent::Key(_) | AppEvent::Paste(_));
        self.route_event(ev);
        if drain_after {
            self.drain_staged_to_live_tx();
        }
    }

    fn route_event(&mut self, ev: AppEvent) {
        let Some(ev) = self.handle_input_stream_event(ev) else {
            return;
        };
        let Some(ev) = self.handle_runtime_event(ev) else {
            return;
        };
        let Some(ev) = self.handle_work_event(ev) else {
            return;
        };
        let Some(ev) = self.handle_review_event(ev) else {
            return;
        };
        let Some(ev) = self.handle_capability_settings_event(ev) else {
            return;
        };
        self.handle_operations_event(ev);
    }

    fn handle_input_stream_event(&mut self, ev: AppEvent) -> Option<AppEvent> {
        match ev {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Paste(data) => self.handle_paste(data),
            AppEvent::Scroll { up } => self.handle_scroll(up),
            AppEvent::MouseClick { x, y } => self.handle_mouse_click(x, y),
            AppEvent::Tick => self.handle_tick(),
            AppEvent::Stream(stream_ev) => self.handle_stream(stream_ev),
            AppEvent::StreamDone(result) => self.handle_stream_done(result),
            AppEvent::StreamStarted { interject_tx } => {
                self.current_stream_input_tx = Some(interject_tx);
                self.drain_staged_to_live_tx();
                return None;
            }
            other => return Some(other),
        }
        None
    }

    fn handle_runtime_event(&mut self, ev: AppEvent) -> Option<AppEvent> {
        match ev {
            AppEvent::KernelReady(kernel) => self.handle_kernel_ready(kernel),
            AppEvent::KernelError(err) => self.handle_kernel_error(err),
            AppEvent::AgentSpawned {
                id,
                name,
                api_sheet,
            } => self.handle_agent_spawned(id, name, api_sheet),
            AppEvent::AgentSpawnError(err) => self.handle_agent_spawn_error(err),
            AppEvent::MemoryStored {
                subject,
                predicate,
                object,
                source,
            } => self.handle_memory_stored_event(subject, predicate, object, source),
            AppEvent::MemoryQueued {
                review_id,
                subject,
                predicate,
                object,
                source,
            } => self.handle_memory_queued_event(review_id, subject, predicate, object, source),
            AppEvent::SkillProposalQueued {
                proposal_id,
                name,
                description,
                trigger_hint,
                confidence,
                family,
            } => self.handle_skill_proposal_queued_event(
                proposal_id,
                name,
                description,
                trigger_hint,
                confidence,
                family,
            ),
            AppEvent::AgentLifecycle {
                kind,
                agent_id,
                name,
                detail,
            } => self.handle_agent_lifecycle_event(kind, agent_id, name, detail),
            AppEvent::ToolRunStatus {
                run_id,
                tool_name,
                status,
                caller_agent_id,
            } => self.handle_tool_run_status_event(run_id, tool_name, status, caller_agent_id),
            AppEvent::DaemonDetected { url, agent_count } => {
                self.handle_daemon_detected(url, agent_count);
            }
            AppEvent::DashboardData {
                agent_count,
                uptime_secs,
                version,
                provider,
                model,
                status,
            } => self.handle_dashboard_data(
                agent_count,
                uptime_secs,
                version,
                provider,
                model,
                status,
            ),
            AppEvent::AuditLoaded(rows) => self.handle_dashboard_audit_loaded(rows),
            other => return Some(other),
        }
        None
    }

    fn handle_work_event(&mut self, ev: AppEvent) -> Option<AppEvent> {
        match ev {
            AppEvent::ChannelListLoaded(list) => self.handle_channel_list_loaded(list),
            AppEvent::ChannelTestResult { success, message } => {
                self.handle_channel_test_result(success, message);
            }
            AppEvent::WorkflowListLoaded(list) => self.handle_workflow_list_loaded(list),
            AppEvent::WorkflowRunsLoaded(runs) => self.handle_workflow_runs_loaded(runs),
            AppEvent::WorkflowRunResult(result) => self.handle_workflow_run_result(result),
            AppEvent::WorkflowCreated(_id) => self.handle_workflow_created(),
            AppEvent::ProjectListLoaded(list) => self.handle_project_list_loaded(list),
            AppEvent::ProjectDetailLoaded(detail) => self.handle_project_detail_loaded(detail),
            AppEvent::ProjectMutated(message) => self.handle_project_mutated(message),
            AppEvent::TriggerListLoaded(list) => self.handle_trigger_list_loaded(list),
            AppEvent::TriggerCreated(_id) => self.handle_trigger_created(),
            AppEvent::TriggerDeleted(id) => self.handle_trigger_deleted(id),
            AppEvent::TriggerToggled { id, enabled } => self.handle_trigger_toggled(id, enabled),
            AppEvent::AgentKilled { id } => self.handle_agent_killed(id),
            AppEvent::AgentKillError(err) => self.handle_agent_kill_error(err),
            AppEvent::AgentSkillsLoaded {
                assigned,
                available,
            } => self.handle_agent_skills_loaded(assigned, available),
            AppEvent::AgentMcpServersLoaded {
                assigned,
                available,
            } => self.handle_agent_mcp_servers_loaded(assigned, available),
            AppEvent::AgentSkillsUpdated(id) => self.handle_agent_skills_updated(id),
            AppEvent::AgentMcpServersUpdated(id) => self.handle_agent_mcp_servers_updated(id),
            AppEvent::FetchError(err) => self.apply_fetch_error(err),
            other => return Some(other),
        }
        None
    }

    fn handle_review_event(&mut self, ev: AppEvent) -> Option<AppEvent> {
        match ev {
            AppEvent::SessionsLoaded(list) => self.handle_sessions_loaded(list),
            AppEvent::SessionLoaded(session) => self.handle_loaded_session(session),
            AppEvent::SessionDeleted(id) => self.handle_session_deleted(id),
            AppEvent::LearningLoaded {
                pending,
                committed,
                metrics,
            } => self.handle_learning_loaded(pending, committed, metrics),
            AppEvent::LearningDecided { id, approved } => {
                self.handle_learning_decided(id, approved);
            }
            AppEvent::SkillsProposedLoaded { workflows, metrics } => {
                self.handle_skills_proposed_loaded(workflows, metrics)
            }
            AppEvent::WorkflowProposalDecided {
                proposal_id,
                action,
            } => {
                self.handle_workflow_proposal_decided(proposal_id, action);
            }
            AppEvent::CronJobsLoaded(jobs) => self.handle_cron_jobs_loaded(jobs),
            AppEvent::CronJobMutated { id, what } => self.handle_cron_job_mutated(id, what),
            AppEvent::ApprovalsLoaded(items) => self.handle_approvals_loaded(items),
            AppEvent::ApprovalDecided { id, approved } => {
                self.handle_approval_decided(id, approved);
            }
            AppEvent::ChatApprovalDetected(maybe_req) => {
                self.handle_chat_approval_detected(maybe_req);
            }
            AppEvent::VoiceRecorded(result) => self.handle_voice_recorded(result),
            AppEvent::ProviderQuotasLoaded(result) => {
                self.handle_provider_quotas_loaded(result);
            }
            AppEvent::BudgetLoaded {
                global,
                provider_state,
                provider_quotas,
                agents,
            } => self.handle_budget_loaded(global, provider_state, provider_quotas, agents),
            AppEvent::GraphLoaded {
                stats,
                entities,
                facts,
            } => self.handle_graph_loaded(stats, entities, facts),
            AppEvent::MemoryAgentsLoaded(agents) => self.handle_memory_agents_loaded(agents),
            AppEvent::MemoryKvLoaded(pairs) => self.handle_memory_kv_loaded(pairs),
            AppEvent::MemoryKvSaved { key } => self.handle_memory_kv_saved(key),
            AppEvent::MemoryKvDeleted(key) => self.handle_memory_kv_deleted(key),
            other => return Some(other),
        }
        None
    }

    fn handle_capability_settings_event(&mut self, ev: AppEvent) -> Option<AppEvent> {
        match ev {
            AppEvent::NativeCapabilitiesLoaded { capabilities, runs } => {
                self.native_capabilities.replace(capabilities, runs);
            }
            AppEvent::NativeCapabilityInspected(capability) => {
                self.native_capabilities.replace_inspected(capability);
            }
            AppEvent::NativeCapabilityChanged(message) => {
                self.native_capabilities.status_msg = message;
                self.refresh_native_capabilities();
            }
            AppEvent::SkillsLoaded(list) => self.handle_skills_loaded(list),
            AppEvent::ClawHubLoaded(results) => self.handle_clawhub_loaded(results),
            AppEvent::SkillInstalled(name) => self.handle_skill_installed(name),
            AppEvent::SkillUninstalled(name) => self.handle_skill_uninstalled(name),
            AppEvent::McpServersLoaded(servers) => self.handle_mcp_servers_loaded(servers),
            AppEvent::TemplateProvidersLoaded(providers) => {
                self.handle_template_providers_loaded(providers);
            }
            AppEvent::SecurityLoaded(features) => self.handle_security_loaded(features),
            AppEvent::SecurityChainVerified { valid, message } => {
                self.handle_security_chain_verified(valid, message);
            }
            AppEvent::AuditEntriesLoaded(entries) => self.handle_audit_entries_loaded(entries),
            AppEvent::AuditChainVerified(valid) => self.handle_audit_chain_verified(valid),
            AppEvent::UsageSummaryLoaded(summary) => self.handle_usage_summary_loaded(summary),
            AppEvent::UsageByModelLoaded(models) => self.handle_usage_by_model_loaded(models),
            AppEvent::UsageByAgentLoaded(agents) => self.handle_usage_by_agent_loaded(agents),
            AppEvent::SettingsProvidersLoaded(providers) => {
                self.handle_settings_providers_loaded(providers);
            }
            AppEvent::SettingsModelsLoaded(models) => self.handle_settings_models_loaded(models),
            AppEvent::SettingsToolsLoaded(tools) => self.handle_settings_tools_loaded(tools),
            AppEvent::ProviderKeySaved(name) => self.handle_provider_key_saved(name),
            AppEvent::ProviderKeyDeleted(name) => self.handle_provider_key_deleted(name),
            AppEvent::ProviderTestResult(result) => self.handle_provider_test_result(result),
            other => return Some(other),
        }
        None
    }

    fn handle_operations_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::PeersLoaded(list) => self.handle_peers_loaded(list),
            AppEvent::CommsTopologyLoaded { nodes, edges } => {
                self.handle_comms_topology_loaded(nodes, edges);
            }
            AppEvent::CommsEventsLoaded(events) => self.handle_comms_events_loaded(events),
            AppEvent::CommsSendResult(msg) => self.handle_comms_send_result(msg),
            AppEvent::CommsTaskResult(msg) => self.handle_comms_task_result(msg),
            AppEvent::LogsLoaded(entries) => self.handle_logs_loaded(entries),
            AppEvent::HandsLoaded(list) => self.handle_hands_loaded(list),
            AppEvent::ActiveHandsLoaded(list) => self.handle_active_hands_loaded(list),
            AppEvent::HandActivated(name) => self.handle_hand_activated(name),
            AppEvent::HandDeactivated(id) => self.handle_hand_deactivated(id),
            AppEvent::HandPaused(id) => self.handle_hand_paused(id),
            AppEvent::HandResumed(id) => self.handle_hand_resumed(id),
            AppEvent::ExtensionsLoaded(list) => self.handle_extensions_loaded(list),
            AppEvent::ExtensionHealthLoaded(entries) => {
                self.handle_extension_health_loaded(entries)
            }
            AppEvent::ExtensionInstalled(id) => self.handle_extension_installed(id),
            AppEvent::ExtensionRemoved(id) => self.handle_extension_removed(id),
            AppEvent::ExtensionReconnected(id, tools) => {
                self.handle_extension_reconnected(id, tools);
            }
            other => self.handle_unrouted_event(other),
        }
    }

    fn handle_unrouted_event(&mut self, _event: AppEvent) {
        tracing::warn!("unhandled TUI event route");
    }

    fn handle_daemon_detected(&mut self, url: Option<String>, agent_count: u64) {
        let has_daemon = url.is_some();
        self.welcome.on_daemon_detected(url, agent_count);
        // Chat-first auto-routing (escape hatch CAPTAIN_NO_AUTO_DAEMON=1):
        //   phase-f.6 daemon present -> ConnectDaemon
        //   phase-f.9 no daemon      -> spawn in-process kernel directly
        // The Welcome menu is only shown when the user explicitly opts out.
        if let Some(action) = welcome_route::auto_route_action(
            matches!(self.phase, Phase::Boot(BootScreen::Welcome)),
            std::env::var("CAPTAIN_NO_AUTO_DAEMON").is_ok(),
            has_daemon,
        ) {
            self.handle_welcome_action(action);
        }
    }

    fn handle_voice_recorded(&mut self, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                let path_str = path.display().to_string();
                self.chat.push_message(
                    chat::Role::System,
                    slash_local::voice_uploading_message(&path_str),
                );
                // Reuse the /file upload + auto-transcribe pipeline. The
                // daemon transcribes audio uploads automatically and inlines
                // the transcript when the next message is sent.
                self.handle_image_attach(&path_str);
            }
            Err(msg) => {
                self.chat
                    .push_message(chat::Role::System, slash_local::voice_error_message(msg));
            }
        }
    }

    fn handle_budget_loaded(
        &mut self,
        global: Option<budget::BudgetGlobal>,
        provider_state: String,
        provider_quotas: Vec<provider_quota::ProviderQuota>,
        agents: Vec<budget::AgentSpend>,
    ) {
        self.chat.provider_quota_status = provider_quota::ProviderQuotaStatus {
            reported_by_provider: !provider_quotas.is_empty(),
            state: provider_state.clone(),
            quotas: provider_quotas.clone(),
        };
        self.budget.global = global;
        self.budget.provider_state = provider_state;
        self.budget.provider_quotas = provider_quotas;
        self.budget.agents = agents;
        self.budget.loading = false;
        select_first_if_unselected(&mut self.budget.list_state, &self.budget.agents);
    }

    fn handle_provider_quotas_loaded(
        &mut self,
        result: Result<provider_quota::ProviderQuotaStatus, String>,
    ) {
        match result {
            Ok(status) => {
                self.budget.provider_state = status.state.clone();
                self.budget.provider_quotas = status.quotas.clone();
                self.chat.provider_quota_status = status;
            }
            Err(error) => {
                if !self.chat.provider_quota_status.has_observation() {
                    self.chat.provider_quota_status =
                        provider_quota::ProviderQuotaStatus::default();
                }
                tracing::debug!(error = %error, "TUI provider quota refresh unavailable");
            }
        }
    }

    fn handle_graph_loaded(
        &mut self,
        stats: Option<graph::GraphStats>,
        entities: Vec<graph::GraphEntity>,
        facts: Vec<graph::GraphFact>,
    ) {
        self.graph.stats = stats;
        self.graph.entities = entities;
        self.graph.facts = facts;
        self.graph.loading = false;
        select_first_if_unselected(&mut self.graph.list_state, &self.graph.entities);
    }

    fn handle_memory_stored_event(
        &mut self,
        subject: String,
        predicate: String,
        object: String,
        source: String,
    ) {
        self.chat.push_message(
            chat::Role::System,
            event_messages::memory_stored_line(&subject, &predicate, &object, &source),
        );
    }

    fn handle_memory_queued_event(
        &mut self,
        review_id: String,
        subject: String,
        predicate: String,
        object: String,
        source: String,
    ) {
        self.chat.push_message(
            chat::Role::System,
            event_messages::memory_queued_line(&review_id, &subject, &predicate, &object, &source),
        );
    }

    fn handle_agent_lifecycle_event(
        &mut self,
        kind: String,
        agent_id: String,
        name: Option<String>,
        detail: Option<String>,
    ) {
        let display_name = name.clone().unwrap_or_else(|| agent_id.clone());
        match kind.as_str() {
            "spawned" => self
                .chat
                .track_background_activity(agent_id, format!("agent {display_name}")),
            "terminated" | "crashed" => {
                self.chat.clear_background_activity(&agent_id);
                self.chat.push_message(
                    chat::Role::System,
                    event_messages::agent_lifecycle_line(&kind, &display_name, detail.as_deref()),
                );
            }
            _ => {}
        }
    }

    fn handle_tool_run_status_event(
        &mut self,
        run_id: String,
        tool_name: String,
        status: String,
        _caller_agent_id: Option<String>,
    ) {
        match status.as_str() {
            "running" => self
                .chat
                .track_background_activity(run_id, format!("tool_run {tool_name}")),
            "completed" | "failed" | "cancelled" => {
                self.chat.clear_background_activity(&run_id);
            }
            _ => {}
        }
    }

    fn handle_skill_proposal_queued_event(
        &mut self,
        proposal_id: String,
        name: String,
        description: String,
        trigger_hint: String,
        confidence: f32,
        family: Option<String>,
    ) {
        self.chat.push_message(
            chat::Role::System,
            event_messages::skill_proposal_line(
                &proposal_id,
                &name,
                &description,
                &trigger_hint,
                confidence,
                family.as_deref(),
            ),
        );
        self.refresh_skills_proposed();
    }

    fn handle_dashboard_data(
        &mut self,
        agent_count: u64,
        uptime_secs: u64,
        version: String,
        provider: String,
        model: String,
        status: dashboard::StatusSnapshot,
    ) {
        self.dashboard.status = status;
        self.dashboard.agent_count = agent_count;
        self.dashboard.uptime_secs = uptime_secs;
        self.dashboard.version = version;
        self.dashboard.provider = provider;
        self.dashboard.model = model;
        self.dashboard.loading = false;
    }

    fn handle_dashboard_audit_loaded(&mut self, rows: Vec<dashboard::AuditRow>) {
        self.dashboard.recent_audit = rows;
        self.dashboard.loading = false;
    }

    fn handle_channel_list_loaded(&mut self, list: Vec<channels::ChannelInfo>) {
        if !list.is_empty() {
            self.channels.channels = list;
            select_first_if_non_empty(&mut self.channels.list_state, &self.channels.channels);
        }
        self.channels.loading = false;
    }

    fn handle_channel_test_result(&mut self, success: bool, message: String) {
        self.channels.test_result = Some((success, message));
    }

    fn handle_workflow_list_loaded(&mut self, list: Vec<workflows::WorkflowInfo>) {
        self.workflows.workflows = list;
        select_first_if_non_empty(&mut self.workflows.list_state, &self.workflows.workflows);
        self.workflows.loading = false;
    }

    fn handle_workflow_runs_loaded(&mut self, runs: Vec<workflows::WorkflowRun>) {
        self.workflows.runs = runs;
        select_first_if_non_empty(&mut self.workflows.runs_list_state, &self.workflows.runs);
        self.workflows.loading = false;
    }

    fn handle_workflow_run_result(&mut self, result: String) {
        self.workflows.run_result = Some(result);
        self.workflows.loading = false;
    }

    fn handle_workflow_created(&mut self) {
        self.workflows.status_msg = automation_status::workflow_created_message().to_string();
        self.refresh_workflows();
    }

    fn handle_trigger_list_loaded(&mut self, list: Vec<triggers::TriggerInfo>) {
        self.triggers.triggers = list;
        select_first_if_non_empty(&mut self.triggers.list_state, &self.triggers.triggers);
        self.triggers.loading = false;
    }

    fn handle_trigger_created(&mut self) {
        self.triggers.status_msg = automation_status::trigger_created_message().to_string();
        self.refresh_triggers();
    }

    fn handle_trigger_deleted(&mut self, id: String) {
        self.triggers.triggers.retain(|t| t.id != id);
        self.triggers.status_msg = automation_status::trigger_deleted_message(&id);
    }

    fn handle_trigger_toggled(&mut self, id: String, enabled: bool) {
        if let Some(t) = self.triggers.triggers.iter_mut().find(|t| t.id == id) {
            t.enabled = enabled;
        }
        self.triggers.status_msg = automation_status::trigger_toggled_message(&id, enabled);
    }

    fn handle_cron_jobs_loaded(&mut self, jobs: Vec<cron::CronJob>) {
        self.cron.jobs = jobs;
        self.cron.loading = false;
        select_first_if_unselected(&mut self.cron.list_state, &self.cron.jobs);
    }

    fn handle_cron_job_mutated(&mut self, id: String, what: &'static str) {
        self.cron.status_msg = automation_status::cron_job_mutated_message(&id, what);
        self.refresh_cron();
    }

    fn handle_sessions_loaded(&mut self, list: Vec<sessions::SessionInfo>) {
        if self.chat.show_session_picker {
            self.chat.set_authoritative_session_picker_items(&list);
            if !self.chat.show_session_picker {
                self.chat.push_message(
                    chat::Role::System,
                    slash_session::no_saved_history_message(crate::i18n::current()).to_string(),
                );
            }
        }
        self.sessions.sessions = list;
        self.sessions.refilter();
        self.sessions.loading = false;
    }

    fn handle_loaded_session(&mut self, loaded: session_runtime::LoadedSession) {
        match &self.backend {
            Backend::Daemon { .. } => {
                self.enter_chat_daemon(loaded.agent_id.clone(), loaded.agent_name.clone());
            }
            Backend::InProcess { .. } => {
                let Ok(agent_id) = loaded.agent_id.parse::<AgentId>() else {
                    self.chat.status_msg = Some("Session owner is invalid".to_string());
                    return;
                };
                self.enter_chat_inprocess(agent_id, loaded.agent_name.clone());
            }
            Backend::None => {
                self.chat.status_msg = Some("No backend connected".to_string());
                return;
            }
        }

        let short = loaded
            .session_id
            .get(..8)
            .unwrap_or(loaded.session_id.as_str());
        self.chat
            .start_session(&format!("session-{}-{short}", loaded.agent_id));
        self.chat
            .bind_authoritative_session(&loaded.agent_id, &loaded.session_id);
        if let Some(target) = self.chat_target.as_mut() {
            target.session_id = Some(loaded.session_id.clone());
        }
        let restored =
            session_runtime::restore_public_session_messages(&mut self.chat, &loaded.detail);
        self.chat.push_message(
            chat::Role::System,
            format!(
                "Session restaurée : {} ({restored} messages).",
                loaded.label
            ),
        );
        self.active_tab = Tab::Chat;
    }

    fn handle_session_deleted(&mut self, id: String) {
        self.sessions.sessions.retain(|s| s.id != id);
        self.sessions.refilter();
        self.sessions.status_msg = resource_status::session_deleted_message(&id);
    }

    fn handle_project_list_loaded(&mut self, list: Vec<projects::ProjectInfo>) {
        self.projects.projects = list;
        select_first_if_unselected(&mut self.projects.list_state, &self.projects.projects);
        self.projects.loading = false;
    }

    fn handle_project_detail_loaded(&mut self, detail: projects::ProjectDetail) {
        self.projects.detail = Some(detail);
        if self.projects.goal_state.selected().is_none() {
            self.projects.goal_state.select(Some(0));
        }
        self.projects.loading = false;
        self.projects.sub = projects::ProjectSubScreen::Detail;
    }

    fn handle_project_mutated(&mut self, message: String) {
        self.projects.status_msg = message;
        if matches!(self.projects.sub, projects::ProjectSubScreen::Detail)
            && self.projects.active_project_id().is_some()
        {
            self.refresh_active_project_detail(true);
        } else {
            self.refresh_projects();
        }
    }

    fn handle_learning_loaded(
        &mut self,
        pending: Vec<learning::ReviewItem>,
        committed: Vec<learning::CommittedRow>,
        metrics: Option<learning::LearningMetrics>,
    ) {
        self.learning.pending = pending;
        self.learning.committed = committed;
        self.learning.metrics = metrics;
        self.learning.loading = false;
        select_first_if_unselected(&mut self.learning.list_state, &self.learning.pending);
    }

    fn handle_learning_decided(&mut self, id: String, approved: bool) {
        self.learning.pending.retain(|p| p.id != id);
        self.learning.status_msg = decision_status::decision_message(&id, approved);
        self.refresh_learning();
    }

    fn handle_skills_proposed_loaded(
        &mut self,
        workflows: Vec<captain_types::workflow_learning::WorkflowLearningView>,
        metrics: Option<skills_proposed::SkillsMetrics>,
    ) {
        self.skills_proposed.workflows = workflows;
        self.skills_proposed.metrics = metrics;
        self.skills_proposed.loading = false;
        if self.skills_proposed.list_state.selected().is_none()
            && !self.skills_proposed.workflows.is_empty()
        {
            self.skills_proposed.list_state.select(Some(0));
        }
    }

    fn handle_workflow_proposal_decided(
        &mut self,
        proposal_id: String,
        action: captain_types::workflow_learning::ProposalCardAction,
    ) {
        self.skills_proposed.status_msg =
            format!("action {} acceptée pour {}", action.as_str(), proposal_id);
        self.refresh_skills_proposed();
    }

    fn handle_approvals_loaded(&mut self, items: Vec<approvals::ApprovalRequest>) {
        self.approvals.pending = items;
        self.approvals.loading = false;
        select_first_if_unselected(&mut self.approvals.list_state, &self.approvals.pending);
    }

    fn handle_approval_decided(&mut self, id: String, approved: bool) {
        self.approvals.pending.retain(|a| a.id != id);
        if let Some(ref pending) = self.chat.pending_approval {
            if pending.id == id {
                self.chat.pending_approval = None;
            }
        }
        self.approvals.status_msg = decision_status::decision_message(&id, approved);
        self.refresh_approvals();
    }

    fn handle_chat_approval_detected(&mut self, maybe_req: Option<approvals::ApprovalRequest>) {
        if self.chat.pending_approval.is_none() {
            self.chat.pending_approval = maybe_req;
        }
    }

    fn handle_memory_agents_loaded(&mut self, agents: Vec<memory::AgentEntry>) {
        self.memory.agents = agents;
        select_first_if_non_empty(&mut self.memory.agent_list_state, &self.memory.agents);
        self.memory.loading = false;
    }

    fn handle_memory_kv_loaded(&mut self, pairs: Vec<memory::KvPair>) {
        self.memory.kv_pairs = pairs;
        select_first_if_non_empty(&mut self.memory.kv_list_state, &self.memory.kv_pairs);
        self.memory.loading = false;
    }

    fn handle_memory_kv_saved(&mut self, key: String) {
        self.memory.status_msg = resource_status::memory_key_saved_message(&key);
        if let Some(agent) = &self.memory.selected_agent {
            if let Some(backend) = self.backend.to_ref() {
                event::spawn_fetch_memory_kv(backend, agent.id.clone(), self.event_tx.clone());
            }
        }
    }

    fn handle_memory_kv_deleted(&mut self, key: String) {
        self.memory.kv_pairs.retain(|kv| kv.key != key);
        self.memory.status_msg = resource_status::memory_key_deleted_message(&key);
    }

    fn handle_skills_loaded(&mut self, list: Vec<skills::SkillInfo>) {
        self.skills.installed = list;
        select_first_if_non_empty(&mut self.skills.installed_list, &self.skills.installed);
        self.skills.loading = false;
    }

    fn handle_clawhub_loaded(&mut self, results: Vec<skills::ClawHubResult>) {
        self.skills.clawhub_results = results;
        select_first_if_non_empty(&mut self.skills.clawhub_list, &self.skills.clawhub_results);
        self.skills.loading = false;
    }

    fn handle_skill_installed(&mut self, name: String) {
        self.skills.status_msg = resource_status::skill_installed_message(&name);
        self.refresh_skills();
    }

    fn handle_skill_uninstalled(&mut self, name: String) {
        self.skills.installed.retain(|s| s.name != name);
        self.skills.status_msg = resource_status::skill_uninstalled_message(&name);
    }

    fn handle_mcp_servers_loaded(&mut self, servers: Vec<skills::McpServerInfo>) {
        self.skills.mcp_servers = servers;
        select_first_if_non_empty(&mut self.skills.mcp_list, &self.skills.mcp_servers);
        self.skills.loading = false;
    }

    fn handle_template_providers_loaded(&mut self, providers: Vec<templates::ProviderAuth>) {
        self.templates.providers = providers;
    }

    fn handle_security_loaded(&mut self, features: Vec<security::SecurityFeature>) {
        self.security.features = features;
        self.security.loading = false;
    }

    fn handle_security_chain_verified(&mut self, valid: bool, message: String) {
        self.security.chain_verified = Some(valid);
        self.security.verify_result = message;
        self.security.loading = false;
    }

    fn handle_audit_entries_loaded(&mut self, entries: Vec<audit::AuditEntry>) {
        self.audit.entries = entries;
        self.audit.refilter();
        self.audit.loading = false;
    }

    fn handle_audit_chain_verified(&mut self, valid: bool) {
        self.audit.chain_verified = Some(valid);
    }

    fn handle_usage_summary_loaded(&mut self, summary: usage::UsageSummary) {
        self.usage.summary = summary;
        self.usage.loading = false;
    }

    fn handle_usage_by_model_loaded(&mut self, models: Vec<usage::ModelUsage>) {
        self.usage.by_model = models;
        select_first_if_non_empty(&mut self.usage.model_list, &self.usage.by_model);
    }

    fn handle_usage_by_agent_loaded(&mut self, agents: Vec<usage::AgentUsage>) {
        self.usage.by_agent = agents;
        select_first_if_non_empty(&mut self.usage.agent_list, &self.usage.by_agent);
    }

    fn handle_settings_providers_loaded(&mut self, providers: Vec<settings::ProviderInfo>) {
        self.settings.providers = providers;
        select_first_if_non_empty(&mut self.settings.provider_list, &self.settings.providers);
        self.settings.loading = false;
    }

    fn handle_settings_models_loaded(&mut self, models: Vec<settings::ModelInfo>) {
        self.settings.models = models;
        select_first_if_non_empty(&mut self.settings.model_list, &self.settings.models);
        self.settings.loading = false;
    }

    fn handle_settings_tools_loaded(&mut self, tools: Vec<settings::ToolInfo>) {
        self.settings.tools = tools;
        select_first_if_non_empty(&mut self.settings.tool_list, &self.settings.tools);
        self.settings.loading = false;
    }

    fn handle_provider_key_saved(&mut self, name: String) {
        self.settings.status_msg = resource_status::provider_key_saved_message(&name);
        self.refresh_settings_providers();
    }

    fn handle_provider_key_deleted(&mut self, name: String) {
        self.settings.status_msg = resource_status::provider_key_deleted_message(&name);
        self.refresh_settings_providers();
    }

    fn handle_provider_test_result(&mut self, result: settings::TestResult) {
        self.settings.test_result = Some(result);
    }

    fn handle_peers_loaded(&mut self, list: Vec<peers::PeerInfo>) {
        self.peers.peers = list;
        select_first_if_unselected(&mut self.peers.list_state, &self.peers.peers);
        self.peers.loading = false;
    }

    fn handle_comms_topology_loaded(
        &mut self,
        nodes: Vec<comms::CommsNode>,
        edges: Vec<comms::CommsEdge>,
    ) {
        self.comms.nodes = nodes;
        self.comms.edges = edges;
        self.comms.loading = false;
    }

    fn handle_comms_events_loaded(&mut self, events: Vec<comms::CommsEventItem>) {
        self.comms.events = events;
        select_first_if_unselected(&mut self.comms.event_list_state, &self.comms.events);
    }

    fn handle_comms_send_result(&mut self, msg: String) {
        self.comms.status_msg = msg;
        self.refresh_comms();
    }

    fn handle_comms_task_result(&mut self, msg: String) {
        self.comms.status_msg = msg;
    }

    fn handle_logs_loaded(&mut self, entries: Vec<logs::LogEntry>) {
        self.logs.entries = entries;
        self.logs.refilter();
        self.logs.loading = false;
    }

    fn handle_hands_loaded(&mut self, list: Vec<hands::HandInfo>) {
        self.hands.definitions = list;
        select_first_if_non_empty(&mut self.hands.marketplace_list, &self.hands.definitions);
        self.hands.loading = false;
    }

    fn handle_active_hands_loaded(&mut self, list: Vec<hands::HandInstanceInfo>) {
        self.hands.instances = list;
        select_first_if_unselected(&mut self.hands.active_list, &self.hands.instances);
        self.hands.loading = false;
    }

    fn handle_hand_activated(&mut self, name: String) {
        self.hands.status_msg = format!("Activated: {name}");
        self.refresh_hands();
    }

    fn handle_hand_deactivated(&mut self, id: String) {
        self.hands.instances.retain(|i| i.instance_id != id);
        self.hands.status_msg = format!("Deactivated: {id}");
    }

    fn handle_hand_paused(&mut self, id: String) {
        self.set_hand_instance_status(&id, "Paused");
        self.hands.status_msg = "Hand paused".to_string();
    }

    fn handle_hand_resumed(&mut self, id: String) {
        self.set_hand_instance_status(&id, "Active");
        self.hands.status_msg = "Hand resumed".to_string();
    }

    fn set_hand_instance_status(&mut self, id: &str, status: &str) {
        if let Some(inst) = self
            .hands
            .instances
            .iter_mut()
            .find(|i| i.instance_id == id)
        {
            inst.status = status.to_string();
        }
    }

    fn handle_extensions_loaded(&mut self, list: Vec<extensions::ExtensionInfo>) {
        self.extensions.all_extensions = list;
        select_first_if_unselected(
            &mut self.extensions.browse_list,
            &self.extensions.all_extensions,
        );
        self.extensions.loading = false;
    }

    fn handle_extension_health_loaded(&mut self, entries: Vec<extensions::ExtensionHealthInfo>) {
        self.extensions.health_entries = entries;
        select_first_if_unselected(
            &mut self.extensions.health_list,
            &self.extensions.health_entries,
        );
    }

    fn handle_extension_installed(&mut self, id: String) {
        self.extensions.status_msg = format!("Installed: {id}");
        self.refresh_extensions();
    }

    fn handle_extension_removed(&mut self, id: String) {
        self.extensions.status_msg = format!("Removed: {id}");
        self.refresh_extensions();
    }

    fn handle_extension_reconnected(&mut self, id: String, tools: usize) {
        self.extensions.status_msg = format!("Reconnected {id}: {tools} tools");
        self.refresh_extension_health();
    }

    fn handle_agent_killed(&mut self, id: String) {
        self.agents.status_msg = agent_status::agent_killed_message(&id);
        self.agents.sub = agents::AgentSubScreen::AgentList;
        self.refresh_agents();
    }

    fn handle_agent_kill_error(&mut self, err: String) {
        self.agents.status_msg = agent_status::agent_kill_failed_message(err);
    }

    fn handle_agent_skills_loaded(&mut self, assigned: Vec<String>, available: Vec<String>) {
        self.agents.load_available_skills(assigned, available);
    }

    fn handle_agent_mcp_servers_loaded(&mut self, assigned: Vec<String>, available: Vec<String>) {
        self.agents.load_available_mcp_servers(assigned, available);
    }

    fn handle_agent_skills_updated(&mut self, id: String) {
        self.agents.status_msg = agent_status::agent_skills_updated_message(&id);
        self.agents.sub = agents::AgentSubScreen::AgentDetail;
    }

    fn handle_agent_mcp_servers_updated(&mut self, id: String) {
        self.agents.status_msg = agent_status::agent_mcp_servers_updated_message(&id);
        self.agents.sub = agents::AgentSubScreen::AgentDetail;
    }

    fn apply_fetch_error(&mut self, err: String) {
        let Some(target) = error_route::fetch_error_target(
            self.active_tab,
            self.automation_view,
            self.connections_view,
            self.learning_view,
            self.capabilities_view,
        ) else {
            return;
        };
        self.apply_fetch_error_to_target(target, err);
    }

    fn apply_fetch_error_to_target(&mut self, target: error_route::FetchErrorTarget, err: String) {
        match target {
            error_route::FetchErrorTarget::Workflows => self.workflows.status_msg = err,
            error_route::FetchErrorTarget::Triggers => self.triggers.status_msg = err,
            error_route::FetchErrorTarget::Cron => self.cron.status_msg = err,
            error_route::FetchErrorTarget::Approvals => self.approvals.status_msg = err,
            error_route::FetchErrorTarget::Projects => self.projects.status_msg = err,
            error_route::FetchErrorTarget::Channels => self.channels.status_msg = err,
            error_route::FetchErrorTarget::Extensions => self.extensions.status_msg = err,
            error_route::FetchErrorTarget::PeersLoading => self.peers.loading = false,
            error_route::FetchErrorTarget::Comms => self.comms.status_msg = err,
            error_route::FetchErrorTarget::Sessions => self.sessions.status_msg = err,
            error_route::FetchErrorTarget::Learning => self.learning.status_msg = err,
            error_route::FetchErrorTarget::SkillsProposed => self.skills_proposed.status_msg = err,
            error_route::FetchErrorTarget::Memory => self.memory.status_msg = err,
            error_route::FetchErrorTarget::Graph => self.graph.status_msg = err,
            error_route::FetchErrorTarget::NativeCapabilities => {
                self.native_capabilities.loading = false;
                self.native_capabilities.status_msg = err;
            }
            error_route::FetchErrorTarget::Skills => self.skills.status_msg = err,
            error_route::FetchErrorTarget::Hands => self.hands.status_msg = err,
            error_route::FetchErrorTarget::Templates => self.templates.status_msg = err,
            error_route::FetchErrorTarget::Settings => self.settings.status_msg = err,
        }
    }

    /// IJ.4 — push any messages staged during a live stream into the running
    /// agent loop's `user_input_rx`. No-op when no stream is live or nothing
    /// is staged. Re-stages a message and stops on backpressure (channel
    /// full) so the user message is not silently dropped.
    fn drain_staged_to_live_tx(&mut self) {
        let Some(tx) = self.current_stream_input_tx.as_ref() else {
            return;
        };
        while let Some(msg) = self.chat.take_staged() {
            if let Err(err) = tx.try_send(msg.clone()) {
                use tokio::sync::mpsc::error::TrySendError;
                self.chat.staged_messages.insert(0, msg);
                if matches!(err, TrySendError::Closed(_)) {
                    self.current_stream_input_tx = None;
                }
                return;
            }
        }
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        if self.handle_ctrl_c_key(key) {
            return;
        }
        if self.handle_file_picker_key(key) {
            return;
        }
        if self.handle_main_global_key(key) {
            return;
        }
        if self.handle_overlay_key(key) {
            return;
        }

        self.handle_screen_key_route(screen_key_route_for_state(self.phase, self.active_tab), key);
    }

    fn handle_ctrl_c_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        match ctrl_c_action_for_key(key.code, key.modifiers, self.ctrl_c_pending, self.phase) {
            CtrlCAction::Quit => {
                self.should_quit = true;
                true
            }
            CtrlCAction::ArmAndStopRouting => {
                self.ctrl_c_pending = true;
                self.ctrl_c_tick = self.tick_count;
                true
            }
            CtrlCAction::ArmAndContinueRouting => {
                self.ctrl_c_pending = true;
                self.ctrl_c_tick = self.tick_count;
                false
            }
            CtrlCAction::ClearPending => {
                self.ctrl_c_pending = false;
                false
            }
        }
    }

    fn handle_file_picker_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        if let Some(action) = file_picker_key_action_for_key(self.file_picker.is_some(), key.code) {
            match action {
                FilePickerKeyAction::Close => {
                    self.file_picker = None;
                }
                FilePickerKeyAction::RouteToPicker => {
                    let event = ratatui::crossterm::event::Event::Key(key);
                    let outcome = match self.file_picker.as_mut() {
                        Some(p) => p.handle(&event),
                        None => return true,
                    };
                    match outcome {
                        Ok(Some(path)) => {
                            self.file_picker = None;
                            let raw = path.to_string_lossy().into_owned();
                            self.handle_image_attach(&raw);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            self.file_picker = None;
                            self.chat.push_message(
                                chat::Role::System,
                                slash_attachment::picker_runtime_error_message(e),
                            );
                        }
                    }
                }
            }
            return true;
        }
        false
    }

    fn handle_main_global_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        if matches!(self.phase, Phase::Main) {
            if self.overlay_tab.is_none() && self.handle_hub_shortcut(key) {
                return true;
            }

            if let Some(action) = main_global_key_action_for_key(
                key.code,
                key.modifiers,
                matches!(self.active_tab, Tab::Chat) && self.chat.input.starts_with('/'),
            ) {
                match action {
                    MainGlobalKeyAction::Quit => self.should_quit = true,
                    MainGlobalKeyAction::SwitchTab(tab) => self.switch_tab(tab),
                    MainGlobalKeyAction::CycleTab(TabCycle::Previous) => self.prev_tab(),
                    MainGlobalKeyAction::CycleTab(TabCycle::Next) => self.next_tab(),
                }
                return true;
            }
        }
        false
    }

    fn handle_screen_key_route(
        &mut self,
        route: ScreenKeyRoute,
        key: ratatui::crossterm::event::KeyEvent,
    ) {
        match route {
            ScreenKeyRoute::Welcome => {
                if let Some(action) = self.welcome.handle_key(key) {
                    self.handle_welcome_action(action);
                }
            }
            ScreenKeyRoute::Wizard => match self.wizard.handle_key(key) {
                wizard::WizardResult::Cancelled => {
                    self.phase = Phase::Boot(BootScreen::Welcome);
                    self.start_daemon_detect();
                }
                wizard::WizardResult::Continue => {
                    if self.wizard.step == wizard::WizardStep::Done
                        && self.wizard.created_config.is_some()
                    {
                        self.config_path = self.wizard.created_config.clone();
                        self.welcome.setup_just_completed = true;
                        self.phase = Phase::Boot(BootScreen::Welcome);
                        self.start_daemon_detect();
                    }
                }
            },
            ScreenKeyRoute::ResumePrompt => match resume_prompt_action_for_key(key.code) {
                Some(ResumePromptAction::Accept) => self.accept_resume(),
                Some(ResumePromptAction::Decline) => self.decline_resume(),
                None => {}
            },
            ScreenKeyRoute::Main(tab) => self.handle_main_tab_key(tab, key),
        }
    }

    fn handle_main_tab_key(&mut self, tab: Tab, key: ratatui::crossterm::event::KeyEvent) {
        let Some((tab, key)) = self.handle_primary_main_tab_key(tab, key) else {
            return;
        };
        let Some((tab, key)) = self.handle_work_main_tab_key(tab, key) else {
            return;
        };
        self.handle_status_main_tab_key(tab, key);
    }

    fn handle_primary_main_tab_key(
        &mut self,
        tab: Tab,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> Option<(Tab, ratatui::crossterm::event::KeyEvent)> {
        match tab {
            Tab::Dashboard => {
                let action = self.dashboard.handle_key(key);
                self.handle_dashboard_action(action);
                None
            }
            Tab::Agents => {
                let action = self.agents.handle_key(key);
                self.handle_agent_action(action);
                None
            }
            Tab::Chat => {
                let action = self.chat.handle_key(key);
                self.handle_chat_action(action);
                None
            }
            Tab::Projects => {
                let action = self.projects.handle_key(key);
                self.handle_project_action(action);
                None
            }
            Tab::Channels => {
                self.handle_connections_key(key);
                None
            }
            Tab::Workflows => {
                self.handle_automation_key(key);
                None
            }
            Tab::Triggers => {
                let action = self.triggers.handle_key(key);
                self.handle_trigger_action(action);
                None
            }
            _ => Some((tab, key)),
        }
    }

    fn handle_work_main_tab_key(
        &mut self,
        tab: Tab,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> Option<(Tab, ratatui::crossterm::event::KeyEvent)> {
        match tab {
            Tab::Cron => {
                let action = self.cron.handle_key(key);
                self.handle_cron_action(action);
                None
            }
            Tab::Approvals => {
                let action = self.approvals.handle_key(key);
                self.handle_approvals_action(action);
                None
            }
            Tab::Budget => {
                let action = self.budget.handle_key(key);
                self.handle_budget_action(action);
                None
            }
            Tab::Graph => {
                let action = self.graph.handle_key(key);
                self.handle_graph_action(action);
                None
            }
            Tab::Sessions => {
                let action = self.sessions.handle_key(key);
                self.handle_sessions_action(action);
                None
            }
            Tab::Memory => {
                let action = self.memory.handle_key(key);
                self.handle_memory_action(action);
                None
            }
            Tab::Learning => {
                self.handle_learning_key(key);
                None
            }
            Tab::SkillsProposed => {
                let action = self.skills_proposed.handle_key(key);
                self.handle_skills_proposed_action(action);
                None
            }
            _ => Some((tab, key)),
        }
    }

    fn handle_status_main_tab_key(&mut self, tab: Tab, key: ratatui::crossterm::event::KeyEvent) {
        match tab {
            Tab::Skills => self.handle_capabilities_key(key),
            Tab::Extensions => {
                let action = self.extensions.handle_key(key);
                self.handle_extensions_action(action);
            }
            Tab::Hands => {
                let action = self.hands.handle_key(key);
                self.handle_hands_action(action);
            }
            Tab::Templates => {
                let action = self.templates.handle_key(key);
                self.handle_templates_action(action);
            }
            Tab::Security => {
                let action = self.security.handle_key(key);
                self.handle_security_action(action);
            }
            Tab::Audit => {
                let action = self.audit.handle_key(key);
                self.handle_audit_action(action);
            }
            Tab::Usage => {
                let action = self.usage.handle_key(key);
                self.handle_usage_action(action);
            }
            Tab::Settings => {
                let action = self.settings.handle_key(key);
                self.handle_settings_action(action);
            }
            Tab::Peers => {
                let action = self.peers.handle_key(key);
                self.handle_peers_action(action);
            }
            Tab::Comms => {
                let action = self.comms.handle_key(key);
                self.handle_comms_action(action);
            }
            Tab::Logs => {
                let action = self.logs.handle_key(key);
                self.handle_logs_action(action);
            }
            _ => {}
        }
    }

    fn handle_overlay_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        let Some(action) =
            navigation_state::overlay_key_action_for_key(self.phase, self.overlay_tab, key.code)
        else {
            return false;
        };

        match action {
            OverlayKeyAction::Close => self.close_overlay(),
            OverlayKeyAction::RouteTo(overlay) => match overlay {
                Tab::Memory => {
                    let action = self.memory.handle_key(key);
                    self.handle_memory_action(action);
                }
                Tab::Logs => {
                    let action = self.logs.handle_key(key);
                    self.handle_logs_action(action);
                }
                Tab::Settings => {
                    let action = self.settings.handle_key(key);
                    self.handle_settings_action(action);
                }
                Tab::Learning => {
                    let action = self.learning.handle_key(key);
                    self.handle_learning_action(action);
                }
                Tab::SkillsProposed => {
                    let action = self.skills_proposed.handle_key(key);
                    self.handle_skills_proposed_action(action);
                }
                Tab::Cron => {
                    let action = self.cron.handle_key(key);
                    self.handle_cron_action(action);
                }
                Tab::Approvals => {
                    let action = self.approvals.handle_key(key);
                    self.handle_approvals_action(action);
                }
                Tab::Budget => {
                    let action = self.budget.handle_key(key);
                    self.handle_budget_action(action);
                }
                Tab::Graph => {
                    let action = self.graph.handle_key(key);
                    self.handle_graph_action(action);
                }
                _ => {}
            },
        }
        true
    }

    fn handle_hub_shortcut(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        match hub_shortcut_route_for_key(self.active_tab, key.code, key.modifiers) {
            Some(HubShortcutRoute::Automation(action)) => {
                automation_view_after_shortcut(self.automation_view, action)
                    .map(|view| self.switch_automation_view(view))
                    .is_some()
            }
            Some(HubShortcutRoute::Learning(action)) => {
                learning_view_after_shortcut(self.learning_view, action)
                    .map(|view| self.switch_learning_view(view))
                    .is_some()
            }
            Some(HubShortcutRoute::Capabilities(action)) => {
                capabilities_view_after_shortcut(self.capabilities_view, action)
                    .map(|view| self.switch_capabilities_view(view))
                    .is_some()
            }
            Some(HubShortcutRoute::Connections(action)) => {
                connections_view_after_shortcut(self.connections_view, action)
                    .map(|view| self.switch_connections_view(view))
                    .is_some()
            }
            None => false,
        }
    }

    fn handle_automation_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        match automation_key_route_for_view(self.automation_view) {
            AutomationKeyRoute::Workflows => {
                let action = self.workflows.handle_key(key);
                self.handle_workflow_action(action);
            }
            AutomationKeyRoute::Triggers => {
                let action = self.triggers.handle_key(key);
                self.handle_trigger_action(action);
            }
            AutomationKeyRoute::Cron => {
                let action = self.cron.handle_key(key);
                self.handle_cron_action(action);
            }
            AutomationKeyRoute::Approvals => {
                let action = self.approvals.handle_key(key);
                self.handle_approvals_action(action);
            }
        }
    }

    fn handle_learning_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        match learning_key_route_for_view(self.learning_view) {
            LearningKeyRoute::Review => {
                let action = self.learning.handle_key(key);
                self.handle_learning_action(action);
            }
            LearningKeyRoute::SkillProposals => {
                let action = self.skills_proposed.handle_key(key);
                self.handle_skills_proposed_action(action);
            }
            LearningKeyRoute::Memory => {
                let action = self.memory.handle_key(key);
                self.handle_memory_action(action);
            }
            LearningKeyRoute::Graph => {
                let action = self.graph.handle_key(key);
                self.handle_graph_action(action);
            }
        }
    }

    fn handle_capabilities_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        match capabilities_key_route_for_view(self.capabilities_view) {
            CapabilitiesKeyRoute::Native => {
                let action = self.native_capabilities.handle_key(key);
                self.handle_native_capabilities_action(action);
            }
            CapabilitiesKeyRoute::Skills => {
                let action = self.skills.handle_key(key);
                self.handle_skills_action(action);
            }
        }
    }

    fn handle_connections_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        match connections_key_route_for_view(self.connections_view) {
            ConnectionsKeyRoute::Channels => {
                let action = self.channels.handle_key(key);
                self.handle_channel_action(action);
            }
            ConnectionsKeyRoute::Extensions => {
                let action = self.extensions.handle_key(key);
                self.handle_extensions_action(action);
            }
            ConnectionsKeyRoute::Peers => {
                let action = self.peers.handle_key(key);
                self.handle_peers_action(action);
            }
            ConnectionsKeyRoute::Comms => {
                let action = self.comms.handle_key(key);
                self.handle_comms_action(action);
            }
        }
    }

    /// Translate a mouse scroll-wheel tick into the active screen's scroll
    /// model. Today only the chat transcript scrolls; other tabs do nothing
    /// so a stray wheel event doesn't reset their state.
    fn handle_scroll(&mut self, up: bool) {
        if matches!(
            non_key_input_route_for_state(self.phase, self.active_tab),
            Some(NonKeyInputRoute::Chat)
        ) {
            self.chat.scroll_offset = chat_scroll_offset_after_wheel(self.chat.scroll_offset, up);
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) {
        if matches!(
            non_key_input_route_for_state(self.phase, self.active_tab),
            Some(NonKeyInputRoute::Chat)
        ) {
            if let Some(effect) = self
                .chat
                .handle_mouse_click(x, y)
                .and_then(chat_mouse_effect_for_action)
            {
                self.apply_chat_mouse_effect(effect);
            }
        }
    }

    fn apply_chat_mouse_effect(&mut self, effect: ChatMouseEffect) {
        match effect {
            ChatMouseEffect::CopyCommand(command) => {
                self.copy_to_clipboard_status(command, "Commande");
            }
            ChatMouseEffect::ApplyModelSwitch {
                model_id,
                session_strategy,
            } => self.switch_model(&model_id, Some(&session_strategy)),
            ChatMouseEffect::ChatAction(action) => self.handle_chat_action(action),
        }
    }

    fn copy_to_clipboard_status(&mut self, text: String, label: &str) {
        let byte_len = text.len();
        let msg = match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone())) {
            Ok(()) => slash_local::copy_success_message(
                slash_local::CopyStatusSurface::FullTui,
                label,
                byte_len,
            ),
            Err(e) => slash_local::copy_failure_message(slash_local::CopyStatusSurface::FullTui, e),
        };
        self.chat.push_message(chat::Role::System, msg);
    }

    /// Route a bracketed-paste blob to the active screen.
    ///
    /// Only the chat input meaningfully consumes pasted content today; other
    /// screens have no free-text input field, so paste is silently dropped
    /// rather than leaking into navigation state.
    fn handle_paste(&mut self, data: String) {
        match paste_effect_for_state(self.phase, self.active_tab, &data) {
            Some(PasteEffect::AttachPath(path)) => {
                self.handle_image_attach(&path.to_string_lossy())
            }
            Some(PasteEffect::PasteText) => self.chat.handle_paste(&data),
            None => {}
        }
    }

    fn handle_tick(&mut self) {
        self.tick_count = next_tick_count(self.tick_count);
        if should_clear_ctrl_c_pending(self.ctrl_c_pending, self.tick_count, self.ctrl_c_tick) {
            self.ctrl_c_pending = false;
        }

        for route in screen_tick_routes() {
            self.tick_screen_route(*route);
        }

        if should_poll_pending_approval(
            self.chat.is_streaming,
            self.chat.pending_approval.is_some(),
            self.tick_count,
        ) {
            self.poll_pending_approval_for_chat();
        }

        if let Some(route) =
            auto_poll_route_for_state(self.phase, self.active_tab, self.connections_view)
        {
            match route {
                AutoPollRoute::Logs if self.logs.should_poll() => self.refresh_logs(),
                AutoPollRoute::Peers if self.peers.should_poll() => self.refresh_peers(),
                AutoPollRoute::Comms if self.comms.should_poll() => self.refresh_comms(),
                AutoPollRoute::ProjectRuntime if self.projects.should_poll_runtime() => {
                    self.refresh_active_project_detail(false)
                }
                _ => {}
            }
        }
    }

    fn tick_screen_route(&mut self, route: ScreenTickRoute) {
        match route {
            ScreenTickRoute::Welcome => self.welcome.tick(),
            ScreenTickRoute::Chat => self.chat.tick(),
            ScreenTickRoute::Dashboard => self.dashboard.tick(),
            ScreenTickRoute::Channels => self.channels.tick(),
            ScreenTickRoute::Workflows => self.workflows.tick(),
            ScreenTickRoute::Triggers => self.triggers.tick(),
            ScreenTickRoute::Sessions => self.sessions.tick(),
            ScreenTickRoute::Memory => self.memory.tick(),
            ScreenTickRoute::Skills => self.skills.tick(),
            ScreenTickRoute::Hands => self.hands.tick(),
            ScreenTickRoute::Extensions => self.extensions.tick(),
            ScreenTickRoute::Templates => self.templates.tick(),
            ScreenTickRoute::Security => self.security.tick(),
            ScreenTickRoute::Audit => self.audit.tick(),
            ScreenTickRoute::Usage => self.usage.tick(),
            ScreenTickRoute::Settings => self.settings.tick(),
            ScreenTickRoute::Peers => self.peers.tick(),
            ScreenTickRoute::Comms => self.comms.tick(),
            ScreenTickRoute::Logs => self.logs.tick(),
            ScreenTickRoute::Projects => self.projects.tick(),
            ScreenTickRoute::Learning => self.learning.tick(),
            ScreenTickRoute::SkillsProposed => self.skills_proposed.tick(),
            ScreenTickRoute::Cron => self.cron.tick(),
            ScreenTickRoute::Approvals => self.approvals.tick(),
            ScreenTickRoute::Budget => self.budget.tick(),
            ScreenTickRoute::Graph => self.graph.tick(),
        }
    }

    // ─── Tab navigation ──────────────────────────────────────────────────────

    fn next_tab(&mut self) {
        self.switch_tab(next_primary_tab(self.active_tab));
    }

    fn prev_tab(&mut self) {
        self.switch_tab(previous_primary_tab(self.active_tab));
    }

    fn switch_tab(&mut self, tab: Tab) {
        let state = tab_switch_state_after_switch(self.tab_scroll_offset, tab);
        self.active_tab = state.active_tab;
        self.tab_scroll_offset = state.scroll_offset;
        // Will be further adjusted during draw based on actual width
        self.on_tab_enter(state.active_tab);
    }

    /// Phase-f.13: open a modal overlay on top of the current chat.
    /// Reuses `on_tab_enter` so the overlay's data is loaded lazily,
    /// exactly as when the tab itself is focused.
    fn open_overlay(&mut self, tab: Tab) {
        self.apply_overlay_state(overlay_state_after_open(tab));
    }

    fn close_overlay(&mut self) {
        self.apply_overlay_state(overlay_state_after_close());
    }

    fn apply_overlay_state(&mut self, state: OverlayState) {
        self.overlay_tab = state.overlay_tab;
        if let Some(tab) = state.enter_tab {
            self.on_tab_enter(tab);
        }
    }

    fn switch_automation_view(&mut self, view: AutomationView) {
        let state = automation_view_state_after_switch(view);
        self.automation_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn open_automation_view(&mut self, view: AutomationView) {
        let state = automation_view_state_after_open(view);
        self.automation_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn switch_learning_view(&mut self, view: LearningView) {
        let state = learning_view_state_after_switch(view);
        self.learning_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn open_learning_view(&mut self, view: LearningView) {
        let state = learning_view_state_after_open(view);
        self.learning_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn switch_capabilities_view(&mut self, view: CapabilitiesView) {
        let state = capabilities_view_state_after_switch(view);
        self.capabilities_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn open_capabilities_view(&mut self, view: CapabilitiesView) {
        let state = capabilities_view_state_after_open(view);
        self.capabilities_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn switch_connections_view(&mut self, view: ConnectionsView) {
        let state = connections_view_state_after_switch(view);
        self.connections_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn open_connections_view(&mut self, view: ConnectionsView) {
        let state = connections_view_state_after_open(view);
        self.connections_view = state.view;
        self.apply_hub_view_effect(state.effect);
    }

    fn apply_hub_view_effect(&mut self, effect: HubViewEffect) {
        match effect {
            HubViewEffect::RefreshAutomationCurrent => self.refresh_automation_current(),
            HubViewEffect::RefreshLearningCurrent => self.refresh_learning_current(),
            HubViewEffect::RefreshCapabilitiesCurrent => self.refresh_capabilities_current(),
            HubViewEffect::RefreshConnectionsCurrent => self.refresh_connections_current(),
            HubViewEffect::SwitchTab(tab) => self.switch_tab(tab),
        }
    }

    /// Called when a tab becomes active — load data if needed.
    fn on_tab_enter(&mut self, tab: Tab) {
        if let Some(route) = tab_refresh_route_for_tab(tab) {
            self.refresh_tab_route(route);
        }
    }

    fn refresh_tab_route(&mut self, route: TabRefreshRoute) {
        match route {
            TabRefreshRoute::Dashboard => self.refresh_dashboard(),
            TabRefreshRoute::Agents => self.refresh_agents(),
            TabRefreshRoute::Projects => self.refresh_projects(),
            TabRefreshRoute::ConnectionsCurrent => self.refresh_connections_current(),
            TabRefreshRoute::AutomationCurrent => self.refresh_automation_current(),
            TabRefreshRoute::Triggers => self.refresh_triggers(),
            TabRefreshRoute::Cron => self.refresh_cron(),
            TabRefreshRoute::Approvals => self.refresh_approvals(),
            TabRefreshRoute::Budget => self.refresh_budget(),
            TabRefreshRoute::Graph => self.refresh_graph(),
            TabRefreshRoute::Sessions => self.refresh_sessions(),
            TabRefreshRoute::Memory => self.refresh_memory(),
            TabRefreshRoute::LearningCurrent => self.refresh_learning_current(),
            TabRefreshRoute::SkillsProposed => self.refresh_skills_proposed(),
            TabRefreshRoute::CapabilitiesCurrent => self.refresh_capabilities_current(),
            TabRefreshRoute::Hands => self.refresh_hands(),
            TabRefreshRoute::Extensions => self.refresh_extensions(),
            TabRefreshRoute::Templates => self.refresh_templates(),
            TabRefreshRoute::Security => self.refresh_security(),
            TabRefreshRoute::Audit => self.refresh_audit(),
            TabRefreshRoute::Usage => self.refresh_usage(),
            TabRefreshRoute::SettingsProviders => self.refresh_settings_providers(),
            TabRefreshRoute::Peers => self.refresh_peers(),
            TabRefreshRoute::Comms => self.refresh_comms(),
            TabRefreshRoute::Logs => self.refresh_logs(),
        }
    }

    /// Transition from Boot to Main phase.
    ///
    /// Phase-f.5: land on `Tab::Chat` for a chat-first UX.
    /// The agents list is still pre-loaded in the background so the user
    /// can switch agents without waiting.
    fn enter_main_phase(&mut self) {
        let plan = main_phase_entry_plan();
        self.phase = Phase::Main;
        self.active_tab = plan.active_tab;
        for route in plan.routes {
            self.apply_main_phase_entry_route(*route);
        }
        self.start_provider_quota_watch();
    }

    fn start_provider_quota_watch(&mut self) {
        if self.provider_quota_watch_started {
            return;
        }
        let Some(backend) = self.backend.to_ref() else {
            return;
        };
        self.provider_quota_watch_started = true;
        event::spawn_provider_quota_watch(backend, self.event_tx.clone());
    }

    fn apply_main_phase_entry_route(&mut self, route: MainPhaseEntryRoute) {
        match route {
            MainPhaseEntryRoute::RefreshAgents => self.refresh_agents(),
            MainPhaseEntryRoute::RefreshDashboard => self.refresh_dashboard(),
            MainPhaseEntryRoute::RefreshChannels => self.refresh_channels(),
            MainPhaseEntryRoute::AutoSelectDefaultAgent => self.auto_select_default_agent(),
            MainPhaseEntryRoute::ApplyWorkspaceExtraPaths => self.apply_workspace_extra_paths(),
        }
    }

    /// Apply `.captain.toml` `extra_paths` to the kernel sandbox so the
    /// agent can immediately reach those directories. The list has already
    /// been canonicalised and sandbox-checked in `App::new`; here we just
    /// route each path to the right backend.
    ///
    /// In daemon mode this issues `POST /api/workspace/add` per path; failures
    /// log at warn but never abort the bind, since `extra_paths` is
    /// explicitly best-effort.
    fn apply_workspace_extra_paths(&self) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };
        if ws.config.captain.extra_paths.is_empty() {
            return;
        }
        for path in &ws.config.captain.extra_paths {
            match &self.backend {
                Backend::InProcess { kernel } => {
                    use captain_runtime::kernel_handle::KernelHandle;
                    if let Err(e) = kernel.add_workspace_path(path) {
                        tracing::warn!(
                            error = %e,
                            path = %path.display(),
                            "workspace extra_path rejected by kernel"
                        );
                    } else {
                        tracing::info!(
                            path = %path.display(),
                            "applied workspace extra_path"
                        );
                    }
                }
                Backend::Daemon { base_url } => {
                    let url = format!("{base_url}/api/workspace/add");
                    let body = serde_json::json!({ "path": path.display().to_string() });
                    let path_clone = path.clone();
                    std::thread::spawn(move || {
                        let client = crate::daemon_client();
                        match client.post(&url).json(&body).send() {
                            Ok(resp) if resp.status().is_success() => {
                                tracing::info!(
                                    path = %path_clone.display(),
                                    "applied workspace extra_path (daemon)"
                                );
                            }
                            Ok(resp) => {
                                tracing::warn!(
                                    status = %resp.status(),
                                    path = %path_clone.display(),
                                    "workspace extra_path rejected by daemon"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    path = %path_clone.display(),
                                    "workspace extra_path POST failed"
                                );
                            }
                        }
                    });
                }
                Backend::None => {}
            }
        }
    }

    /// Pick a default chat target so a user landing on the Chat tab can type
    /// immediately.
    ///
    /// Preference order:
    /// 1. existing chat_target,
    /// 2. owner of the session accepted in the boot resume prompt,
    /// 3. `.captain.toml` `agent` UUID (workspace bind),
    /// 4. `.captain.toml` `agent_name` match (workspace bind),
    /// 5. agent named "captain",
    /// 6. first agent in the registry.
    ///
    /// No-op when nothing matches (the chat will then surface a guiding
    /// error on send).
    fn auto_select_default_agent(&mut self) {
        if self.chat_target.is_some() {
            return;
        }
        let pending_agent_uuid = self
            .pending_resume_target
            .as_ref()
            .and_then(|(_, agent_id, _)| agent_id.as_deref())
            .map(str::to_string);
        let pending_agent_name = self
            .pending_resume_target
            .as_ref()
            .map(|(_, _, agent_name)| agent_name.clone());
        let ws_agent_uuid = self
            .workspace
            .as_ref()
            .and_then(|w| w.config.captain.agent.as_deref())
            .map(|s| s.to_string());
        let ws_agent_name = self
            .workspace
            .as_ref()
            .and_then(|w| w.config.captain.agent_name.as_deref())
            .map(|s| s.to_string());
        let preference = default_agent::AgentPreference {
            id: pending_agent_uuid.as_deref().or(ws_agent_uuid.as_deref()),
            name: pending_agent_name.as_deref().or(ws_agent_name.as_deref()),
        };
        match &self.backend {
            Backend::InProcess { kernel } => {
                let entries: Vec<_> = kernel.registry.list().into_iter().collect();
                let pick = default_agent::select_index(
                    &entries,
                    preference,
                    |entry, agent_id| {
                        agent_id
                            .parse::<AgentId>()
                            .map(|id| entry.id == id)
                            .unwrap_or(false)
                    },
                    |entry| Some(entry.name.as_str()),
                )
                .and_then(|index| entries.get(index));
                if let Some(entry) = pick {
                    let id = entry.id;
                    let name = entry.name.clone();
                    self.enter_chat_inprocess(id, name);
                }
            }
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        if let Some(arr) = body.as_array() {
                            let pick = default_agent::select_index(
                                arr,
                                preference,
                                |agent, agent_id| agent["id"].as_str() == Some(agent_id),
                                |agent| agent["name"].as_str(),
                            )
                            .and_then(|index| arr.get(index))
                            .and_then(default_agent::daemon_agent_identity);
                            if let Some((id, name)) = pick {
                                self.enter_chat_daemon(id, name);
                            }
                        }
                    }
                }
            }
            Backend::None => {}
        }
    }

    // ─── Data refresh helpers ────────────────────────────────────────────────

    fn refresh_dashboard(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.dashboard.loading = true;
            event::spawn_fetch_dashboard(backend, self.event_tx.clone());
        }
    }

    fn refresh_agents(&mut self) {
        match &self.backend {
            Backend::Daemon { base_url } => {
                self.agents.load_daemon_agents(base_url);
            }
            Backend::InProcess { kernel } => {
                self.agents.load_inprocess_agents(kernel);
            }
            Backend::None => {}
        }
    }

    fn refresh_channels(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.channels.loading = true;
            event::spawn_fetch_channels(backend, self.event_tx.clone());
        }
        // Also build defaults from env detection
        if self.channels.channels.is_empty() {
            self.channels.build_default_channels();
        }
    }

    fn refresh_automation_current(&mut self) {
        self.refresh_automation_route(automation_refresh_route_for_view(self.automation_view));
    }

    fn refresh_automation_route(&mut self, route: AutomationRefreshRoute) {
        match route {
            AutomationRefreshRoute::Workflows => self.refresh_workflows(),
            AutomationRefreshRoute::Triggers => self.refresh_triggers(),
            AutomationRefreshRoute::Cron => self.refresh_cron(),
            AutomationRefreshRoute::Approvals => self.refresh_approvals(),
        }
    }

    fn refresh_workflows(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.workflows.loading = true;
            event::spawn_fetch_workflows(backend, self.event_tx.clone());
        }
    }

    fn refresh_projects(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.projects.loading = true;
            event::spawn_fetch_projects(backend, self.event_tx.clone());
        }
    }

    fn refresh_active_project_detail(&mut self, show_loading: bool) {
        let Some(id) = self.projects.active_project_id() else {
            return;
        };
        if let Some(backend) = self.backend.to_ref() {
            self.projects.loading = show_loading;
            event::spawn_resume_project(backend, id, self.event_tx.clone());
        }
    }

    fn refresh_triggers(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.triggers.loading = true;
            event::spawn_fetch_triggers(backend, self.event_tx.clone());
        }
    }

    fn refresh_sessions(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.sessions.loading = true;
            event::spawn_fetch_sessions(backend, self.event_tx.clone());
        }
    }

    fn refresh_memory(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.memory.loading = true;
            event::spawn_fetch_memory_agents(backend, self.event_tx.clone());
        }
    }

    fn refresh_learning_current(&mut self) {
        self.refresh_learning_route(learning_refresh_route_for_view(self.learning_view));
    }

    fn refresh_learning_route(&mut self, route: LearningRefreshRoute) {
        match route {
            LearningRefreshRoute::Review => self.refresh_learning(),
            LearningRefreshRoute::SkillProposals => self.refresh_skills_proposed(),
            LearningRefreshRoute::Memory => self.refresh_memory(),
            LearningRefreshRoute::Graph => self.refresh_graph(),
        }
    }

    fn refresh_learning(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.learning.loading = true;
            event::spawn_fetch_learning(backend, self.event_tx.clone());
        }
    }

    fn refresh_skills_proposed(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.skills_proposed.loading = true;
            event::spawn_fetch_skills_proposed(backend, self.event_tx.clone());
        }
    }

    fn refresh_cron(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.cron.loading = true;
            event::spawn_fetch_cron(backend, self.event_tx.clone());
        }
    }

    /// Phase-i.6: fire a one-shot poll for an approval matching the chat agent.
    /// Triggered from handle_tick during streaming; the response sets
    /// chat.pending_approval so the in-chat modal pops up.
    fn poll_pending_approval_for_chat(&mut self) {
        let agent_name = match &self.chat_target {
            Some(t) if !t.agent_name.is_empty() => t.agent_name.clone(),
            _ => return,
        };
        if let Some(backend) = self.backend.to_ref() {
            event::spawn_poll_chat_approval(backend, agent_name, self.event_tx.clone());
        }
    }

    fn refresh_approvals(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.approvals.loading = true;
            event::spawn_fetch_approvals(backend, self.event_tx.clone());
        }
    }

    fn handle_approvals_action(&mut self, action: approvals::ApprovalsAction) {
        match action {
            approvals::ApprovalsAction::Continue => {}
            approvals::ApprovalsAction::Refresh => self.refresh_approvals(),
            approvals::ApprovalsAction::Approve(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(backend, id, "approve", self.event_tx.clone());
                }
            }
            approvals::ApprovalsAction::ApproveSession(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(
                        backend,
                        id,
                        "approve_session",
                        self.event_tx.clone(),
                    );
                }
            }
            approvals::ApprovalsAction::ApproveAlways(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(
                        backend,
                        id,
                        "approve_always",
                        self.event_tx.clone(),
                    );
                }
            }
            approvals::ApprovalsAction::Reject(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(backend, id, "reject", self.event_tx.clone());
                }
            }
        }
    }

    fn refresh_budget(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.budget.loading = true;
            event::spawn_fetch_budget(backend, self.event_tx.clone());
        }
    }

    fn handle_budget_action(&mut self, action: budget::BudgetAction) {
        match action {
            budget::BudgetAction::Continue => {}
            budget::BudgetAction::Refresh => self.refresh_budget(),
        }
    }

    fn refresh_graph(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.graph.loading = true;
            event::spawn_fetch_graph(backend, self.event_tx.clone());
        }
    }

    fn handle_graph_action(&mut self, action: graph::GraphAction) {
        match action {
            graph::GraphAction::Continue => {}
            graph::GraphAction::Refresh => self.refresh_graph(),
            graph::GraphAction::Search(query) => {
                if let Some(backend) = self.backend.to_ref() {
                    self.graph.loading = true;
                    event::spawn_search_graph(backend, query, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_cron_action(&mut self, action: cron::CronAction) {
        match action {
            cron::CronAction::Continue => {}
            cron::CronAction::Refresh => self.refresh_cron(),
            cron::CronAction::Toggle(id) => {
                let enabled = self
                    .cron
                    .jobs
                    .iter()
                    .find(|j| j.id == id)
                    .map(|j| !j.enabled)
                    .unwrap_or(true);
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_cron_toggle(backend, id, enabled, self.event_tx.clone());
                }
            }
            cron::CronAction::RunNow(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_cron_run_now(backend, id, self.event_tx.clone());
                }
            }
            cron::CronAction::Delete(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_cron_delete(backend, id, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_skills_proposed_action(&mut self, action: skills_proposed::SkillsProposedAction) {
        match action {
            skills_proposed::SkillsProposedAction::Continue => {}
            skills_proposed::SkillsProposedAction::Refresh => self.refresh_skills_proposed(),
            skills_proposed::SkillsProposedAction::Decide {
                proposal_id,
                operator_token,
                decision_version,
                action,
            } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_workflow_proposal(
                        backend,
                        proposal_id,
                        operator_token,
                        decision_version,
                        action,
                        self.event_tx.clone(),
                    );
                }
            }
        }
    }

    fn handle_learning_action(&mut self, action: learning::LearningAction) {
        match action {
            learning::LearningAction::Continue => {}
            learning::LearningAction::Refresh => self.refresh_learning(),
            learning::LearningAction::Approve(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_learning(backend, id, true, self.event_tx.clone());
                }
            }
            learning::LearningAction::Deny(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_learning(backend, id, false, self.event_tx.clone());
                }
            }
        }
    }

    fn refresh_skills(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.skills.loading = true;
            event::spawn_fetch_skills(backend, self.event_tx.clone());
        }
    }

    fn refresh_native_capabilities(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.native_capabilities.loading = true;
            event_native_capabilities::spawn_fetch(
                backend,
                self.native_capabilities.scope,
                self.native_capability_workspace(),
                self.event_tx.clone(),
            );
        }
    }

    fn refresh_capabilities_current(&mut self) {
        self.refresh_capabilities_route(capabilities_refresh_route_for_view(
            self.capabilities_view,
        ));
    }

    fn refresh_capabilities_route(&mut self, route: CapabilitiesRefreshRoute) {
        match route {
            CapabilitiesRefreshRoute::Native => self.refresh_native_capabilities(),
            CapabilitiesRefreshRoute::Skills => self.refresh_skills(),
        }
    }

    fn refresh_hands(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.hands.loading = true;
            event::spawn_fetch_hands(backend.clone(), self.event_tx.clone());
            event::spawn_fetch_active_hands(backend, self.event_tx.clone());
        }
    }

    fn refresh_extensions(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.extensions.loading = true;
            event::spawn_fetch_extensions(backend, self.event_tx.clone());
        }
    }

    fn refresh_connections_current(&mut self) {
        self.refresh_connections_route(connections_refresh_route_for_view(self.connections_view));
    }

    fn refresh_connections_route(&mut self, route: ConnectionsRefreshRoute) {
        match route {
            ConnectionsRefreshRoute::Channels => self.refresh_channels(),
            ConnectionsRefreshRoute::Extensions => self.refresh_extensions(),
            ConnectionsRefreshRoute::Peers => self.refresh_peers(),
            ConnectionsRefreshRoute::Comms => self.refresh_comms(),
        }
    }

    fn refresh_extension_health(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            event::spawn_fetch_extension_health(backend, self.event_tx.clone());
        }
    }

    fn refresh_templates(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            event::spawn_fetch_template_providers(backend, self.event_tx.clone());
        }
    }

    fn refresh_security(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.security.loading = true;
            event::spawn_fetch_security(backend, self.event_tx.clone());
        }
    }

    fn refresh_audit(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.audit.loading = true;
            event::spawn_fetch_audit(backend, self.event_tx.clone());
        }
    }

    fn refresh_usage(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.usage.loading = true;
            event::spawn_fetch_usage(backend, self.event_tx.clone());
        }
    }

    fn refresh_settings_providers(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.settings.loading = true;
            event::spawn_fetch_providers(backend, self.event_tx.clone());
        }
    }

    fn refresh_settings_models(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.settings.loading = true;
            event::spawn_fetch_models(backend, self.event_tx.clone());
        }
    }

    fn refresh_settings_tools(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            event::spawn_fetch_tools(backend, self.event_tx.clone());
        }
    }

    fn refresh_peers(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.peers.loading = true;
            event::spawn_fetch_peers(backend, self.event_tx.clone());
        }
    }

    fn refresh_comms(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.comms.loading = true;
            event::spawn_fetch_comms(backend, self.event_tx.clone());
        }
    }

    fn refresh_logs(&mut self) {
        if let Some(backend) = self.backend.to_ref() {
            self.logs.loading = true;
            event::spawn_fetch_logs(backend, self.event_tx.clone());
        }
    }

    // ─── Streaming ───────────────────────────────────────────────────────────

    fn handle_stream(&mut self, ev: StreamEvent) {
        stream_route::apply_stream_event(&mut self.chat, ev);
    }

    fn handle_stream_done(
        &mut self,
        result: Result<captain_runtime::agent_loop::AgentLoopResult, String>,
    ) {
        self.current_stream_input_tx = None;
        stream_lifecycle::apply_stream_result(&mut self.chat, result);
        self.refresh_active_chat_metadata();
        // Phase L.3: persiste l'historique après chaque tour LLM.
        self.chat.persist_session();
        // Auto-send the next staged message if any
        if let Some(msg) = self.chat.take_staged() {
            self.send_message(msg);
        }
    }

    fn refresh_active_chat_metadata(&mut self) {
        match (&self.backend, self.chat_target.as_ref()) {
            (Backend::Daemon { base_url }, Some(target)) => {
                let Some(agent_id) = target.agent_id_daemon.as_deref() else {
                    return;
                };
                let client = crate::daemon_client();
                let Ok(resp) = client
                    .get(format!("{base_url}/api/agents/{agent_id}"))
                    .send()
                else {
                    return;
                };
                let Ok(body) = resp.json::<serde_json::Value>() else {
                    return;
                };
                self.chat.apply_agent_runtime_metadata(&body);
            }
            (Backend::InProcess { kernel }, Some(target)) => {
                let Some(agent_id) = target.agent_id_inprocess else {
                    return;
                };
                if let Some(entry) = kernel.registry.get(agent_id) {
                    self.chat.model_label = format!(
                        "{}/{}",
                        entry.manifest.model.provider, entry.manifest.model.model
                    );
                }
                if let Some(context_window) = kernel.effective_context_window_for_agent(agent_id) {
                    self.chat.set_context_window_tokens(context_window as u64);
                }
            }
            _ => {}
        }
    }

    // ─── Kernel lifecycle ────────────────────────────────────────────────────

    fn handle_kernel_ready(&mut self, kernel: Arc<CaptainKernel>) {
        self.kernel_booting = false;
        // Phase O.2: subscribe to broadcast events so we surface
        // auto-memorize commits in the chat.
        event::spawn_memory_subscriber(Arc::clone(&kernel), self.event_tx.clone());
        event::spawn_inprocess_capspec_resume_recovery(Arc::clone(&kernel));
        self.backend = Backend::InProcess { kernel };
        self.agents.reset();
        self.enter_main_phase();
    }

    fn handle_kernel_error(&mut self, err: String) {
        self.kernel_booting = false;
        self.kernel_boot_error = Some(err.clone());
        if err.contains("Missing API key") || err.contains("api_key") {
            self.wizard.reset();
            self.phase = Phase::Boot(BootScreen::Wizard);
        } else {
            self.phase = Phase::Boot(BootScreen::Welcome);
            self.start_daemon_detect();
        }
    }

    fn handle_agent_spawned(
        &mut self,
        id: String,
        name: String,
        api_sheet: Option<crate::agent_api_sheet::AgentApiSpawnSheet>,
    ) {
        self.agents.sub = agents::AgentSubScreen::AgentList;
        self.enter_chat_daemon(id, name);
        self.push_agent_api_spawn_notice(api_sheet);
    }

    fn push_agent_api_spawn_notice(
        &mut self,
        api_sheet: Option<crate::agent_api_sheet::AgentApiSpawnSheet>,
    ) {
        match api_sheet {
            Some(sheet) => self.chat.push_operator_notice(sheet.tui_notice_lines()),
            None => self.chat.push_operator_notice(vec![
                "Agent created.".to_string(),
                "Agent API sheet unavailable in this response.".to_string(),
                "Run `captain agent api <agent> --rotate-token` to inspect ingress/egress and get a fresh token.".to_string(),
            ]),
        }
    }

    fn handle_agent_spawn_error(&mut self, err: String) {
        self.agents.status_msg = err;
        self.agents.sub = agents::AgentSubScreen::AgentList;
    }

    // ─── Screen transitions ──────────────────────────────────────────────────

    fn start_daemon_detect(&mut self) {
        self.welcome.detecting = true;
        event::spawn_daemon_detect(self.event_tx.clone());
    }

    /// Boot ResumePrompt: user accepted. Stashes (a) the visual replay
    /// info so `enter_chat_{daemon,inprocess}` can rehydrate the chat
    /// AFTER its mandatory `reset()` call, and (b) the kernel-format
    /// messages so the backend agent's context can be restored once the
    /// agent id is resolved. Then falls through to the normal Welcome
    /// auto-detect path so the connection comes up.
    ///
    /// Note: we do NOT call `replay_session_from` directly here — boot
    /// goes through `enter_main_phase` → `auto_select_default_agent` →
    /// `enter_chat_*` → `chat.reset()`, which would wipe whatever
    /// history we'd just loaded. The bug fix (#182) is to defer the
    /// replay until *after* that reset.
    fn accept_resume(&mut self) {
        let Some(summary) = self.pending_resume.take() else {
            return;
        };
        let agent_key = summary.agent_key.clone();
        let path = summary.path.clone();
        if let Some(loaded) = session_store::load_session_at(&path) {
            if let Some(session_id) = loaded.session_id.clone() {
                self.pending_resume_target = Some((
                    session_id,
                    loaded.agent_id.clone(),
                    loaded.agent_name.clone(),
                ));
            }
            self.pending_restore_messages = Some(session_store::to_kernel_messages(&loaded));
        }
        self.pending_chat_replay = Some((agent_key, path));
        self.phase = Phase::Boot(BootScreen::Welcome);
        self.start_daemon_detect();
    }

    /// Boot ResumePrompt: user refused. Drop the candidate and fall
    /// through to the empty Welcome screen (legacy boot behavior).
    fn decline_resume(&mut self) {
        self.pending_resume = None;
        self.pending_resume_target = None;
        self.pending_restore_messages = None;
        self.pending_chat_replay = None;
        self.phase = Phase::Boot(BootScreen::Welcome);
        self.start_daemon_detect();
    }

    /// Drain the kernel-format messages stashed by accept_resume and
    /// rehydrate them on the agent. Daemon mode posts to
    /// /api/agents/{id}/session/restore in the background; in-process
    /// mode calls the kernel method directly. Errors log at warn but
    /// never break the chat — the visual replay always works.
    fn apply_pending_restore(&mut self, daemon_id: Option<&str>, inprocess_id: Option<AgentId>) {
        if self.pending_resume_target.is_some() {
            // An authoritative UUID is still waiting for its owning agent.
            // Never import its transcript into whichever fallback agent was
            // selected first.
            return;
        }
        let Some(messages) = self.pending_restore_messages.take() else {
            return;
        };
        if messages.is_empty() {
            return;
        }
        match (&self.backend, daemon_id, inprocess_id) {
            (Backend::Daemon { base_url }, Some(id), _) => {
                let url = format!("{base_url}/api/agents/{id}/session/restore");
                let body = serde_json::json!({ "messages": messages });
                std::thread::spawn(move || {
                    let client = crate::daemon_client();
                    if let Err(e) = client.post(&url).json(&body).send() {
                        tracing::warn!(error = %e, url = %url, "session restore POST failed");
                    }
                });
            }
            (Backend::InProcess { kernel }, _, Some(id)) => {
                if let Err(e) = kernel.restore_agent_session(id, messages) {
                    tracing::warn!(error = %e, "session restore (in-process) failed");
                }
            }
            _ => {}
        }
    }

    fn handle_welcome_action(&mut self, action: welcome::WelcomeAction) {
        match action {
            welcome::WelcomeAction::Exit => self.should_quit = true,
            welcome::WelcomeAction::ConnectDaemon => {
                if let Some(ref url) = self.welcome.daemon_url {
                    let url_clone = url.clone();
                    self.backend = Backend::Daemon {
                        base_url: url_clone.clone(),
                    };
                    // Phase O.3: souscrire au flux SSE des commits memorize.
                    event::spawn_daemon_memory_subscriber(url_clone, self.event_tx.clone());
                    self.agents.reset();
                    self.enter_main_phase();
                }
            }
            welcome::WelcomeAction::InProcess => {
                // Late-detection guard: between the initial async daemon probe
                // and this InProcess decision, a daemon may have come up (or
                // the async probe may not have landed yet because the user
                // submitted the welcome menu first). Booting a second kernel
                // here would re-instance the channel bridge and collide with
                // the daemon's adapters (Telegram 409, Discord 60s reconnect).
                // Re-check synchronously and switch to ConnectDaemon if a
                // daemon answers /api/health on a known address.
                if std::env::var("CAPTAIN_NO_AUTO_DAEMON").is_err() {
                    if let Some(url) = crate::find_daemon() {
                        tracing::info!(daemon_url = %url, "Late-detected daemon, switching to ConnectDaemon to avoid bridge collision");
                        self.welcome.on_daemon_detected(Some(url.clone()), 0);
                        self.backend = Backend::Daemon {
                            base_url: url.clone(),
                        };
                        event::spawn_daemon_memory_subscriber(url, self.event_tx.clone());
                        self.agents.reset();
                        self.enter_main_phase();
                        return;
                    }
                }

                if self.kernel_booting {
                    return;
                }
                self.kernel_booting = true;
                self.kernel_boot_error = None;
                event::spawn_kernel_boot(self.config_path.clone(), self.event_tx.clone());
            }
            welcome::WelcomeAction::Wizard => {
                self.wizard.reset();
                self.phase = Phase::Boot(BootScreen::Wizard);
            }
        }
    }

    // ─── Tab action handlers ─────────────────────────────────────────────────

    fn handle_dashboard_action(&mut self, action: dashboard::DashboardAction) {
        match action {
            dashboard::DashboardAction::Continue => {}
            dashboard::DashboardAction::Refresh => self.refresh_dashboard(),
        }
    }

    fn handle_agent_action(&mut self, action: agents::AgentAction) {
        match action {
            agents::AgentAction::Continue => {}
            agents::AgentAction::Back => {
                // In Main phase, Esc from agents just stays on the tab
            }
            agents::AgentAction::CreatedManifest(toml_content) => {
                self.spawn_agent(toml_content);
            }
            agents::AgentAction::ChatWithAgent { id, name } => {
                // From detail view — enter chat with this agent
                if let Some(agent) = self.agents.daemon_agents.iter().find(|a| a.id == id) {
                    self.enter_chat_daemon(agent.id.clone(), agent.name.clone());
                } else if let Some(agent) = self
                    .agents
                    .inprocess_agents
                    .iter()
                    .find(|a| format!("{}", a.id) == id)
                {
                    self.enter_chat_inprocess(agent.id, agent.name.clone());
                } else {
                    // Fallback: treat as daemon
                    self.enter_chat_daemon(id, name);
                }
            }
            agents::AgentAction::KillAgent(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_kill_agent(backend, id, self.event_tx.clone());
                }
            }
            agents::AgentAction::UpdateSkills { id, skills } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_update_agent_skills(backend, id, skills, self.event_tx.clone());
                }
            }
            agents::AgentAction::UpdateMcpServers { id, servers } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_update_agent_mcp_servers(
                        backend,
                        id,
                        servers,
                        self.event_tx.clone(),
                    );
                }
            }
            agents::AgentAction::FetchAgentSkills(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_fetch_agent_skills(backend, id, self.event_tx.clone());
                }
            }
            agents::AgentAction::FetchAgentMcpServers(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_fetch_agent_mcp_servers(backend, id, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_chat_action(&mut self, action: chat::ChatAction) {
        match action {
            chat::ChatAction::Continue => {}
            chat::ChatAction::Back => {
                // In Main phase, go back to Agents tab
                self.chat.reset();
                self.chat_target = None;
                self.switch_tab(Tab::Agents);
            }
            chat::ChatAction::SendMessage(msg) => self.send_message(msg),
            chat::ChatAction::SlashCommand(cmd) => self.handle_slash_command(&cmd),
            chat::ChatAction::ResumeSession(session_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_load_session(backend, session_id, self.event_tx.clone());
                }
            }
            chat::ChatAction::OpenSessionPicker => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_fetch_sessions(backend, self.event_tx.clone());
                }
            }
            chat::ChatAction::OpenModelPicker => self.open_model_picker(),
            chat::ChatAction::SwitchModel(model_id) => self.switch_model(&model_id, None),
            chat::ChatAction::ApplyModelSwitch {
                model_id,
                session_strategy,
            } => self.switch_model(&model_id, Some(&session_strategy)),
            chat::ChatAction::ApproveRequest(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(backend, id, "approve", self.event_tx.clone());
                }
            }
            chat::ChatAction::ApproveSessionRequest(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(
                        backend,
                        id,
                        "approve_session",
                        self.event_tx.clone(),
                    );
                }
            }
            chat::ChatAction::ApproveAlwaysRequest(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(
                        backend,
                        id,
                        "approve_always",
                        self.event_tx.clone(),
                    );
                }
            }
            chat::ChatAction::RejectRequest(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_decide_approval(backend, id, "reject", self.event_tx.clone());
                }
            }
            chat::ChatAction::AnswerAskUser(content) => match &self.backend {
                Backend::InProcess { .. } => {
                    if let Some(tx) = self.current_stream_input_tx.as_ref() {
                        if tx.try_send(content).is_err() {
                            self.current_stream_input_tx = None;
                        }
                    }
                }
                Backend::Daemon { base_url } => {
                    if let Some(agent_id) = self
                        .chat_target
                        .as_ref()
                        .and_then(|t| t.agent_id_daemon.clone())
                    {
                        event::spawn_answer_ask_user(
                            event::BackendRef::Daemon(base_url.clone()),
                            agent_id,
                            self.chat_target
                                .as_ref()
                                .and_then(|target| target.session_id.clone()),
                            content,
                            self.event_tx.clone(),
                        );
                    }
                }
                Backend::None => {}
            },
        }
    }

    fn handle_channel_action(&mut self, action: channels::ChannelAction) {
        match action {
            channels::ChannelAction::Continue => {}
            channels::ChannelAction::Refresh => self.refresh_channels(),
            channels::ChannelAction::TestChannel(name) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_test_channel(backend, name, self.event_tx.clone());
                }
            }
            channels::ChannelAction::ToggleChannel(_name, _enabled) => {
                // Toggle is handled locally in the state; daemon toggle
                // could be spawned here if the API supports it.
            }
            channels::ChannelAction::SaveChannel(name, values) => {
                // Save channel credentials via daemon API
                if let Some(backend) = self.backend.to_ref() {
                    let tx = self.event_tx.clone();
                    std::thread::spawn(move || {
                        if let event::BackendRef::Daemon(base_url) = backend {
                            let client = reqwest::blocking::Client::builder()
                                .timeout(std::time::Duration::from_secs(10))
                                .build()
                                .ok();
                            if let Some(client) = client {
                                let mut fields = serde_json::Map::new();
                                for (k, v) in &values {
                                    fields.insert(k.clone(), serde_json::Value::String(v.clone()));
                                }
                                let body = serde_json::json!({ "fields": fields });
                                let _ = client
                                    .post(format!("{base_url}/api/channels/{name}/configure"))
                                    .json(&body)
                                    .send();
                            }
                        }
                        // Signal tick so the UI refreshes next cycle
                        let _ = tx.send(event::AppEvent::Tick);
                    });
                }
                // Immediately trigger a refresh of the channel list
                self.refresh_channels();
            }
        }
    }

    fn handle_workflow_action(&mut self, action: workflows::WorkflowAction) {
        match action {
            workflows::WorkflowAction::Continue => {}
            workflows::WorkflowAction::Refresh => self.refresh_workflows(),
            workflows::WorkflowAction::LoadRuns(wf_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    self.workflows.loading = true;
                    event::spawn_fetch_workflow_runs(backend, wf_id, self.event_tx.clone());
                }
            }
            workflows::WorkflowAction::CreateWorkflow {
                name,
                description,
                steps_json,
            } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_create_workflow(
                        backend,
                        name,
                        description,
                        steps_json,
                        self.event_tx.clone(),
                    );
                }
            }
            workflows::WorkflowAction::RunWorkflow { id, input } => {
                if let Some(backend) = self.backend.to_ref() {
                    self.workflows.loading = true;
                    event::spawn_run_workflow(backend, id, input, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_project_action(&mut self, action: projects::ProjectAction) {
        let Some(action) = self.handle_project_primary_action(action) else {
            return;
        };
        let Some(action) = self.handle_project_runtime_lifecycle_action(action) else {
            return;
        };
        self.handle_project_goal_action(action);
    }

    fn handle_project_primary_action(
        &mut self,
        action: projects::ProjectAction,
    ) -> Option<projects::ProjectAction> {
        match action {
            projects::ProjectAction::Continue => {}
            projects::ProjectAction::Refresh => self.refresh_projects(),
            projects::ProjectAction::Resume(id) => self.resume_project(id),
            projects::ProjectAction::OpenChat(slug) => self.open_project_chat(slug),
            projects::ProjectAction::CreateProject {
                name,
                slug,
                goal,
                source_type,
                local_path,
                github_full_name,
                branch,
            } => {
                if let Some(backend) = self.backend.to_ref() {
                    self.projects.loading = true;
                    event::spawn_launch_project(
                        backend,
                        name,
                        slug,
                        goal,
                        source_type,
                        local_path,
                        github_full_name,
                        branch,
                        self.event_tx.clone(),
                    );
                }
            }
            other => return Some(other),
        }
        None
    }

    fn handle_project_runtime_lifecycle_action(
        &mut self,
        action: projects::ProjectAction,
    ) -> Option<projects::ProjectAction> {
        match action {
            projects::ProjectAction::DeleteProject(id) => {
                self.spawn_project_simple_action(
                    "DELETE",
                    format!("/api/projects/{}", command_args::path_segment(&id)),
                    None,
                );
            }
            projects::ProjectAction::StartRuntime(id) => self.project_runtime_action(id, "start"),
            projects::ProjectAction::PauseRuntime(id) => self.project_runtime_action(id, "pause"),
            projects::ProjectAction::ResumeRuntime(id) => self.project_runtime_action(id, "resume"),
            projects::ProjectAction::TakeoverRuntime(id) => {
                self.project_runtime_action(id, "takeover")
            }
            projects::ProjectAction::SetLifecycle { id_or_slug, phase } => {
                self.spawn_project_simple_action(
                    "PATCH",
                    format!(
                        "/api/projects/{}/lifecycle",
                        command_args::path_segment(&id_or_slug)
                    ),
                    Some(serde_json::json!({ "phase": phase })),
                );
            }
            other => return Some(other),
        }
        None
    }

    fn handle_project_goal_action(&mut self, action: projects::ProjectAction) {
        match action {
            projects::ProjectAction::CreateGoal {
                project_id,
                name,
                check_command,
                interval_secs,
            } => {
                self.spawn_project_simple_action(
                    "POST",
                    format!(
                        "/api/projects/{}/goals",
                        command_args::path_segment(&project_id)
                    ),
                    Some(serde_json::json!({
                        "name": name,
                        "check_command": check_command,
                        "interval_secs": interval_secs,
                    })),
                );
            }
            projects::ProjectAction::PauseGoal {
                project_id,
                goal_id,
            } => self.project_goal_status_action(project_id, goal_id, "pause"),
            projects::ProjectAction::ResumeGoal {
                project_id,
                goal_id,
            } => self.project_goal_status_action(project_id, goal_id, "resume"),
            projects::ProjectAction::DeleteGoal {
                project_id,
                goal_id,
            } => {
                self.spawn_project_simple_action(
                    "DELETE",
                    format!(
                        "/api/projects/{}/goals/{}",
                        command_args::path_segment(&project_id),
                        command_args::path_segment(&goal_id)
                    ),
                    None,
                );
            }
            _ => {}
        }
    }

    fn resume_project(&mut self, id: String) {
        if let Some(backend) = self.backend.to_ref() {
            self.projects.loading = true;
            event::spawn_resume_project(backend, id, self.event_tx.clone());
        }
    }

    fn open_project_chat(&mut self, slug: String) {
        self.activate_project_for_chat(&slug);
        self.switch_tab(Tab::Chat);
        self.chat.push_message(
            chat::Role::System,
            format!("Project workspace active: {slug}"),
        );
    }

    fn spawn_project_simple_action(
        &mut self,
        method: &'static str,
        path: String,
        body: Option<serde_json::Value>,
    ) {
        if let Some(backend) = self.backend.to_ref() {
            self.projects.loading = true;
            event::spawn_project_simple_action(backend, method, path, body, self.event_tx.clone());
        }
    }

    fn project_goal_status_action(&mut self, project_id: String, goal_id: String, verb: &str) {
        self.spawn_project_simple_action(
            "POST",
            format!(
                "/api/projects/{}/goals/{}/{}",
                command_args::path_segment(&project_id),
                command_args::path_segment(&goal_id),
                verb
            ),
            None,
        );
    }

    fn project_runtime_action(&mut self, project_id: String, verb: &str) {
        self.spawn_project_simple_action(
            "POST",
            format!(
                "/api/projects/{}/runtime/{}",
                command_args::path_segment(&project_id),
                verb
            ),
            None,
        );
    }

    fn activate_project_for_chat(&mut self, slug: &str) {
        match (&self.backend, &self.chat_target) {
            (
                Backend::Daemon { base_url },
                Some(ChatTarget {
                    agent_id_daemon: Some(agent_id),
                    ..
                }),
            ) => {
                event::spawn_project_simple_action(
                    BackendRef::Daemon(base_url.clone()),
                    "PUT",
                    format!(
                        "/api/active-project/{}",
                        command_args::path_segment(agent_id)
                    ),
                    Some(serde_json::json!({ "slug": slug })),
                    self.event_tx.clone(),
                );
            }
            (
                Backend::InProcess { .. },
                Some(ChatTarget {
                    agent_id_inprocess: Some(agent_id),
                    ..
                }),
            ) => {
                if let Some(reg) = captain_runtime::active_project::global() {
                    reg.set(agent_id.to_string(), slug.to_string());
                }
            }
            _ => {
                self.projects.status_msg = "Open a Captain chat agent first.".to_string();
            }
        }
    }

    fn handle_trigger_action(&mut self, action: triggers::TriggerAction) {
        match action {
            triggers::TriggerAction::Continue => {}
            triggers::TriggerAction::Refresh => self.refresh_triggers(),
            triggers::TriggerAction::CreateTrigger {
                agent_id,
                pattern_type,
                pattern_param,
                prompt,
                max_fires,
            } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_create_trigger(
                        backend,
                        agent_id,
                        pattern_type,
                        pattern_param,
                        prompt,
                        max_fires,
                        self.event_tx.clone(),
                    );
                }
            }
            triggers::TriggerAction::DeleteTrigger(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_delete_trigger(backend, id, self.event_tx.clone());
                }
            }
            triggers::TriggerAction::ToggleTrigger { id, enabled } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_toggle_trigger(backend, id, enabled, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_sessions_action(&mut self, action: sessions::SessionsAction) {
        match action {
            sessions::SessionsAction::Continue => {}
            sessions::SessionsAction::Refresh => self.refresh_sessions(),
            sessions::SessionsAction::OpenInChat { session_id } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_load_session(backend, session_id, self.event_tx.clone());
                }
            }
            sessions::SessionsAction::DeleteSession(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_delete_session(backend, id, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_memory_action(&mut self, action: memory::MemoryAction) {
        match action {
            memory::MemoryAction::Continue => {}
            memory::MemoryAction::LoadAgents => self.refresh_memory(),
            memory::MemoryAction::LoadKv(agent_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    self.memory.loading = true;
                    event::spawn_fetch_memory_kv(backend, agent_id, self.event_tx.clone());
                }
            }
            memory::MemoryAction::SaveKv {
                agent_id,
                key,
                value,
            } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_save_memory_kv(
                        backend,
                        agent_id,
                        key,
                        value,
                        self.event_tx.clone(),
                    );
                }
            }
            memory::MemoryAction::DeleteKv { agent_id, key } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_delete_memory_kv(backend, agent_id, key, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_native_capabilities_action(
        &mut self,
        action: native_capabilities::NativeCapabilitiesAction,
    ) {
        use event_native_capabilities::NativeMutation;

        let Some(backend) = self.backend.to_ref() else {
            return;
        };
        let workspace = self.native_capability_workspace();
        match action {
            native_capabilities::NativeCapabilitiesAction::Continue => {}
            native_capabilities::NativeCapabilitiesAction::Refresh => {
                self.refresh_native_capabilities();
            }
            native_capabilities::NativeCapabilitiesAction::Inspect {
                name,
                scope,
                include_source,
            } => {
                self.native_capabilities.loading = true;
                event_native_capabilities::spawn_inspect(
                    backend,
                    name,
                    scope,
                    workspace,
                    include_source,
                    self.event_tx.clone(),
                );
            }
            native_capabilities::NativeCapabilitiesAction::Decide {
                name,
                scope,
                expected_hash,
                approve,
            } => {
                self.native_capabilities.loading = true;
                event_native_capabilities::spawn_mutation(
                    backend,
                    NativeMutation::Decide {
                        name,
                        scope,
                        expected_hash,
                        approve,
                    },
                    workspace,
                    self.event_tx.clone(),
                );
            }
            native_capabilities::NativeCapabilitiesAction::Rollback {
                name,
                scope,
                target_hash,
            } => {
                self.native_capabilities.loading = true;
                event_native_capabilities::spawn_mutation(
                    backend,
                    NativeMutation::Rollback {
                        name,
                        scope,
                        target_hash,
                    },
                    workspace,
                    self.event_tx.clone(),
                );
            }
            native_capabilities::NativeCapabilitiesAction::Disable { name, scope } => {
                self.native_capabilities.loading = true;
                event_native_capabilities::spawn_mutation(
                    backend,
                    NativeMutation::Disable { name, scope },
                    workspace,
                    self.event_tx.clone(),
                );
            }
            native_capabilities::NativeCapabilitiesAction::ResolveRun {
                run_id,
                node_id,
                tool_use_id,
                attempt,
                decision,
            } => {
                self.native_capabilities.loading = true;
                event_native_capabilities::spawn_mutation(
                    backend,
                    NativeMutation::ResolveRun {
                        run_id,
                        node_id,
                        tool_use_id,
                        attempt,
                        decision,
                    },
                    workspace,
                    self.event_tx.clone(),
                );
            }
        }
    }

    fn native_capability_workspace(&self) -> Option<String> {
        self.workspace
            .as_ref()
            .and_then(|workspace| workspace.config_path.parent().map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .and_then(|path| path.canonicalize().ok().or(Some(path)))
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn handle_skills_action(&mut self, action: skills::SkillsAction) {
        match action {
            skills::SkillsAction::Continue => {}
            skills::SkillsAction::RefreshInstalled => self.refresh_skills(),
            skills::SkillsAction::SearchClawHub(query) => {
                if let Some(backend) = self.backend.to_ref() {
                    self.skills.loading = true;
                    event::spawn_search_clawhub(backend, query, self.event_tx.clone());
                }
            }
            skills::SkillsAction::BrowseClawHub(sort) => {
                if let Some(backend) = self.backend.to_ref() {
                    self.skills.loading = true;
                    event::spawn_browse_clawhub(backend, sort, self.event_tx.clone());
                }
            }
            skills::SkillsAction::InstallSkill(slug) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_install_skill(backend, slug, self.event_tx.clone());
                }
            }
            skills::SkillsAction::UninstallSkill(name) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_uninstall_skill(backend, name, self.event_tx.clone());
                }
            }
            skills::SkillsAction::RefreshMcp => {
                if let Some(backend) = self.backend.to_ref() {
                    self.skills.loading = true;
                    event::spawn_fetch_mcp_servers(backend, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_extensions_action(&mut self, action: extensions::ExtensionsAction) {
        match action {
            extensions::ExtensionsAction::Continue => {}
            extensions::ExtensionsAction::RefreshAll => self.refresh_extensions(),
            extensions::ExtensionsAction::RefreshHealth => self.refresh_extension_health(),
            extensions::ExtensionsAction::Install(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_install_extension(backend, id, self.event_tx.clone());
                }
            }
            extensions::ExtensionsAction::Remove(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_remove_extension(backend, id, self.event_tx.clone());
                }
            }
            extensions::ExtensionsAction::Reconnect(id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_reconnect_extension(backend, id, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_hands_action(&mut self, action: hands::HandsAction) {
        match action {
            hands::HandsAction::Continue => {}
            hands::HandsAction::RefreshDefinitions => self.refresh_hands(),
            hands::HandsAction::RefreshActive => {
                if let Some(backend) = self.backend.to_ref() {
                    self.hands.loading = true;
                    event::spawn_fetch_active_hands(backend, self.event_tx.clone());
                }
            }
            hands::HandsAction::ActivateHand(hand_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_activate_hand(backend, hand_id, self.event_tx.clone());
                }
            }
            hands::HandsAction::DeactivateHand(instance_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_deactivate_hand(backend, instance_id, self.event_tx.clone());
                }
            }
            hands::HandsAction::PauseHand(instance_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_pause_hand(backend, instance_id, self.event_tx.clone());
                }
            }
            hands::HandsAction::ResumeHand(instance_id) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_resume_hand(backend, instance_id, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_templates_action(&mut self, action: templates::TemplatesAction) {
        match action {
            templates::TemplatesAction::Continue => {}
            templates::TemplatesAction::Refresh => self.refresh_templates(),
            templates::TemplatesAction::SpawnTemplate(name) => {
                // Find template and generate TOML manifest
                if let Some(t) = self.templates.templates.iter().find(|t| t.name == name) {
                    let toml_content = format!(
                        "name = \"{}\"\ndescription = \"{}\"\n\n[model]\nprovider = \"{}\"\nmodel = \"{}\"\n\n[capabilities]\ntools = [\"shell\", \"file_read\", \"file_write\", \"web_fetch\", \"web_search\"]\n",
                        t.name, t.description, t.provider, t.model,
                    );
                    self.spawn_agent(toml_content);
                }
            }
        }
    }

    fn handle_security_action(&mut self, action: security::SecurityAction) {
        match action {
            security::SecurityAction::Continue => {}
            security::SecurityAction::Refresh => self.refresh_security(),
            security::SecurityAction::VerifyChain => {
                if let Some(backend) = self.backend.to_ref() {
                    self.security.loading = true;
                    event::spawn_verify_chain(backend, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_audit_action(&mut self, action: audit::AuditAction) {
        match action {
            audit::AuditAction::Continue => {}
            audit::AuditAction::Refresh => self.refresh_audit(),
            audit::AuditAction::VerifyChain => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_verify_chain(backend, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_usage_action(&mut self, action: usage::UsageAction) {
        match action {
            usage::UsageAction::Continue => {}
            usage::UsageAction::Refresh => self.refresh_usage(),
        }
    }

    fn handle_settings_action(&mut self, action: settings::SettingsAction) {
        match action {
            settings::SettingsAction::Continue => {}
            settings::SettingsAction::RefreshProviders => self.refresh_settings_providers(),
            settings::SettingsAction::RefreshModels => self.refresh_settings_models(),
            settings::SettingsAction::RefreshTools => self.refresh_settings_tools(),
            settings::SettingsAction::SaveProviderKey { name, key } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_save_provider_key(backend, name, key, self.event_tx.clone());
                }
            }
            settings::SettingsAction::DeleteProviderKey(name) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_delete_provider_key(backend, name, self.event_tx.clone());
                }
            }
            settings::SettingsAction::TestProvider(name) => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_test_provider(backend, name, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_peers_action(&mut self, action: peers::PeersAction) {
        match action {
            peers::PeersAction::Continue => {}
            peers::PeersAction::Refresh => self.refresh_peers(),
        }
    }

    fn handle_comms_action(&mut self, action: comms::CommsAction) {
        match action {
            comms::CommsAction::Continue => {}
            comms::CommsAction::Refresh => self.refresh_comms(),
            comms::CommsAction::SendMessage { from, to, msg } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_comms_send(backend, from, to, msg, self.event_tx.clone());
                }
            }
            comms::CommsAction::PostTask {
                title,
                desc,
                assign,
            } => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_comms_task(backend, title, desc, assign, self.event_tx.clone());
                }
            }
        }
    }

    fn handle_logs_action(&mut self, action: logs::LogsAction) {
        match action {
            logs::LogsAction::Continue => {}
            logs::LogsAction::Refresh => self.refresh_logs(),
        }
    }

    // ─── Chat helpers ────────────────────────────────────────────────────────

    fn chat_session_prefix(&self) -> Option<String> {
        let target = self.chat_target.as_ref()?;
        if let Some(id) = &target.agent_id_daemon {
            Some(format!("daemon-{id}"))
        } else {
            target
                .agent_id_inprocess
                .map(|id| format!("inprocess-{}", id.0))
        }
    }

    fn usage_slash_snapshot(&self) -> UsageSlashSnapshot {
        UsageSlashSnapshot {
            session_input_tokens: self.chat.session_input_tokens,
            session_output_tokens: self.chat.session_output_tokens,
            session_cached_input_tokens: self.chat.session_cached_input_tokens,
            session_cache_creation_tokens: self.chat.session_cache_creation_tokens,
            last_tokens: self.chat.last_tokens,
            last_cached_input_tokens: self.chat.last_cached_input_tokens,
            session_cost_usd: self.chat.session_cost_usd,
            last_cost_usd: self.chat.last_cost_usd,
            message_count: self.chat.messages.len(),
        }
    }

    fn start_fresh_local_chat_session(&mut self) {
        let key = self.chat_session_prefix();
        self.chat.reset_preserving_chat_identity();
        if let Some(key) = key {
            let new_key = format!(
                "{key}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            );
            self.chat.start_session(&new_key);
        }
    }

    fn reset_current_backend_session(&mut self) -> Result<(), String> {
        if matches!(self.backend, Backend::None) {
            return Err(slash_session::reset_no_backend_connected_message().to_string());
        }
        let target = self
            .chat_target
            .as_mut()
            .ok_or_else(|| slash_session::reset_daemon_agent_missing_message().to_string())?;
        // The current scoped session remains durable. The next message lazily
        // creates another detached session, without mutating Telegram or any
        // other client currently using the agent's global session.
        target.session_id = None;
        Ok(())
    }

    fn enter_chat_daemon(&mut self, id: String, name: String) {
        let resume_matches =
            self.pending_resume_target
                .as_ref()
                .is_some_and(|(_, owner_id, owner_name)| {
                    resume_owner_matches(owner_id.as_deref(), owner_name, &id, &name)
                });
        let resumed_session_id = resume_matches
            .then(|| self.pending_resume_target.take().map(|target| target.0))
            .flatten();
        if resumed_session_id.is_some() {
            self.pending_restore_messages = None;
        }
        self.chat.reset();
        self.chat.agent_name = name.clone();
        self.chat.mode_label = "daemon".to_string();

        // Phase L.3 / #182: prefer rehydration if the user accepted a
        // resume at boot, otherwise start a fresh session bound to this
        // agent_id. Either branch must run BEFORE the welcome line so
        // the historical transcript shows up first.
        if resumed_session_id.is_some() || self.pending_resume_target.is_none() {
            if let Some((replay_key, replay_path)) = self.pending_chat_replay.take() {
                self.chat.replay_session_from(&replay_key, &replay_path);
            } else {
                let session_key = format!("daemon-{id}");
                self.chat.start_session(&session_key);
            }
        } else {
            let session_key = format!("daemon-{id}");
            self.chat.start_session(&session_key);
        }
        self.chat_target = Some(ChatTarget {
            agent_id_daemon: Some(id),
            agent_id_inprocess: None,
            agent_name: name,
            session_id: resumed_session_id.clone(),
        });
        if let (Some(target), Some(session_id)) =
            (self.chat_target.as_ref(), resumed_session_id.as_deref())
        {
            if let Some(agent_id) = target.agent_id_daemon.as_deref() {
                self.chat.bind_authoritative_session(agent_id, session_id);
            }
        }
        self.chat.mode_label = "daemon".to_string();
        if let Some(ref target) = self.chat_target {
            self.chat.agent_name = target.agent_name.clone();
        }
        self.refresh_active_chat_metadata();
        self.chat.push_message(
            chat::Role::System,
            slash_session::chat_session_help_message().to_string(),
        );
        self.active_tab = Tab::Chat;
        if let Some(ref t) = self.chat_target {
            let did = t.agent_id_daemon.clone();
            self.apply_pending_restore(did.as_deref(), None);
        }
    }

    fn enter_chat_inprocess(&mut self, id: AgentId, name: String) {
        let id_string = id.to_string();
        let resume_matches =
            self.pending_resume_target
                .as_ref()
                .is_some_and(|(_, owner_id, owner_name)| {
                    resume_owner_matches(owner_id.as_deref(), owner_name, &id_string, &name)
                });
        let resumed_session_id = resume_matches
            .then(|| self.pending_resume_target.take().map(|target| target.0))
            .flatten();
        if resumed_session_id.is_some() {
            self.pending_restore_messages = None;
        }
        self.chat.reset();
        self.chat.agent_name = name.clone();
        self.chat.mode_label = "in-process".to_string();

        // Phase L.3 / #182: prefer rehydration if the user accepted a
        // resume at boot, otherwise start a fresh session bound to this
        // agent_id. Either branch must run BEFORE the welcome line so
        // the historical transcript shows up first.
        if resumed_session_id.is_some() || self.pending_resume_target.is_none() {
            if let Some((replay_key, replay_path)) = self.pending_chat_replay.take() {
                self.chat.replay_session_from(&replay_key, &replay_path);
            } else {
                let session_key = format!("inprocess-{}", id.0);
                self.chat.start_session(&session_key);
            }
        } else {
            let session_key = format!("inprocess-{}", id.0);
            self.chat.start_session(&session_key);
        }
        self.chat_target = Some(ChatTarget {
            agent_id_daemon: None,
            agent_id_inprocess: Some(id),
            agent_name: name,
            session_id: resumed_session_id.clone(),
        });
        if let Some(session_id) = resumed_session_id.as_deref() {
            self.chat
                .bind_authoritative_session(&id.to_string(), session_id);
        }
        self.chat.mode_label = "in-process".to_string();
        if let Some(ref target) = self.chat_target {
            self.chat.agent_name = target.agent_name.clone();
        }
        self.refresh_active_chat_metadata();
        self.chat.push_message(
            chat::Role::System,
            slash_session::chat_session_help_message().to_string(),
        );
        self.active_tab = Tab::Chat;
        self.apply_pending_restore(None, Some(id));
    }

    /// Phase-i.8: handle the /image slash command. Reads the file from disk,
    /// detects content-type from extension, POSTs sync to /api/agents/{id}/upload
    /// to obtain a file_id, and stages it for the next outgoing message.
    fn handle_image_attach(&mut self, raw_path: &str) {
        let file_upload::PreparedUpload {
            path,
            filename,
            content_type,
            bytes,
        } = match file_upload::prepare_upload(raw_path) {
            Ok(upload) => upload,
            Err(message) => {
                self.chat.push_message(chat::Role::System, message);
                return;
            }
        };

        let (base_url, agent_id) = match self.attachment_upload_target() {
            Ok(target) => target,
            Err(message) => {
                self.chat
                    .push_message(chat::Role::System, message.to_string());
                return;
            }
        };

        let resp =
            Self::send_attachment_upload(&base_url, &agent_id, &filename, content_type, bytes);
        self.handle_attachment_upload_response(resp, path, filename, content_type);
    }

    fn attachment_upload_target(&self) -> Result<(String, String), &'static str> {
        match (&self.backend, &self.chat_target) {
            (Backend::Daemon { base_url }, Some(target)) => target
                .agent_id_daemon
                .as_ref()
                .map(|agent_id| (base_url.clone(), agent_id.clone()))
                .ok_or_else(slash_attachment::upload_requires_daemon_message),
            _ => Err(slash_attachment::upload_requires_daemon_message()),
        }
    }

    fn send_attachment_upload(
        base_url: &str,
        agent_id: &str,
        filename: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<reqwest::blocking::Response, reqwest::Error> {
        crate::daemon_client()
            .post(format!("{base_url}/api/agents/{agent_id}/upload"))
            .header("Content-Type", content_type)
            .header("X-Filename", filename)
            .body(bytes)
            .send()
    }

    fn handle_attachment_upload_response(
        &mut self,
        resp: Result<reqwest::blocking::Response, reqwest::Error>,
        path: PathBuf,
        filename: String,
        content_type: &str,
    ) {
        match resp {
            Ok(r) if r.status().is_success() => {
                let file_id = r
                    .json::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v["file_id"].as_str().map(String::from));
                match file_id {
                    Some(id) => {
                        self.chat.pending_attachments.push(chat::PendingAttachment {
                            file_id: id.clone(),
                            filename: filename.clone(),
                            content_type: content_type.to_string(),
                            local_path: Some(path.clone()),
                        });
                        self.chat.push_message(
                            chat::Role::System,
                            slash_attachment::upload_staged_message(&filename, &id),
                        );
                    }
                    None => self.chat.push_message(
                        chat::Role::System,
                        slash_attachment::upload_missing_file_id_message().to_string(),
                    ),
                }
            }
            Ok(r) => self.chat.push_message(
                chat::Role::System,
                slash_attachment::upload_http_error_message(r.status()),
            ),
            Err(e) => self.chat.push_message(
                chat::Role::System,
                slash_attachment::upload_error_message(e),
            ),
        }
    }

    /// Phase-j.2: POST /api/agents/{id}/feedback with thumbs up/down.
    /// `value` is "up" or "down". Optional `note` becomes a free-text comment.
    fn handle_feedback(&mut self, value: &str, note: &str) {
        let (base_url, agent_id) = match (&self.backend, &self.chat_target) {
            (Backend::Daemon { base_url }, Some(t)) if t.agent_id_daemon.is_some() => {
                (base_url.clone(), t.agent_id_daemon.clone().unwrap())
            }
            _ => {
                self.chat.push_message(
                    chat::Role::System,
                    slash_feedback::feedback_requires_daemon_message().to_string(),
                );
                return;
            }
        };
        let last_agent = self
            .chat
            .messages
            .iter()
            .rev()
            .find(|m| m.role == chat::Role::Agent)
            .map(|m| slash_feedback::response_preview(&m.text))
            .unwrap_or_default();
        let timestamp_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let body = slash_feedback::feedback_payload(value, note, &last_agent, timestamp_secs);
        let client = crate::daemon_client();
        let resp = client
            .post(format!("{base_url}/api/agents/{agent_id}/feedback"))
            .json(&body)
            .send();
        match resp {
            Ok(r) if r.status().is_success() => {
                self.chat.push_message(
                    chat::Role::System,
                    slash_feedback::feedback_saved_message(value),
                );
            }
            Ok(r) => self.chat.push_message(
                chat::Role::System,
                slash_feedback::feedback_http_error_message(r.status()),
            ),
            Err(e) => self.chat.push_message(
                chat::Role::System,
                slash_feedback::feedback_error_message(e),
            ),
        }
    }

    fn send_message(&mut self, message: String) {
        let session_id = match self.ensure_authoritative_session() {
            Ok(session_id) => session_id,
            Err(error) => {
                self.chat.status_msg = Some(format!("Session creation failed: {error}"));
                return;
            }
        };
        stream_lifecycle::prepare_stream_start(&mut self.chat);

        // Phase-i.8: drain any pending attachments queued by /image so they
        // ride along with this message.
        let attachments = std::mem::take(&mut self.chat.pending_attachments);

        match (&self.backend, &self.chat_target) {
            (Backend::Daemon { base_url }, Some(target)) if target.agent_id_daemon.is_some() => {
                event::spawn_daemon_stream(
                    base_url.clone(),
                    target.agent_id_daemon.as_ref().unwrap().clone(),
                    Some(session_id.clone()),
                    message,
                    attachments,
                    self.event_tx.clone(),
                );
            }
            (Backend::InProcess { kernel }, Some(target))
                if target.agent_id_inprocess.is_some() =>
            {
                if !attachments.is_empty() {
                    self.chat.push_message(
                        chat::Role::System,
                        slash_attachment::attachments_ignored_without_daemon_message().to_string(),
                    );
                }
                event::spawn_inprocess_stream(
                    kernel.clone(),
                    target.agent_id_inprocess.unwrap(),
                    session_id
                        .parse::<uuid::Uuid>()
                        .ok()
                        .map(captain_types::agent::SessionId),
                    message,
                    self.event_tx.clone(),
                );
            }
            _ => {
                self.chat.is_streaming = false;
                let msg = match (&self.backend, &self.chat_target) {
                    (Backend::None, _) => {
                        "Backend non démarré. Quitte (q) puis relance `captain` — ou `captain start` dans un autre terminal.".to_string()
                    }
                    (_, None) => {
                        "Aucun agent sélectionné. Onglet Agents (Tab) → choisis un agent, ou crée-en un.".to_string()
                    }
                    (Backend::Daemon { .. }, Some(t)) if t.agent_id_daemon.is_none() => {
                        format!(
                            "Agent '{}' enregistré en local mais le backend est le daemon. Onglet Agents → sélectionne un agent du daemon.",
                            t.agent_name
                        )
                    }
                    (Backend::InProcess { .. }, Some(t)) if t.agent_id_inprocess.is_none() => {
                        format!(
                            "Agent '{}' lié au daemon mais le backend est in-process. Onglet Agents → re-sélectionne.",
                            t.agent_name
                        )
                    }
                    _ => "Configuration agent invalide. Onglet Agents → re-sélectionne.".to_string(),
                };
                self.chat.status_msg = Some(msg);
            }
        }
    }

    fn ensure_authoritative_session(&mut self) -> Result<String, String> {
        let target = self
            .chat_target
            .as_ref()
            .ok_or_else(|| "No agent selected".to_string())?;
        if let Some(session_id) = target.session_id.as_ref() {
            return Ok(session_id.clone());
        }

        let (agent_id, session_id) = match (&self.backend, target) {
            (Backend::Daemon { base_url }, target) => {
                let agent_id = target
                    .agent_id_daemon
                    .as_ref()
                    .ok_or_else(|| "No daemon agent selected".to_string())?
                    .clone();
                let response = crate::daemon_client()
                    .post(format!("{base_url}/api/agents/{agent_id}/sessions"))
                    .json(&serde_json::json!({"activate": false}))
                    .send()
                    .map_err(|error| error.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("daemon returned HTTP {}", response.status()));
                }
                let body = response
                    .json::<serde_json::Value>()
                    .map_err(|error| error.to_string())?;
                let session_id = body
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| uuid::Uuid::parse_str(value).is_ok())
                    .ok_or_else(|| "daemon returned no valid session ID".to_string())?
                    .to_string();
                (agent_id, session_id)
            }
            (Backend::InProcess { kernel }, target) => {
                let agent_id = target
                    .agent_id_inprocess
                    .ok_or_else(|| "No in-process agent selected".to_string())?;
                let created = kernel
                    .create_agent_session_detached(agent_id, None)
                    .map_err(|error| error.to_string())?;
                let session_id = created
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| uuid::Uuid::parse_str(value).is_ok())
                    .ok_or_else(|| "kernel returned no valid session ID".to_string())?
                    .to_string();
                (agent_id.to_string(), session_id)
            }
            (Backend::None, _) => return Err("No backend connected".to_string()),
        };

        if let Some(target) = self.chat_target.as_mut() {
            target.session_id = Some(session_id.clone());
        }
        self.chat.bind_authoritative_session(&agent_id, &session_id);
        Ok(session_id)
    }

    fn forward_daemon_slash_command(&mut self, command: &str) {
        if matches!(self.backend, Backend::Daemon { .. }) {
            self.send_message(command.to_string());
        } else {
            self.chat.push_message(
                chat::Role::System,
                slash_daemon::unavailable_message(crate::i18n::Lang::Fr).to_string(),
            );
        }
    }

    fn spawn_agent(&mut self, toml_content: String) {
        match &self.backend {
            Backend::Daemon { base_url } => {
                self.agents.sub = agents::AgentSubScreen::Spawning;
                event::spawn_daemon_agent(base_url.clone(), toml_content, self.event_tx.clone());
            }
            Backend::InProcess { kernel } => {
                let manifest: captain_types::agent::AgentManifest =
                    match toml::from_str(&toml_content) {
                        Ok(m) => m,
                        Err(e) => {
                            self.agents.status_msg = agent_status::invalid_manifest_message(e);
                            self.agents.sub = agents::AgentSubScreen::AgentList;
                            return;
                        }
                    };
                let name = manifest.name.clone();
                match kernel.spawn_agent(manifest) {
                    Ok(id) => self.enter_chat_inprocess(id, name),
                    Err(e) => {
                        self.agents.status_msg = agent_status::spawn_failed_message(e);
                        self.agents.sub = agents::AgentSubScreen::AgentList;
                    }
                }
            }
            Backend::None => {
                self.agents.status_msg = agent_status::no_backend_connected_message().to_string();
                self.agents.sub = agents::AgentSubScreen::AgentList;
            }
        }
    }

    // ─── Model picker ────────────────────────────────────────────────────────

    fn open_model_picker(&mut self) {
        let models = match &self.backend {
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                match client.get(format!("{base_url}/api/models")).send() {
                    Ok(resp) => match resp.json::<serde_json::Value>() {
                        Ok(body) => body["models"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter(|m| m["available"].as_bool().unwrap_or(false))
                                    .map(|m| chat::ModelEntry {
                                        id: m["id"].as_str().unwrap_or("").to_string(),
                                        display_name: m["display_name"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        provider: m["provider"].as_str().unwrap_or("").to_string(),
                                        tier: m["tier"].as_str().unwrap_or("Balanced").to_string(),
                                        context_window: m["context_window"]
                                            .as_u64()
                                            .unwrap_or_default(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        Err(_) => Vec::new(),
                    },
                    Err(_) => Vec::new(),
                }
            }
            Backend::InProcess { kernel } => {
                let catalog = kernel.model_catalog.read().unwrap();
                catalog
                    .available_models()
                    .into_iter()
                    .map(|e| chat::ModelEntry {
                        id: e.id.clone(),
                        display_name: e.display_name.clone(),
                        provider: e.provider.clone(),
                        tier: format!("{:?}", e.tier),
                        context_window: e.context_window,
                    })
                    .collect()
            }
            Backend::None => Vec::new(),
        };

        if models.is_empty() {
            self.chat
                .push_message(chat::Role::System, "No models available.".to_string());
            return;
        }

        self.chat.model_picker_models = models;
        self.chat.model_picker_filter.clear();
        self.chat.model_picker_idx = 0;
        self.chat.show_model_picker = true;
    }

    fn switch_model(&mut self, model_id: &str, session_strategy: Option<&str>) {
        if self.chat.model_label.ends_with(model_id) {
            return;
        }

        enum ModelSwitchRoute {
            Daemon {
                base_url: String,
                agent_id: String,
            },
            InProcess {
                kernel: Arc<CaptainKernel>,
                agent_id: AgentId,
            },
            Unavailable,
            Ignore,
        }

        let route = match (&self.backend, &self.chat_target) {
            (Backend::Daemon { base_url }, Some(target)) => target
                .agent_id_daemon
                .clone()
                .map(|agent_id| ModelSwitchRoute::Daemon {
                    base_url: base_url.clone(),
                    agent_id,
                })
                .unwrap_or(ModelSwitchRoute::Ignore),
            (Backend::InProcess { kernel }, Some(target)) => target
                .agent_id_inprocess
                .map(|agent_id| ModelSwitchRoute::InProcess {
                    kernel: Arc::clone(kernel),
                    agent_id,
                })
                .unwrap_or(ModelSwitchRoute::Ignore),
            _ => ModelSwitchRoute::Unavailable,
        };

        match route {
            ModelSwitchRoute::Daemon { base_url, agent_id } => {
                self.switch_daemon_model(&base_url, &agent_id, model_id, session_strategy);
            }
            ModelSwitchRoute::InProcess { kernel, agent_id } => {
                self.switch_inprocess_model(&kernel, agent_id, model_id, session_strategy);
            }
            ModelSwitchRoute::Unavailable => self.chat.push_message(
                chat::Role::System,
                slash_model::no_backend_connected_message().to_string(),
            ),
            ModelSwitchRoute::Ignore => {}
        }
    }

    fn enter_startup_phase(&mut self) {
        let needs_setup = wizard::needs_setup();
        let latest_resume = if needs_setup {
            None
        } else {
            // Sessions are sorted by mtime desc; skip empty sessions because
            // there is nothing useful to resume from a stillborn chat.
            session_store::list_sessions()
                .into_iter()
                .find(|s| s.message_count > 0)
        };

        self.phase = startup_phase_for_state(needs_setup, latest_resume.is_some());
        match self.phase {
            Phase::Boot(BootScreen::Wizard) => self.wizard.reset(),
            Phase::Boot(BootScreen::ResumePrompt) => {
                self.pending_resume = latest_resume;
                self.start_daemon_detect();
            }
            Phase::Boot(BootScreen::Welcome) => {
                self.start_daemon_detect();
            }
            Phase::Main => {}
        }
    }

    fn switch_daemon_model(
        &mut self,
        base_url: &str,
        agent_id: &str,
        model_id: &str,
        session_strategy: Option<&str>,
    ) {
        let client = crate::daemon_client();
        let plan = match Self::load_daemon_model_switch_plan(&client, base_url, agent_id, model_id)
        {
            Ok(plan) => plan,
            Err(message) => {
                self.chat.push_message(chat::Role::System, message);
                return;
            }
        };
        if !self.accept_daemon_model_switch_plan(&plan) {
            return;
        }
        let strategy =
            match self.resolve_daemon_model_switch_strategy(model_id, session_strategy, &plan) {
                Some(strategy) => strategy,
                None => return,
            };
        self.apply_daemon_model_switch(&client, base_url, agent_id, model_id, &strategy);
    }

    fn load_daemon_model_switch_plan(
        client: &reqwest::blocking::Client,
        base_url: &str,
        agent_id: &str,
        model_id: &str,
    ) -> Result<serde_json::Value, String> {
        let plan_url = format!("{base_url}/api/agents/{agent_id}/model-switch/plan");
        match client
            .post(&plan_url)
            .json(&serde_json::json!({"model": model_id}))
            .send()
        {
            Ok(r) if r.status().is_success() => r
                .json::<serde_json::Value>()
                .map_err(slash_model::daemon_preflight_parse_failed_message),
            Ok(r) => Err(slash_model::daemon_preflight_http_failed_message(
                r.status(),
            )),
            Err(e) => Err(slash_model::daemon_preflight_error_message(e)),
        }
    }

    fn accept_daemon_model_switch_plan(&mut self, plan: &serde_json::Value) -> bool {
        if plan["can_apply"].as_bool().unwrap_or(false) {
            return true;
        }
        let issues = slash_model::daemon_blocking_issues(plan);
        self.chat.push_message(
            chat::Role::System,
            slash_model::model_switch_blocked_message(&issues),
        );
        false
    }

    fn resolve_daemon_model_switch_strategy(
        &mut self,
        model_id: &str,
        session_strategy: Option<&str>,
        plan: &serde_json::Value,
    ) -> Option<String> {
        match slash_model::daemon_model_switch_decision(model_id, session_strategy, plan) {
            slash_model::DaemonModelSwitchDecision::Apply(strategy) => Some(strategy),
            slash_model::DaemonModelSwitchDecision::RequestChoice(prompt) => {
                self.chat.request_model_switch_choice(prompt);
                None
            }
        }
    }

    fn apply_daemon_model_switch(
        &mut self,
        client: &reqwest::blocking::Client,
        base_url: &str,
        agent_id: &str,
        model_id: &str,
        strategy: &str,
    ) {
        let apply_url = format!("{base_url}/api/agents/{agent_id}/model-switch/apply");
        match client
            .post(&apply_url)
            .json(&serde_json::json!({
                "model": model_id,
                "session_strategy": strategy,
            }))
            .send()
        {
            Ok(r) if r.status().is_success() => {
                if let Ok(body) = r.json::<serde_json::Value>() {
                    let (label, message) = slash_model::daemon_apply_success(&body, model_id);
                    if let Some(label) = label {
                        self.chat.model_label = label;
                    }
                    self.chat.apply_model_context_window(model_id);
                    self.chat.push_message(chat::Role::System, message);
                    self.refresh_active_chat_metadata();
                }
            }
            Ok(r) => self.chat.push_message(
                chat::Role::System,
                slash_model::safe_switch_http_failed_message(r.status()),
            ),
            Err(e) => self.chat.push_message(
                chat::Role::System,
                slash_model::safe_switch_error_message(e),
            ),
        }
    }

    fn switch_inprocess_model(
        &mut self,
        kernel: &Arc<CaptainKernel>,
        agent_id: AgentId,
        model_id: &str,
        session_strategy: Option<&str>,
    ) {
        let plan = match kernel.plan_model_switch(agent_id, model_id, None) {
            Ok(plan) => plan,
            Err(e) => {
                self.chat.push_message(
                    chat::Role::System,
                    slash_model::inprocess_preflight_failed_message(e),
                );
                return;
            }
        };
        if !plan.can_apply {
            self.chat.push_message(
                chat::Role::System,
                slash_model::model_switch_blocked_message(&plan.blocking_issues.join("\n")),
            );
            return;
        }
        let strategy =
            match slash_model::inprocess_model_switch_decision(model_id, session_strategy, &plan) {
                slash_model::InProcessModelSwitchDecision::Apply(strategy) => strategy,
                slash_model::InProcessModelSwitchDecision::RequestChoice(prompt) => {
                    self.chat.request_model_switch_choice(prompt);
                    return;
                }
            };
        match kernel.apply_model_switch(agent_id, model_id, None, strategy) {
            Ok(result) => {
                self.chat.model_label = format!(
                    "{}/{}",
                    result.plan.target_provider, result.plan.target_model
                );
                self.chat.apply_model_context_window(model_id);
                self.chat.push_message(chat::Role::System, result.message);
                self.refresh_active_chat_metadata();
            }
            Err(e) => self
                .chat
                .push_message(chat::Role::System, slash_model::switch_failed_message(e)),
        }
    }

    // ─── Slash commands ──────────────────────────────────────────────────────

    fn handle_slash_command(&mut self, cmd: &str) {
        let (raw_command, args) = slash_command::split_slash_command(cmd);
        let command = raw_command.to_ascii_lowercase();
        let canonical_command = slash_command::canonical_slash_command(&raw_command, args);
        let hub_route = hub_slash_route_for_command(command.as_str());
        let surface_route = surface_slash_route_for_command(command.as_str());
        if self.handle_slash_prelude(&command, &canonical_command) {
            return;
        }
        if self.handle_navigation_slash_route(&command, args, surface_route, hub_route) {
            return;
        }
        if self.handle_session_slash_command(&command, args, &canonical_command) {
            return;
        }
        if self.handle_utility_slash_command(&command, args) {
            return;
        }
        self.handle_agent_status_slash_command(&command, args);
    }

    fn handle_session_slash_command(
        &mut self,
        command: &str,
        args: &str,
        canonical_command: &str,
    ) -> bool {
        match command {
            // Nouvelle session réelle: reset backend d'abord, puis état local TUI.
            "/new" => {
                self.handle_new_slash();
                true
            }
            "/resume" => {
                if let Some(backend) = self.backend.to_ref() {
                    event::spawn_resolve_session(backend, args.to_string(), self.event_tx.clone());
                }
                true
            }
            // Phase N.1 — recharge la dernière session sauvegardée pour cet agent.
            "/reload" => match slash_reload::reload_for(args) {
                slash_reload::SlashReload::ForwardDaemon => {
                    self.forward_daemon_slash_command(canonical_command);
                    true
                }
                slash_reload::SlashReload::ReloadSession => {
                    self.handle_reload_slash();
                    true
                }
            },
            // Phase L.4 — ouvre l'overlay session picker (alias Ctrl+O).
            "/history" => {
                self.handle_history_slash();
                true
            }
            "/sessions" | "/tasks" => {
                self.handle_sessions_slash();
                true
            }
            "/retry" => {
                self.handle_retry_slash();
                true
            }
            "/undo" => {
                self.handle_undo_slash();
                true
            }
            "/queue" => {
                self.handle_queue_slash();
                true
            }
            "/clear" => {
                self.handle_clear_slash();
                true
            }
            _ => false,
        }
    }

    fn handle_utility_slash_command(&mut self, command: &str, args: &str) -> bool {
        match command {
            // Phase N.1 — copie la dernière réponse agent dans le clipboard.
            "/copy" => {
                self.handle_copy_slash(args);
                true
            }
            "/mouse" => {
                self.handle_mouse_slash(args);
                true
            }
            // Phase N.1 — détail tokens session courante.
            "/tokens" => {
                self.handle_tokens_slash();
                true
            }
            // Phase N.1 — coût session + budget restant si configuré.
            "/cost" => {
                self.handle_cost_slash();
                true
            }
            // Phase L.4 — exporte la conversation courante en markdown.
            "/export" => {
                self.handle_export_slash();
                true
            }
            "/help" => {
                self.handle_help_slash();
                true
            }
            "/image" | "/file" => {
                self.handle_attachment_slash(command, args);
                true
            }
            "/like" | "/dislike" => {
                self.handle_feedback_slash(command, args);
                true
            }
            "/voice" => {
                self.handle_voice_slash(args);
                true
            }
            "/fortune" => {
                self.handle_fortune_slash();
                true
            }
            _ => false,
        }
    }

    fn handle_agent_status_slash_command(&mut self, command: &str, args: &str) {
        match command {
            "/status" => {
                self.handle_status_slash();
            }
            "/agents" => {
                self.handle_agents_slash();
            }
            "/kill" => {
                self.handle_kill_slash();
            }
            "/model" => {
                self.handle_model_slash(args);
            }
            _ => {
                let lang = crate::i18n::current();
                self.chat.push_message(
                    chat::Role::System,
                    format!("{} ({})", crate::i18n::t("chat.unknown_cmd", lang), command),
                );
            }
        }
    }

    fn handle_status_slash(&mut self) {
        let lang = crate::i18n::current();
        let agent_name = self
            .chat_target
            .as_ref()
            .map(|target| target.agent_name.as_str());
        let snapshot = match &self.backend {
            Backend::Daemon { base_url } => slash_info::StatusSnapshot::Daemon {
                base_url,
                agent_name,
            },
            Backend::InProcess { kernel } => slash_info::StatusSnapshot::InProcess {
                agent_count: kernel.registry.count(),
                agent_name,
            },
            Backend::None => slash_info::StatusSnapshot::Disconnected,
        };
        self.chat.push_message(
            chat::Role::System,
            slash_info::status_message(snapshot, lang),
        );
    }

    fn handle_slash_prelude(&mut self, command: &str, canonical_command: &str) -> bool {
        if slash_exit::is_exit_command(command) {
            self.handle_chat_action(chat::ChatAction::Back);
            return true;
        }
        if let Some(scroll) = slash_scroll::scroll_for(command) {
            match scroll {
                slash_scroll::SlashScroll::Top => self.chat.scroll_to_top(),
                slash_scroll::SlashScroll::Bottom => self.chat.scroll_to_bottom(),
            }
            return true;
        }
        if slash_think::is_think_command(command) {
            self.chat.toggle_thinking();
            return true;
        }
        if slash_daemon::is_daemon_forward_command(command) {
            self.forward_daemon_slash_command(canonical_command);
            return true;
        }
        false
    }

    fn handle_navigation_slash_route(
        &mut self,
        command: &str,
        args: &str,
        surface_route: Option<SurfaceSlashRoute>,
        hub_route: Option<HubSlashRoute>,
    ) -> bool {
        if let Some(route) = surface_route {
            self.apply_surface_slash_route(route);
            return true;
        }
        if command == "/project" {
            self.handle_project_slash(args);
            return true;
        }
        if let Some(route) = hub_route {
            self.open_hub_slash_route(route);
            return true;
        }
        false
    }

    fn handle_new_slash(&mut self) {
        match self.reset_current_backend_session() {
            Ok(()) => {
                self.start_fresh_local_chat_session();
                self.chat.push_message(
                    chat::Role::System,
                    slash_session::new_session_started_message(crate::i18n::Lang::Fr).to_string(),
                );
            }
            Err(e) => {
                self.chat.push_message(
                    chat::Role::System,
                    slash_session::reset_session_failed_message(crate::i18n::Lang::En, e),
                );
            }
        }
    }

    fn handle_tokens_slash(&mut self) {
        let msg = token_usage_message(
            self.usage_slash_snapshot(),
            UsageSlashSurface::FullTui,
            crate::i18n::Lang::Fr,
        );
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_cost_slash(&mut self) {
        let msg = cost_usage_message(
            self.usage_slash_snapshot(),
            UsageSlashSurface::FullTui,
            crate::i18n::Lang::Fr,
        );
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_export_slash(&mut self) {
        let msg = match self.chat.export_markdown() {
            Ok(path) => slash_export::export_success_message(crate::i18n::Lang::Fr, &path),
            Err(e) => slash_export::export_failed_message(
                crate::i18n::Lang::Fr,
                slash_export::ExportSurface::FullTui,
                e,
            ),
        };
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_history_slash(&mut self) {
        self.chat.open_session_picker();
        if let Some(backend) = self.backend.to_ref() {
            event::spawn_fetch_sessions(backend, self.event_tx.clone());
        }
    }

    fn handle_help_slash(&mut self) {
        let lang = crate::i18n::current();
        self.chat.push_message(
            chat::Role::System,
            slash_help::full_tui_help(lang).to_string(),
        );
    }

    fn handle_retry_slash(&mut self) {
        match slash_retry::last_user_message(&self.chat.messages) {
            Some(msg) => self.send_message(msg),
            None => {
                let lang = crate::i18n::current();
                self.chat.push_message(
                    chat::Role::System,
                    slash_retry::retry_nothing_message(lang).to_string(),
                );
            }
        }
    }

    fn handle_undo_slash(&mut self) {
        let dropped_user = slash_local::undo_last_exchange(&mut self.chat);
        let lang = crate::i18n::current();
        self.chat.push_message(
            chat::Role::System,
            slash_local::undo_result_message(dropped_user, lang).to_string(),
        );
    }

    fn handle_queue_slash(&mut self) {
        let lang = crate::i18n::current();
        let msg = slash_local::queue_message_for_lang(&self.chat.staged_messages, lang);
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_voice_slash(&mut self, args: &str) {
        let secs = slash_local::voice_record_secs(args);
        self.chat.push_message(
            chat::Role::System,
            slash_local::voice_recording_message(secs),
        );
        event::spawn_record_voice(secs, self.event_tx.clone());
    }

    fn handle_fortune_slash(&mut self) {
        let lang = crate::i18n::current();
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.chat.push_message(
            chat::Role::System,
            slash_fortune::fortune_message_for_timestamp_secs(secs, lang).to_string(),
        );
    }

    fn handle_clear_slash(&mut self) {
        slash_local::clear_chat_preserving_identity(&mut self.chat);
        let lang = crate::i18n::current();
        self.chat.push_message(
            chat::Role::System,
            slash_local::clear_message(lang).to_string(),
        );
    }

    fn handle_attachment_slash(&mut self, command: &str, args: &str) {
        match slash_attachment::attachment_for(command, args) {
            Some(slash_attachment::SlashAttachment::OpenPicker(kind)) => {
                match screens::file_picker::FilePickerState::open(kind) {
                    Ok(picker) => self.file_picker = Some(picker),
                    Err(e) => self.chat.push_message(
                        chat::Role::System,
                        slash_attachment::picker_open_error_message(e),
                    ),
                }
            }
            Some(slash_attachment::SlashAttachment::AttachPath(path)) => {
                self.handle_image_attach(path);
            }
            None => {}
        }
    }

    fn handle_project_slash(&mut self, args: &str) {
        match slash_project::project_slash_action(args) {
            slash_project::ProjectSlashAction::OpenProjects => self.switch_tab(Tab::Projects),
            slash_project::ProjectSlashAction::Activate(slug) => {
                self.activate_project_for_chat(slug);
                self.chat.push_message(
                    chat::Role::System,
                    slash_project::project_workspace_active_message(slug),
                );
            }
        }
    }

    fn handle_feedback_slash(&mut self, command: &str, args: &str) {
        if let Some(feedback) = slash_feedback::feedback_for(command, args) {
            self.handle_feedback(feedback.value, feedback.note);
        }
    }

    fn handle_model_slash(&mut self, args: &str) {
        if args.is_empty() {
            self.open_model_picker();
        } else {
            let (model, strategy) = command_args::parse_model_switch_args(args);
            self.switch_model(model, strategy);
        }
    }

    fn handle_reload_slash(&mut self) {
        let key = self.chat.session_key.clone();
        if key.is_empty() {
            self.chat.push_message(
                chat::Role::System,
                slash_reload::no_active_session_message(crate::i18n::Lang::Fr).to_string(),
            );
            return;
        }

        use crate::tui::session_store as store;
        if let Some((_path, loaded)) = store::load_latest_session(&key) {
            if let (Some(session_id), Some(backend)) = (loaded.session_id, self.backend.to_ref()) {
                event::spawn_load_session(backend, session_id, self.event_tx.clone());
            } else {
                self.chat.push_message(
                    chat::Role::System,
                    slash_reload::no_saved_session_message(crate::i18n::Lang::Fr).to_string(),
                );
            }
        } else {
            self.chat.push_message(
                chat::Role::System,
                slash_reload::no_saved_session_message(crate::i18n::Lang::Fr).to_string(),
            );
        }
    }

    fn handle_mouse_slash(&mut self, args: &str) {
        let msg = match input_state::mouse_capture_after_slash_arg(self.mouse_capture_enabled, args)
        {
            Some(enabled) => match event::set_mouse_capture(enabled) {
                Ok(()) => {
                    self.mouse_capture_enabled = enabled;
                    if enabled {
                        slash_local::mouse_enabled_message(
                            crate::i18n::Lang::Fr,
                            slash_local::MouseMessageSurface::FullTui,
                        )
                        .to_string()
                    } else {
                        slash_local::mouse_disabled_message(crate::i18n::Lang::Fr).to_string()
                    }
                }
                Err(e) => slash_local::mouse_error_message(
                    crate::i18n::Lang::Fr,
                    slash_local::MouseMessageSurface::FullTui,
                    e,
                ),
            },
            None => slash_local::mouse_usage_message(
                crate::i18n::Lang::Fr,
                slash_local::MouseMessageSurface::FullTui,
            )
            .to_string(),
        };
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_sessions_slash(&mut self) {
        let lang = crate::i18n::current();
        let mut lines: Vec<String> = Vec::new();
        match &self.backend {
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                if let Ok(resp) = client.get(format!("{base_url}/api/sessions")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        lines.extend(slash_info::daemon_session_lines(&body, usize::MAX));
                    }
                }
            }
            Backend::InProcess { kernel } => {
                if let Ok(sessions) = kernel.memory.list_sessions() {
                    lines.extend(slash_info::daemon_session_lines(
                        &serde_json::json!({"sessions": sessions}),
                        usize::MAX,
                    ));
                }
            }
            Backend::None => {
                lines.push(slash_info::sessions_not_connected_message(lang).to_string());
            }
        }
        let msg = slash_info::sessions_list_message(lines, lang);
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_agents_slash(&mut self) {
        let lang = crate::i18n::current();
        let mut lines = Vec::new();
        match &self.backend {
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        lines.extend(slash_info::daemon_agent_lines(&body));
                    }
                }
            }
            Backend::InProcess { kernel } => {
                for e in kernel.registry.list() {
                    lines.push(slash_info::inprocess_agent_line(
                        &e.name,
                        &format!("{:?}", e.state),
                        &e.manifest.model.provider,
                        &e.manifest.model.model,
                    ));
                }
            }
            Backend::None => {}
        }
        let msg = slash_info::agents_list_message(lines, lang);
        self.chat.push_message(chat::Role::System, msg);
    }

    fn handle_copy_slash(&mut self, args: &str) {
        let (text, label, empty_msg) = match copy_slash_target_for_arg(args) {
            Some(CopySlashTarget::Command) => {
                let copy_text = slash_local::copy_target_text(
                    slash_local::CopyTarget::Command,
                    crate::i18n::Lang::Fr,
                );
                (
                    self.chat.last_command_to_copy(),
                    copy_text.label,
                    copy_text.empty_message,
                )
            }
            Some(CopySlashTarget::Response) => {
                let copy_text = slash_local::copy_target_text(
                    slash_local::CopyTarget::Response,
                    crate::i18n::Lang::Fr,
                );
                (
                    self.chat
                        .messages
                        .iter()
                        .rev()
                        .find(|m| matches!(m.role, chat::Role::Agent))
                        .map(|m| m.text.clone()),
                    copy_text.label,
                    copy_text.empty_message,
                )
            }
            None => {
                let msg = slash_local::copy_usage_message(
                    crate::i18n::Lang::Fr,
                    slash_local::CopyUsageSurface::FullTui,
                );
                self.chat.push_message(chat::Role::System, msg.to_string());
                return;
            }
        };
        if let Some(text) = text {
            self.copy_to_clipboard_status(text, label);
        } else {
            self.chat
                .push_message(chat::Role::System, empty_msg.to_string());
        }
    }

    fn handle_kill_slash(&mut self) {
        if let Some(ref target) = self.chat_target {
            let name = target.agent_name.clone();
            let lang = crate::i18n::current();
            // Killing the Captain primary agent would disconnect the TUI from its orchestrator.
            if slash_kill::is_protected_agent(&name) {
                self.chat.push_message(
                    chat::Role::System,
                    slash_kill::protected_agent_message(lang).to_string(),
                );
                return;
            }
            match &self.backend {
                Backend::Daemon { base_url } => {
                    if let Some(ref id) = target.agent_id_daemon {
                        let client = crate::daemon_client();
                        let url = format!("{base_url}/api/agents/{id}");
                        match client.delete(&url).send() {
                            Ok(r) if r.status().is_success() => {
                                self.chat.push_message(
                                    chat::Role::System,
                                    slash_kill::kill_success_message(lang, &name),
                                );
                            }
                            _ => {
                                self.chat.push_message(
                                    chat::Role::System,
                                    slash_kill::kill_failed_message(lang, &name),
                                );
                            }
                        }
                    }
                }
                Backend::InProcess { kernel } => {
                    if let Some(id) = target.agent_id_inprocess {
                        match kernel.kill_agent(id) {
                            Ok(()) => {
                                self.chat.push_message(
                                    chat::Role::System,
                                    slash_kill::kill_success_message(lang, &name),
                                );
                            }
                            Err(e) => {
                                self.chat.push_message(
                                    chat::Role::System,
                                    slash_kill::kill_error_message(lang, e),
                                );
                            }
                        }
                    }
                }
                Backend::None => {
                    self.chat.push_message(
                        chat::Role::System,
                        slash_kill::no_backend_message(lang).to_string(),
                    );
                }
            }
        }
    }

    fn open_hub_slash_route(&mut self, route: HubSlashRoute) {
        match route {
            HubSlashRoute::Automation(view) => self.open_automation_view(view),
            HubSlashRoute::Learning(view) => self.open_learning_view(view),
            HubSlashRoute::Capabilities(view) => self.open_capabilities_view(view),
            HubSlashRoute::Connections(view) => self.open_connections_view(view),
        }
    }

    fn apply_surface_slash_route(&mut self, route: SurfaceSlashRoute) {
        match route {
            SurfaceSlashRoute::SwitchTab(tab) => self.switch_tab(tab),
            SurfaceSlashRoute::OpenOverlay(tab) => self.open_overlay(tab),
        }
    }

    // ─── Drawing ─────────────────────────────────────────────────────────────

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        match frame_draw_route_for_state(area, self.phase) {
            FrameDrawRoute::TooSmall {
                min_width,
                min_height,
            } => {
                chrome::draw_too_small(frame, area, min_width, min_height);
            }
            FrameDrawRoute::Welcome => {
                self.draw_welcome_frame(frame, area);
            }
            FrameDrawRoute::Wizard => wizard::draw(frame, area, &mut self.wizard),
            FrameDrawRoute::ResumePrompt => {
                resume_prompt::draw(frame, area, self.pending_resume.as_ref());
            }
            FrameDrawRoute::Main => {
                self.draw_main_frame(frame, area);
            }
        }
    }

    fn draw_welcome_frame(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        welcome::draw(frame, area, &mut self.welcome);

        for toast in chrome::welcome_toasts(
            self.kernel_booting,
            self.welcome.tick,
            self.kernel_boot_error.as_deref(),
        )
        .into_iter()
        .flatten()
        {
            chrome::render_toast(frame, area, &toast.message, toast.color);
        }
    }

    fn draw_main_frame(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let composition =
            main_draw_composition_for_state(area, self.overlay_tab, self.file_picker.is_some());
        let content = composition.content_area;

        self.draw_tab_bar(frame, composition.tab_bar_area);
        self.draw_main_content(frame, content);

        for layer in composition.layer_routes.into_iter().flatten() {
            match layer {
                MainDrawLayerRoute::Overlay(overlay) => {
                    self.draw_overlay(frame, content, overlay);
                }
                MainDrawLayerRoute::FilePicker => {
                    if let Some(ref picker) = self.file_picker {
                        screens::file_picker::draw(frame, content, picker);
                    }
                }
            }
        }
    }

    fn draw_main_content(&mut self, frame: &mut ratatui::Frame, content: Rect) {
        match main_draw_route_for_tab(self.active_tab) {
            MainDrawRoute::Dashboard => dashboard::draw(frame, content, &mut self.dashboard),
            MainDrawRoute::Agents => agents::draw(frame, content, &mut self.agents),
            MainDrawRoute::Chat => {
                self.chat.mouse_capture_enabled = self.mouse_capture_enabled;
                chat::draw(frame, content, &mut self.chat, &mut self.image_cache)
            }
            MainDrawRoute::Projects => projects::draw(frame, content, &mut self.projects),
            MainDrawRoute::ConnectionsHub => self.draw_connections_hub(frame, content),
            MainDrawRoute::AutomationHub => self.draw_automation_hub(frame, content),
            MainDrawRoute::Triggers => triggers::draw(frame, content, &mut self.triggers),
            MainDrawRoute::Cron => cron::draw(frame, content, &mut self.cron),
            MainDrawRoute::Approvals => approvals::draw(frame, content, &mut self.approvals),
            MainDrawRoute::Budget => budget::draw(frame, content, &mut self.budget),
            MainDrawRoute::Graph => graph::draw(frame, content, &mut self.graph),
            MainDrawRoute::Sessions => sessions::draw(frame, content, &mut self.sessions),
            MainDrawRoute::Memory => memory::draw(frame, content, &mut self.memory),
            MainDrawRoute::LearningHub => self.draw_learning_hub(frame, content),
            MainDrawRoute::SkillsProposed => {
                skills_proposed::draw(frame, content, &mut self.skills_proposed)
            }
            MainDrawRoute::CapabilitiesHub => self.draw_capabilities_hub(frame, content),
            MainDrawRoute::Hands => hands::draw(frame, content, &mut self.hands),
            MainDrawRoute::Extensions => extensions::draw(frame, content, &mut self.extensions),
            MainDrawRoute::Templates => templates::draw(frame, content, &mut self.templates),
            MainDrawRoute::Security => security::draw(frame, content, &mut self.security),
            MainDrawRoute::Audit => audit::draw(frame, content, &mut self.audit),
            MainDrawRoute::Usage => usage::draw(frame, content, &mut self.usage),
            MainDrawRoute::Settings => settings::draw(frame, content, &mut self.settings),
            MainDrawRoute::Peers => peers::draw(frame, content, &mut self.peers),
            MainDrawRoute::Comms => comms::draw(frame, content, &mut self.comms),
            MainDrawRoute::Logs => logs::draw(frame, content, &mut self.logs),
        }
    }

    fn draw_automation_hub(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let composition = hub_draw_composition_for_area(area);
        let content = composition.content_area;
        let labels = AUTOMATION_VIEWS
            .iter()
            .map(|view| view.label())
            .collect::<Vec<_>>();
        hub_nav::draw(
            frame,
            composition.nav_area,
            "Automation",
            &labels,
            self.automation_view.index(),
        );
        match automation_hub_draw_route_for_view(self.automation_view) {
            AutomationHubDrawRoute::Workflows => {
                workflows::draw(frame, content, &mut self.workflows)
            }
            AutomationHubDrawRoute::Triggers => triggers::draw(frame, content, &mut self.triggers),
            AutomationHubDrawRoute::Cron => cron::draw(frame, content, &mut self.cron),
            AutomationHubDrawRoute::Approvals => {
                approvals::draw(frame, content, &mut self.approvals)
            }
        }
    }

    fn draw_learning_hub(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let composition = hub_draw_composition_for_area(area);
        let content = composition.content_area;
        let labels = LEARNING_VIEWS
            .iter()
            .map(|view| view.label())
            .collect::<Vec<_>>();
        hub_nav::draw(
            frame,
            composition.nav_area,
            "Learning",
            &labels,
            self.learning_view.index(),
        );
        match learning_hub_draw_route_for_view(self.learning_view) {
            LearningHubDrawRoute::Review => learning::draw(frame, content, &mut self.learning),
            LearningHubDrawRoute::SkillProposals => {
                skills_proposed::draw(frame, content, &mut self.skills_proposed)
            }
            LearningHubDrawRoute::Memory => memory::draw(frame, content, &mut self.memory),
            LearningHubDrawRoute::Graph => graph::draw(frame, content, &mut self.graph),
        }
    }

    fn draw_capabilities_hub(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let composition = hub_draw_composition_for_area(area);
        let content = composition.content_area;
        let labels = CAPABILITIES_VIEWS
            .iter()
            .map(|view| view.label())
            .collect::<Vec<_>>();
        hub_nav::draw(
            frame,
            composition.nav_area,
            "Capabilities",
            &labels,
            self.capabilities_view.index(),
        );
        match capabilities_hub_draw_route_for_view(self.capabilities_view) {
            CapabilitiesHubDrawRoute::Native => {
                native_capabilities::draw(frame, content, &mut self.native_capabilities)
            }
            CapabilitiesHubDrawRoute::Skills => skills::draw(frame, content, &mut self.skills),
        }
    }

    fn draw_connections_hub(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let composition = hub_draw_composition_for_area(area);
        let content = composition.content_area;
        let labels = CONNECTIONS_VIEWS
            .iter()
            .map(|view| view.label())
            .collect::<Vec<_>>();
        hub_nav::draw(
            frame,
            composition.nav_area,
            "Connections",
            &labels,
            self.connections_view.index(),
        );
        match connections_hub_draw_route_for_view(self.connections_view) {
            ConnectionsHubDrawRoute::Channels => channels::draw(frame, content, &mut self.channels),
            ConnectionsHubDrawRoute::Extensions => {
                extensions::draw(frame, content, &mut self.extensions)
            }
            ConnectionsHubDrawRoute::Peers => peers::draw(frame, content, &mut self.peers),
            ConnectionsHubDrawRoute::Comms => comms::draw(frame, content, &mut self.comms),
        }
    }

    fn draw_overlay(&mut self, frame: &mut ratatui::Frame, base: Rect, tab: Tab) {
        let inner = chrome::draw_overlay_shell(frame, base, tab.label());

        match overlay_draw_route_for_tab(tab) {
            OverlayDrawRoute::Memory => memory::draw(frame, inner, &mut self.memory),
            OverlayDrawRoute::Learning => learning::draw(frame, inner, &mut self.learning),
            OverlayDrawRoute::SkillsProposed => {
                skills_proposed::draw(frame, inner, &mut self.skills_proposed)
            }
            OverlayDrawRoute::Cron => cron::draw(frame, inner, &mut self.cron),
            OverlayDrawRoute::Approvals => approvals::draw(frame, inner, &mut self.approvals),
            OverlayDrawRoute::Budget => budget::draw(frame, inner, &mut self.budget),
            OverlayDrawRoute::Graph => graph::draw(frame, inner, &mut self.graph),
            OverlayDrawRoute::Logs => logs::draw(frame, inner, &mut self.logs),
            OverlayDrawRoute::Settings => settings::draw(frame, inner, &mut self.settings),
            OverlayDrawRoute::Unsupported => {
                // Unsupported overlay target — close it silently rather than
                // leave an empty popup on screen.
                self.apply_overlay_state(overlay_state_after_close());
            }
        }
    }

    fn draw_tab_bar(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let labels = TABS.iter().map(|tab| tab.label()).collect::<Vec<_>>();
        tab_bar::draw(
            frame,
            area,
            &labels,
            self.active_tab.index(),
            &mut self.tab_scroll_offset,
            self.ctrl_c_pending,
        );
    }
}

#[cfg(test)]
mod app_event_route_tests {
    use super::*;

    #[test]
    fn boot_resume_owner_accepts_exact_id_or_recreated_agent_name() {
        assert!(resume_owner_matches(
            Some("agent-a"),
            "old-name",
            "agent-a",
            "captain"
        ));
        assert!(resume_owner_matches(
            Some("retired-agent-id"),
            "Captain",
            "new-agent-id",
            "captain"
        ));
        assert!(!resume_owner_matches(
            Some("agent-a"),
            "researcher",
            "agent-b",
            "captain"
        ));
    }

    #[test]
    fn handle_event_routes_tick_and_dashboard_data() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, tx);

        app.handle_event(AppEvent::Tick);
        assert_eq!(app.tick_count, 1);

        app.handle_event(AppEvent::DashboardData {
            agent_count: 3,
            uptime_secs: 42,
            version: "2.0.0".to_string(),
            provider: "codex".to_string(),
            model: "gpt-5-codex".to_string(),
            status: dashboard::StatusSnapshot {
                agent_count: 3,
                runtime_health_state: "ok".to_string(),
                ..Default::default()
            },
        });

        assert_eq!(app.dashboard.agent_count, 3);
        assert_eq!(app.dashboard.uptime_secs, 42);
        assert_eq!(app.dashboard.version, "2.0.0");
        assert_eq!(app.dashboard.provider, "codex");
        assert_eq!(app.dashboard.model, "gpt-5-codex");
        assert_eq!(app.dashboard.status.runtime_health_state, "ok");
        assert!(!app.dashboard.loading);
    }

    #[test]
    fn provider_quota_event_updates_full_tui_and_preserves_last_good_observation() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, tx);
        let status = provider_quota::ProviderQuotaStatus {
            state: "warning".to_string(),
            reported_by_provider: true,
            quotas: vec![provider_quota::ProviderQuota {
                provider: "codex".to_string(),
                limit_id: "codex".to_string(),
                limit_name: "Codex".to_string(),
                ..Default::default()
            }],
        };

        app.handle_event(AppEvent::ProviderQuotasLoaded(Ok(status.clone())));
        assert_eq!(app.chat.provider_quota_status, status);
        assert_eq!(app.budget.provider_state, "warning");
        assert_eq!(app.budget.provider_quotas, status.quotas);

        app.handle_event(AppEvent::ProviderQuotasLoaded(Err(
            "daemon restarting".to_string()
        )));
        assert_eq!(app.chat.provider_quota_status, status);
    }

    #[test]
    fn answer_ask_user_is_a_no_op_without_a_connected_backend() {
        // In-process dispatch requires a live kernel and daemon dispatch a
        // live HTTP endpoint — both exercised via manual tmux/curl testing
        // (see T4). This defensive case is the one branch unit-testable in
        // isolation: no backend connected yet must never panic or touch
        // current_stream_input_tx.
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, tx);
        assert!(matches!(app.backend, Backend::None));

        app.handle_chat_action(chat::ChatAction::AnswerAskUser("bleu".to_string()));

        assert!(app.current_stream_input_tx.is_none());
    }

    #[test]
    fn handle_event_routes_stream_started_interjection_tx() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);
        let (interject_tx, _interject_rx) = tokio::sync::mpsc::channel(1);

        app.handle_event(AppEvent::StreamStarted { interject_tx });

        assert!(app.current_stream_input_tx.is_some());
    }

    #[test]
    fn handle_event_routes_agent_spawned_api_sheet_to_operator_notice() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);
        let body = serde_json::json!({
            "agent_api_provisioning": {
                "protocol": "agent-as-service.v1",
                "status": "ingress_ready",
                "manifest_url": "/api/agents/a1/api/manifest",
                "audit_events_url": "/api/agents/a1/api/events",
                "ingress": {
                    "status": "ready",
                    "ingress_url": "/hooks/agents/a1/ingress",
                    "auth_scheme": "Authorization: Bearer $TOKEN",
                    "token_env": "CAPTAIN_AGENT_A1_TOKEN",
                    "token": "secret-token"
                },
                "egress": {
                    "status": "pending_callback_url",
                    "configure_url": "/api/agents/a1/api/egress/configure",
                    "test_url": "/api/agents/a1/api/egress/test",
                    "queue_status_url": "/api/agents/a1/api/egress/queue"
                },
                "operator_actions": [
                    "Ingress is ready, but Captain cannot infer the external callback URL."
                ]
            }
        });
        let api_sheet =
            crate::agent_api_sheet::AgentApiSpawnSheet::from_spawn_body(&body).expect("sheet");

        app.handle_event(AppEvent::AgentSpawned {
            id: "a1".to_string(),
            name: "veille".to_string(),
            api_sheet: Some(api_sheet),
        });

        assert!(!app
            .chat
            .messages
            .iter()
            .any(|message| message.text.contains("secret-token")));
        assert!(app
            .chat
            .operator_notices
            .iter()
            .any(|line| line.contains("Agent API provisioned")));
        assert!(app
            .chat
            .operator_notices
            .iter()
            .any(|line| line.contains("secret-token")));
        assert_eq!(app.chat.agent_name, "veille");
    }

    #[test]
    fn handle_slash_command_routes_status_family() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);

        app.handle_slash_command("/status");

        assert_eq!(app.chat.messages.len(), 1);
        assert_eq!(
            app.chat.messages[0].text,
            slash_info::status_message(
                slash_info::StatusSnapshot::Disconnected,
                crate::i18n::current()
            )
        );
    }

    #[test]
    fn handle_main_tab_key_routes_dashboard_keys() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);
        let key = ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('k'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        );

        app.handle_main_tab_key(Tab::Dashboard, key);

        assert_eq!(app.dashboard.audit_scroll, 1);
    }

    #[test]
    fn handle_project_action_routes_open_chat() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);

        app.handle_project_action(projects::ProjectAction::OpenChat("alpha".to_string()));

        assert_eq!(app.active_tab, Tab::Chat);
        assert_eq!(
            app.chat
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("Project workspace active: alpha")
        );
    }

    #[test]
    fn draw_welcome_frame_renders_without_panic() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| app.draw(frame)).expect("draw");
    }

    #[test]
    fn handle_key_routes_main_global_quit() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);
        app.phase = Phase::Main;
        let key = ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('q'),
            ratatui::crossterm::event::KeyModifiers::CONTROL,
        );

        app.handle_key(key);

        assert!(app.should_quit);
    }

    #[test]
    fn handle_image_attach_requires_daemon_after_prepare() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("capture.png");
        std::fs::write(&path, b"png").expect("write test image");
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);

        app.handle_image_attach(&path.to_string_lossy());

        assert!(app.chat.pending_attachments.is_empty());
        assert_eq!(
            app.chat
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some(slash_attachment::upload_requires_daemon_message())
        );
    }

    #[test]
    fn switch_model_routes_missing_backend_message() {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, event_tx);

        app.switch_model("openai/gpt-5.1", None);

        assert_eq!(
            app.chat
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some(slash_model::no_backend_connected_message())
        );
    }

    #[test]
    fn startup_phase_routes_setup_resume_and_welcome() {
        assert!(matches!(
            startup_phase_for_state(true, false),
            Phase::Boot(BootScreen::Wizard)
        ));
        assert!(matches!(
            startup_phase_for_state(false, true),
            Phase::Boot(BootScreen::ResumePrompt)
        ));
        assert!(matches!(
            startup_phase_for_state(false, false),
            Phase::Boot(BootScreen::Welcome)
        ));
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Entry point for the TUI interactive mode.
pub fn run(config: Option<PathBuf>) {
    use ratatui::crossterm::event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
    };
    use ratatui::crossterm::execute;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
        original_hook(info);
    }));

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let mouse_capture_enabled = event::mouse_capture_default();
    let _ = event::set_mouse_capture(mouse_capture_enabled);

    // 50ms tick → 20fps spinner animation, snappy key response
    let (tx, rx) = event::spawn_event_thread(Duration::from_millis(50));
    let mut app = App::new(config, tx);
    app.mouse_capture_enabled = mouse_capture_enabled;
    app.enter_startup_phase();

    // ── Main loop ────────────────────────────────────────────────────────────
    // Draw first, then block on events. This ensures the first frame appears
    // immediately, before any event processing.
    while !app.should_quit {
        terminal
            .draw(|frame| app.draw(frame))
            .expect("Failed to draw");

        // Block until at least one event arrives (or 33ms timeout for ~30fps)
        match rx.recv_timeout(Duration::from_millis(33)) {
            Ok(ev) => app.handle_event(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // Drain all queued events immediately (batch processing)
        while let Ok(ev) = rx.try_recv() {
            app.handle_event(ev);
        }
    }

    let _ = event::set_mouse_capture(false);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}
