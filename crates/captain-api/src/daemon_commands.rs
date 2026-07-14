//! Global daemon slash commands handled before the LLM.
//!
//! These commands are product/ops controls, not agent prompts. Keeping them
//! here makes the behavior shared by channels and the HTTP chat API.

use crate::daemon_control_drain::{active_work_deferred_text, record_control_deferred};
use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_kernel::auth::Action;
use captain_kernel::CaptainKernel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tracing::{error, info, warn};

static RESTART_SCHEDULED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_SCHEDULED: AtomicBool = AtomicBool::new(false);

const PENDING_READY_DIR: &str = "system-control/pending-ready";
const RESTART_HELPER_LOG: &str = "restart-helper.log";
const RESTART_EXIT_CODE: i32 = 75;

#[derive(Debug, Clone)]
pub struct DaemonCommandOrigin {
    pub channel: String,
    pub sender_user_id: String,
    pub recipient_id: Option<String>,
    pub thread_id: Option<String>,
    pub source_message_id: Option<String>,
}

impl DaemonCommandOrigin {
    pub fn new(
        channel: impl Into<String>,
        sender_user_id: impl Into<String>,
        recipient_id: Option<String>,
        thread_id: Option<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            sender_user_id: sender_user_id.into(),
            recipient_id,
            thread_id,
            source_message_id: None,
        }
    }

    pub fn api(channel: Option<&str>, sender_id: Option<&str>) -> Self {
        Self::new(
            channel.unwrap_or("web"),
            sender_id.unwrap_or("local-api"),
            None,
            None,
        )
    }

    pub fn with_source_message_id(mut self, source_message_id: Option<String>) -> Self {
        self.source_message_id = source_message_id;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingReadyNotification {
    id: String,
    command: String,
    channel: String,
    recipient_id: String,
    thread_id: Option<String>,
    requested_by: String,
    requested_at: String,
}

pub fn parse_daemon_slash(text: &str) -> Option<(String, Vec<String>)> {
    let first_line = text.lines().next()?.trim();
    let slash = first_line.strip_prefix('/')?;
    let mut parts = slash.split_whitespace();
    let name = parts.next()?.to_ascii_lowercase();
    if !is_daemon_command(&name) {
        return None;
    }
    Some((name, parts.map(ToString::to_string).collect()))
}

pub fn is_daemon_command(name: &str) -> bool {
    matches!(
        name,
        "status" | "health" | "version" | "reload" | "restart" | "shutdown" | "config"
    )
}

pub async fn handle_daemon_command(
    kernel: Arc<CaptainKernel>,
    started_at: Option<Instant>,
    shutdown_notify: Option<Arc<Notify>>,
    command: &str,
    args: &[String],
    origin: DaemonCommandOrigin,
) -> String {
    let command = command.trim_start_matches('/').to_ascii_lowercase();
    match command.as_str() {
        "status" => status_text(&kernel, started_at).await,
        "health" => health_text(&kernel, started_at).await,
        "version" => version_text(&kernel),
        "config" => {
            if let Err(reason) = authorize_sensitive_command(&kernel, &origin, Action::ModifyConfig)
            {
                return denied_text(&reason);
            }
            config_text(&kernel)
        }
        "reload" => {
            if let Err(reason) = authorize_sensitive_command(&kernel, &origin, Action::ModifyConfig)
            {
                return denied_text(&reason);
            }
            reload_text(&kernel)
        }
        "restart" => {
            if let Err(reason) = authorize_sensitive_command(&kernel, &origin, Action::ModifyConfig)
            {
                return denied_text(&reason);
            }
            if args
                .first()
                .map(|s| s.eq_ignore_ascii_case("status"))
                .unwrap_or(false)
            {
                return restart_status_text(&kernel).await;
            }
            restart_text(kernel, origin).await
        }
        "shutdown" => {
            if let Err(reason) = authorize_sensitive_command(&kernel, &origin, Action::ModifyConfig)
            {
                return denied_text(&reason);
            }
            shutdown_text(kernel, args, shutdown_notify).await
        }
        _ => "Commande daemon inconnue.".to_string(),
    }
}

pub async fn notify_pending_ready(
    kernel: Arc<CaptainKernel>,
    adapter: Arc<dyn ChannelAdapter>,
    started_at: Instant,
) {
    let channel = adapter.name().to_string();
    let pending_dir = pending_ready_dir(&kernel);
    let Ok(entries) = std::fs::read_dir(&pending_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(notification) = serde_json::from_str::<PendingReadyNotification>(&raw) else {
            warn!(path = %path.display(), "invalid pending daemon ready notification");
            continue;
        };
        if notification.channel != channel {
            continue;
        }

        let user = ChannelUser {
            platform_id: notification.recipient_id.clone(),
            display_name: "Captain".to_string(),
            captain_user: None,
        };
        let mut metadata = HashMap::new();
        if let Some(thread_id) = notification.thread_id.as_deref() {
            metadata.insert("thread_id".to_string(), serde_json::json!(thread_id));
        }
        let text = format!(
            "✅ Captain redémarré.\n{}\n{}",
            status_text(&kernel, Some(started_at)).await,
            version_text(&kernel)
        );

        match adapter
            .send_rich(&user, ChannelContent::Text(text), &metadata)
            .await
        {
            Ok(_) => {
                if let Err(e) = std::fs::remove_file(&path) {
                    warn!(path = %path.display(), error = %e, "failed to remove pending ready notification");
                }
            }
            Err(e) => {
                warn!(channel = %channel, error = %e, "failed to send pending daemon ready notification");
            }
        }
    }
}

async fn status_text(kernel: &CaptainKernel, started_at: Option<Instant>) -> String {
    let uptime = started_at
        .map(format_uptime)
        .unwrap_or_else(|| "?".to_string());
    let agents = kernel.registry.count();
    let channels = configured_channel_count(&kernel.config.channels);
    let model = format!(
        "{}/{}",
        kernel.config.default_model.provider, kernel.config.default_model.model
    );
    format!(
        "Captain status: running\nUptime: {uptime}\nAgents: {agents}\nModel: {model}\nChannels configured: {channels}\nAPI: {}",
        kernel.config.api_listen
    )
}

async fn health_text(kernel: &CaptainKernel, started_at: Option<Instant>) -> String {
    let config_path = config_path(kernel);
    let config_state = if config_path.exists() {
        "ok"
    } else {
        "missing"
    };
    let data_state = if kernel.config.data_dir.exists() {
        "ok"
    } else {
        "missing"
    };
    let uptime = started_at
        .map(format_uptime)
        .unwrap_or_else(|| "?".to_string());
    format!(
        "Captain health: ok\nUptime: {uptime}\nConfig: {config_state} ({})\nData dir: {data_state} ({})\nAgents: {}\nMemory backend: {:?}",
        config_path.display(),
        kernel.config.data_dir.display(),
        kernel.registry.count(),
        kernel.config.memory.backend,
    )
}

fn version_text(kernel: &CaptainKernel) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    format!(
        "Captain version: {}\nBinary: {exe}\nHome: {}\nData: {}",
        captain_types::version::captain_version(),
        kernel.config.home_dir.display(),
        kernel.config.data_dir.display()
    )
}

fn config_text(kernel: &CaptainKernel) -> String {
    let path = config_path(kernel);
    match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) => format!("Impossible de lire {}: {e}", path.display()),
    }
}

fn reload_text(kernel: &CaptainKernel) -> String {
    match kernel.reload_config() {
        Ok(plan) => {
            let status = if plan.restart_required {
                "partiel, restart requis"
            } else if plan.has_changes() {
                "appliqué"
            } else {
                "aucun changement"
            };
            let hot_actions = plan
                .hot_actions
                .iter()
                .map(|a| format!("{a:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let restart_reasons = if plan.restart_reasons.is_empty() {
                "aucune".to_string()
            } else {
                plan.restart_reasons.join(", ")
            };
            format!(
                "Config reload: {status}\nHot actions: {}\nRestart required: {}\nRestart reasons: {restart_reasons}",
                if hot_actions.is_empty() {
                    "aucune".to_string()
                } else {
                    hot_actions
                },
                plan.restart_required,
            )
        }
        Err(e) => format!("Config reload failed: {e}"),
    }
}

async fn restart_text(kernel: Arc<CaptainKernel>, origin: DaemonCommandOrigin) -> String {
    if crate::restart_dedupe::is_restart_redelivery(
        &kernel.config.home_dir,
        &origin.channel,
        origin.source_message_id.as_deref(),
    ) {
        return "Restart déjà traité pour ce message; redelivery ignorée.".to_string();
    }

    if let Some(message) = active_work_deferred_text(&kernel, "Restart", "`/restart`") {
        record_control_deferred(
            &kernel,
            "restart",
            crate::shutdown_guard::active_shutdown_work(&kernel),
        );
        return message;
    }

    if RESTART_SCHEDULED.swap(true, Ordering::SeqCst) {
        return "Restart déjà planifié.".to_string();
    }

    if origin.channel == "telegram" {
        if let Err(e) = write_pending_ready_notification(&kernel, &origin, "restart") {
            warn!(error = %e, "failed to persist Telegram restart notification");
        }
    }

    let strategy = restart_strategy_summary(&kernel.config.home_dir);
    let schedule_result = schedule_restart(kernel.config.home_dir.clone());
    if let Err(e) = schedule_result {
        RESTART_SCHEDULED.store(false, Ordering::SeqCst);
        return format!("Restart impossible: {e}");
    }
    if let Err(e) = crate::restart_dedupe::record_restart_processed(
        &kernel.config.home_dir,
        &origin.channel,
        origin.source_message_id.as_deref(),
    ) {
        warn!(error = %e, "failed to persist restart dedupe marker");
    }

    format!(
        "🔄 Restart Captain planifié.\nStratégie: {strategy}\nJe coupe le daemon puis je renvoie un message quand il est revenu."
    )
}

async fn restart_status_text(kernel: &CaptainKernel) -> String {
    let inventory = restart_inventory(&kernel.config.home_dir, &kernel.config.data_dir);
    let helper_tail = helper_log_tail(&inventory.helper_log_path, 8)
        .unwrap_or_else(|| "aucun log helper".to_string());
    let launchd = if inventory.launchd_labels.is_empty() {
        "absent".to_string()
    } else {
        inventory.launchd_labels.join(", ")
    };
    let systemd_user = inventory
        .systemd_user_service
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "absent".to_string());
    let systemd_system = inventory
        .systemd_system_service
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "absent".to_string());
    let tmux_state = match inventory.tmux_session_active {
        Some(true) => "available, session captain-daemon active",
        Some(false) => "available, session captain-daemon inactive",
        None => "absent",
    };
    format!(
        "Captain restart status\nScheduled: {}\nStrategy: {}\nPending Telegram ready notifications: {}\nOverride: {}\nlaunchd: {launchd}\nsystemd user: {systemd_user}\nsystemd system: {systemd_system}\ntmux: {tmux_state}\nFallback: nohup\nHelper log: {}\nLast helper lines:\n{helper_tail}",
        RESTART_SCHEDULED.load(Ordering::SeqCst),
        restart_strategy_summary(&kernel.config.home_dir),
        inventory.pending_ready_count,
        if inventory.override_configured {
            "CAPTAIN_RESTART_COMMAND"
        } else {
            "absent"
        },
        inventory.helper_log_path.display(),
    )
}

async fn shutdown_text(
    kernel: Arc<CaptainKernel>,
    args: &[String],
    shutdown_notify: Option<Arc<Notify>>,
) -> String {
    let confirmed = args
        .first()
        .map(|s| s.eq_ignore_ascii_case("confirm"))
        .unwrap_or(false);
    if !confirmed {
        return "Commande sensible. Relance avec `/shutdown confirm` pour arrêter le daemon."
            .to_string();
    }
    if let Some(message) = active_work_deferred_text(&kernel, "Shutdown", "`/shutdown confirm`") {
        record_control_deferred(
            &kernel,
            "shutdown",
            crate::shutdown_guard::active_shutdown_work(&kernel),
        );
        return message;
    }

    if SHUTDOWN_SCHEDULED.swap(true, Ordering::SeqCst) {
        return "Shutdown déjà planifié.".to_string();
    }

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(900)).await;
        kernel.shutdown();
        if let Some(notify) = shutdown_notify {
            notify.notify_one();
        } else {
            std::process::exit(0);
        }
    });
    "⏹ Shutdown Captain planifié.".to_string()
}

fn authorize_sensitive_command(
    kernel: &CaptainKernel,
    origin: &DaemonCommandOrigin,
    action: Action,
) -> Result<(), String> {
    if is_local_channel(&origin.channel) {
        return Ok(());
    }

    if kernel.auth.is_enabled() {
        let user_id = kernel
            .auth
            .identify(&origin.channel, &origin.sender_user_id)
            .ok_or_else(|| "utilisateur non reconnu".to_string())?;
        return kernel
            .auth
            .authorize(user_id, &action)
            .map_err(|e| e.to_string());
    }

    let allowed_users = exact_allowed_users_for_channel(kernel, &origin.channel);
    if allowed_users
        .iter()
        .any(|allowed| allowed == &origin.sender_user_id)
    {
        return Ok(());
    }

    Err(format!(
        "commande réservée au propriétaire. `{}` doit être explicitement présent dans allowed_users de channels.{}",
        origin.sender_user_id, origin.channel
    ))
}

fn denied_text(reason: &str) -> String {
    format!("Access denied: {reason}")
}

fn exact_allowed_users_for_channel(kernel: &CaptainKernel, channel: &str) -> Vec<String> {
    let Ok(value) = serde_json::to_value(&kernel.config.channels) else {
        return Vec::new();
    };
    value
        .get(channel)
        .and_then(|v| v.get("allowed_users"))
        .and_then(|v| v.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|v| *v != "*")
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn is_local_channel(channel: &str) -> bool {
    matches!(channel, "api" | "cli" | "tui" | "web" | "webchat" | "local")
}

fn configured_channel_count(config: &captain_types::config::ChannelsConfig) -> usize {
    let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(config) else {
        return 0;
    };
    fields
        .into_iter()
        .filter(|(name, value)| name != "silent_mode" && !value.is_null())
        .count()
}

fn config_path(kernel: &CaptainKernel) -> PathBuf {
    kernel.config.home_dir.join("config.toml")
}

fn pending_ready_dir(kernel: &CaptainKernel) -> PathBuf {
    kernel.config.data_dir.join(PENDING_READY_DIR)
}

fn write_pending_ready_notification(
    kernel: &CaptainKernel,
    origin: &DaemonCommandOrigin,
    command: &str,
) -> Result<(), String> {
    let Some(recipient_id) = origin.recipient_id.clone() else {
        return Ok(());
    };
    let dir = pending_ready_dir(kernel);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let id = uuid::Uuid::new_v4().to_string();
    let payload = PendingReadyNotification {
        id: id.clone(),
        command: command.to_string(),
        channel: origin.channel.clone(),
        recipient_id,
        thread_id: origin.thread_id.clone(),
        requested_by: origin.sender_user_id.clone(),
        requested_at: chrono::Utc::now().to_rfc3339(),
    };
    let path = dir.join(format!("{id}.json"));
    let raw = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| format!("write {}: {e}", path.display()))
}

fn schedule_restart(home_dir: PathBuf) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let script = restart_helper_script(&home_dir, &exe);
    let exit_code = restart_process_exit_code();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(900)).await;
        // The helper itself must ignore SIGHUP. When Captain is running as the
        // foreground process inside a tmux pane, exiting the daemon tears down
        // that pane and otherwise kills a plain child shell before it can
        // relaunch the service.
        match std::process::Command::new("/usr/bin/nohup")
            .arg("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => info!(pid = child.id(), "spawned Captain restart helper"),
            Err(e) => error!(error = %e, "failed to spawn Captain restart helper"),
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        std::process::exit(exit_code);
    });
    Ok(())
}

fn restart_helper_script(home_dir: &Path, exe: &Path) -> String {
    let helper_log_path = home_dir.join(RESTART_HELPER_LOG);
    if let Ok(command) = std::env::var("CAPTAIN_RESTART_COMMAND") {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return format!(
                "exec >> {} 2>&1\n\
                 echo \"[$(date -u +%Y-%m-%dT%H:%M:%SZ)] Captain restart helper starting\"\n\
                 sleep 1\n\
                 echo \"CAPTAIN_RESTART_COMMAND override\"\n\
                 {trimmed}\n",
                shell_quote_path(&helper_log_path),
            );
        }
    }

    let log_path = home_dir.join("captain.log");
    let exe_start = format!("{} start", shell_quote_path(exe));
    let tmux_cmd = shell_quote(&exe_start);
    let recovery_session = format!("captain-daemon-restart-{}", uuid::Uuid::new_v4());
    let fallback_cmd = format!(
        "nohup {} >> {} 2>&1 &",
        exe_start,
        shell_quote_path(&log_path)
    );
    let launchd_block = launchd_restart_block();
    format!(
        "exec >> {} 2>&1\n\
         echo \"[$(date -u +%Y-%m-%dT%H:%M:%SZ)] Captain restart helper starting\"\n\
         sleep 1\n\
         {}\n\
         if command -v systemctl >/dev/null 2>&1; then\n\
           if [ \"$(id -u 2>/dev/null || echo 1)\" != \"0\" ]; then\n\
             if systemctl --user list-unit-files captain.service >/dev/null 2>&1 || [ -f \"${{XDG_CONFIG_HOME:-$HOME/.config}}/systemd/user/captain.service\" ]; then\n\
               if systemctl --user restart captain.service; then\n\
                 echo \"systemd user restart launched\"\n\
                 exit 0\n\
               fi\n\
               echo \"systemd user restart failed; trying next strategy\"\n\
             fi\n\
           fi\n\
           if systemctl list-unit-files captain.service >/dev/null 2>&1 || [ -f /etc/systemd/system/captain.service ] || [ -f /lib/systemd/system/captain.service ] || [ -f /usr/lib/systemd/system/captain.service ]; then\n\
             if systemctl restart captain.service; then\n\
               echo \"systemd restart launched\"\n\
               exit 0\n\
             fi\n\
             echo \"systemd restart failed; relying on process exit/fallback\"\n\
           fi\n\
         fi\n\
         if command -v tmux >/dev/null 2>&1; then\n\
           if tmux new-session -d -s captain-daemon {} || tmux new-session -d -s {} {}; then\n\
             echo \"tmux restart launched\"\n\
             exit 0\n\
           fi\n\
           echo \"tmux restart failed; falling back to nohup\"\n\
         fi\n\
         {}\n\
         echo \"nohup restart launched\"\n",
        shell_quote_path(&helper_log_path),
        launchd_block,
        tmux_cmd,
        shell_quote(&recovery_session),
        tmux_cmd,
        fallback_cmd,
    )
}

#[derive(Debug)]
struct RestartInventory {
    override_configured: bool,
    launchd_labels: Vec<String>,
    systemd_user_service: Option<PathBuf>,
    systemd_system_service: Option<PathBuf>,
    tmux_session_active: Option<bool>,
    pending_ready_count: usize,
    helper_log_path: PathBuf,
}

fn restart_inventory(home_dir: &Path, data_dir: &Path) -> RestartInventory {
    RestartInventory {
        override_configured: std::env::var("CAPTAIN_RESTART_COMMAND")
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
        launchd_labels: launchd_label_candidates(),
        systemd_user_service: systemd_user_service_path().filter(|p| p.exists()),
        systemd_system_service: systemd_system_service_path(),
        tmux_session_active: tmux_session_active(),
        pending_ready_count: pending_ready_count(data_dir),
        helper_log_path: home_dir.join(RESTART_HELPER_LOG),
    }
}

fn restart_strategy_summary(home_dir: &Path) -> String {
    if std::env::var("CAPTAIN_RESTART_COMMAND")
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return "CAPTAIN_RESTART_COMMAND override".to_string();
    }
    let launchd_labels = launchd_label_candidates();
    if !launchd_labels.is_empty() {
        return format!(
            "launchd ({}) → fallback tmux/nohup",
            launchd_labels.join(", ")
        );
    }
    if let Some(path) = systemd_user_service_path().filter(|p| p.exists()) {
        return format!("systemd --user ({}) → fallback tmux/nohup", path.display());
    }
    if let Some(path) = systemd_system_service_path() {
        return format!("systemd ({}) → fallback tmux/nohup", path.display());
    }
    let tmux = match tmux_session_active() {
        Some(true) => "tmux active session",
        Some(false) => "tmux available",
        None => "tmux unavailable",
    };
    format!(
        "{tmux} → nohup fallback ({})",
        home_dir.join("captain.log").display()
    )
}

fn restart_process_exit_code() -> i32 {
    if cfg!(target_os = "linux")
        && (std::env::var_os("INVOCATION_ID").is_some()
            || std::env::var_os("SYSTEMD_EXEC_PID").is_some()
            || std::env::var_os("JOURNAL_STREAM").is_some())
    {
        RESTART_EXIT_CODE
    } else {
        0
    }
}

fn launchd_restart_block() -> String {
    let labels = launchd_label_candidates();
    if labels.is_empty() {
        return String::new();
    }
    let quoted_labels = labels
        .iter()
        .map(|label| shell_quote(label))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "if command -v launchctl >/dev/null 2>&1; then\n\
           for label in {quoted_labels}; do\n\
             uid=\"$(id -u 2>/dev/null || echo)\"\n\
             if [ -n \"$uid\" ] && launchctl kickstart -k \"gui/$uid/$label\"; then\n\
               echo \"launchd restart launched: $label\"\n\
               exit 0\n\
             fi\n\
             if [ -n \"$uid\" ] && launchctl kickstart -k \"user/$uid/$label\"; then\n\
               echo \"launchd user restart launched: $label\"\n\
               exit 0\n\
             fi\n\
           done\n\
           echo \"launchd restart failed; trying next strategy\"\n\
         fi"
    )
}

fn launchd_label_candidates() -> Vec<String> {
    let Some(home) = user_home_dir() else {
        return Vec::new();
    };
    let launch_agents = home.join("Library/LaunchAgents");
    let mut paths = BTreeSet::new();
    paths.insert(launch_agents.join("ai.captain.desktop.plist"));
    paths.insert(launch_agents.join("com.captain.daemon.plist"));
    paths.insert(launch_agents.join("sh.captain.daemon.plist"));
    if let Ok(entries) = std::fs::read_dir(&launch_agents) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if name.contains("captain") && name.ends_with(".plist") {
                paths.insert(path);
            }
        }
    }

    let mut labels = BTreeSet::new();
    for path in paths {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        if !plist_runs_captain_daemon(&raw) {
            continue;
        }
        if let Some(label) = extract_launchd_label(&raw) {
            labels.insert(label);
        }
    }
    labels.into_iter().collect()
}

fn plist_runs_captain_daemon(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.contains("captain")
        && (lower.contains("<string>start</string>") || lower.contains("captain start"))
}

fn extract_launchd_label(raw: &str) -> Option<String> {
    let key_pos = raw.find("<key>Label</key>")?;
    let rest = &raw[key_pos..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    let label = rest[start..start + end].trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

fn systemd_user_service_path() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("systemd/user/captain.service"));
    }
    user_home_dir().map(|home| home.join(".config/systemd/user/captain.service"))
}

fn systemd_system_service_path() -> Option<PathBuf> {
    [
        "/etc/systemd/system/captain.service",
        "/lib/systemd/system/captain.service",
        "/usr/lib/systemd/system/captain.service",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.exists())
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn tmux_session_active() -> Option<bool> {
    let available = Command::new("tmux")
        .arg("-V")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    if !available {
        return None;
    }
    let active = Command::new("tmux")
        .args(["has-session", "-t", "captain-daemon"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    Some(active)
}

fn pending_ready_count(data_dir: &Path) -> usize {
    std::fs::read_dir(data_dir.join(PENDING_READY_DIR))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .count()
        })
        .unwrap_or(0)
}

fn helper_log_tail(path: &Path, line_count: usize) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut lines = raw
        .lines()
        .rev()
        .take(line_count)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    lines.reverse();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.display().to_string())
}

fn shell_quote(raw: &str) -> String {
    let mut quoted = String::from("'");
    for ch in raw.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn format_uptime(started_at: Instant) -> String {
    let secs = started_at.elapsed().as_secs();
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_daemon_slash_from_first_line_only() {
        let parsed = parse_daemon_slash("/restart now\nthis is ignored").unwrap();
        assert_eq!(parsed.0, "restart");
        assert_eq!(parsed.1, vec!["now"]);
    }

    #[test]
    fn ignores_agent_slashes() {
        assert!(parse_daemon_slash("/project set captain").is_none());
        assert!(parse_daemon_slash("hello /restart").is_none());
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        let quoted = shell_quote_path(Path::new("/tmp/captain's log"));
        assert_eq!(quoted, "'/tmp/captain'\\''s log'");
    }

    #[test]
    fn restart_helper_prefers_tmux_with_nohup_fallback() {
        let script = restart_helper_script(
            Path::new("/tmp/captain home"),
            Path::new("/opt/captain/bin/captain"),
        );
        assert!(script.contains("systemctl --user restart captain.service"));
        assert!(script.contains("systemctl restart captain.service"));
        assert!(script.contains("tmux new-session -d -s captain-daemon"));
        assert!(script.contains("captain-daemon-restart-"));
        assert!(script.contains("nohup '/opt/captain/bin/captain' start"));
        assert!(script.contains("restart-helper.log"));
    }

    #[test]
    fn extracts_launchd_label() {
        let plist = r#"
        <plist version="1.0">
          <dict>
            <key>Label</key>
            <string>ai.captain.desktop</string>
            <key>ProgramArguments</key>
            <array>
              <string>/Users/me/.captain/bin/captain</string>
              <string>start</string>
            </array>
          </dict>
        </plist>
        "#;
        assert!(plist_runs_captain_daemon(plist));
        assert_eq!(
            extract_launchd_label(plist).as_deref(),
            Some("ai.captain.desktop")
        );
    }

    #[test]
    fn rejects_non_daemon_launchd_plist() {
        let plist = r#"
        <dict>
          <key>Label</key>
          <string>ai.captain.desktop</string>
          <key>Program</key>
          <string>/Applications/Captain.app/Contents/MacOS/Captain</string>
        </dict>
        "#;
        assert!(!plist_runs_captain_daemon(plist));
    }
}
