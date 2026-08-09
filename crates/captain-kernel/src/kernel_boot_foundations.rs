use super::kernel_driver_support::resolve_daemon_api_key;
use crate::auth::AuthManager;
use crate::auto_reply::AutoReplyEngine;
use crate::background::BackgroundExecutor;
use crate::error::{KernelError, KernelResult};
use crate::metering::MeteringEngine;
use crate::supervisor::Supervisor;
use crate::triggers::TriggerEngine;
use captain_memory::{artifacts::ArtifactStore, MemorySubstrate};
use captain_runtime::audit::AuditLog;
use captain_types::config::{BroadcastConfig, KernelConfig, KernelMode};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

pub(super) struct BootCore {
    pub(super) memory: Arc<MemorySubstrate>,
    pub(super) artifact_store: Arc<ArtifactStore>,
    pub(super) supervisor: Supervisor,
    pub(super) background: BackgroundExecutor,
    pub(super) audit_log: Arc<AuditLog>,
    pub(super) metering: Arc<MeteringEngine>,
    pub(super) wasm_sandbox: captain_runtime::sandbox::WasmSandbox,
    pub(super) auth: AuthManager,
    pub(super) model_catalog: captain_runtime::model_catalog::ModelCatalog,
    pub(super) credential_resolver: captain_extensions::credentials::CredentialResolver,
    pub(super) web_ctx: captain_runtime::web_search::WebToolsContext,
    pub(super) cron_scheduler: crate::cron::CronScheduler,
    pub(super) goal_store: Arc<crate::goals::GoalStore>,
    pub(super) approval_manager: crate::approval::ApprovalManager,
    pub(super) bindings: Vec<captain_types::config::AgentBinding>,
    pub(super) broadcast: BroadcastConfig,
    pub(super) auto_reply_engine: AutoReplyEngine,
    pub(super) graph_memory: Arc<crate::graph_memory::GraphMemory>,
    pub(super) triggers: TriggerEngine,
    pub(super) process_manager: Arc<captain_runtime::process_manager::ProcessManager>,
}

pub(super) fn prepare_boot_config(mut config: KernelConfig) -> KernelResult<KernelConfig> {
    if let Ok(listen) = std::env::var("CAPTAIN_LISTEN") {
        config.api_listen = listen;
    }

    captain_types::config::ensure_session_signing_state(
        &config.home_dir.join("config.toml"),
        &mut config.auth,
    )
    .map_err(|error| {
        KernelError::BootFailed(format!("Web session signing state unavailable: {error}"))
    })?;

    if let Some(resolved) = resolve_daemon_api_key(&config.home_dir)
        .map_err(|error| KernelError::BootFailed(format!("Secret sources unavailable: {error}")))?
    {
        if config.api_key.trim().is_empty() || resolved.externally_managed {
            info!("Using API key from {}", resolved.key_name);
            config.api_key = resolved.value;
        }
    }

    config.clamp_bounds();
    log_boot_mode(config.mode);
    log_config_warnings(&config);
    ensure_data_dir(&config)?;
    Ok(config)
}

pub(super) fn build_boot_core(config: &KernelConfig) -> KernelResult<BootCore> {
    let memory = open_boot_memory(config)?;
    let artifact_store = Arc::new(
        ArtifactStore::open(config.data_dir.join("artifacts")).map_err(|error| {
            KernelError::BootFailed(format!("Artifact store unavailable: {error}"))
        })?,
    );
    let credential_resolver = build_boot_credential_resolver(config)?;
    let metering = Arc::new(MeteringEngine::new(Arc::new(
        captain_memory::usage::UsageStore::new(memory.usage_conn()),
    )));
    let supervisor = Supervisor::new();
    let background = BackgroundExecutor::new(supervisor.subscribe());
    let wasm_sandbox = captain_runtime::sandbox::WasmSandbox::new()
        .map_err(|e| KernelError::BootFailed(format!("WASM sandbox init failed: {e}")))?;
    let auth = build_boot_auth(config);
    let model_catalog = build_boot_model_catalog(config, &credential_resolver);
    let audit_log = Arc::new(
        AuditLog::with_db(memory.usage_conn())
            .map_err(|error| KernelError::BootFailed(format!("Audit log unavailable: {error}")))?,
    );
    let approval_manager = crate::approval::ApprovalManager::with_persistence(
        config.approval.clone(),
        &config.home_dir,
        Arc::clone(&audit_log),
    )
    .map_err(|e| KernelError::BootFailed(format!("Approval rules unavailable: {e}")))?;

    Ok(BootCore {
        audit_log,
        web_ctx: build_boot_web_context(config),
        cron_scheduler: build_boot_cron_scheduler(config),
        goal_store: build_boot_goal_store(config),
        approval_manager,
        bindings: config.bindings.clone(),
        broadcast: config.broadcast.clone(),
        auto_reply_engine: AutoReplyEngine::new(config.auto_reply.clone()),
        graph_memory: build_boot_graph_memory(config),
        triggers: TriggerEngine::with_file_trigger_persistence(&config.home_dir),
        process_manager: build_boot_process_manager(config),
        memory,
        artifact_store,
        supervisor,
        background,
        metering,
        wasm_sandbox,
        auth,
        model_catalog,
        credential_resolver,
    })
}

fn log_boot_mode(mode: KernelMode) {
    match mode {
        KernelMode::Stable => {
            info!("Booting Captain kernel in STABLE mode — conservative defaults enforced");
        }
        KernelMode::Dev => {
            warn!("Booting Captain kernel in DEV mode — experimental features enabled");
        }
        KernelMode::Default => {
            info!("Booting Captain kernel...");
        }
    }
}

fn log_config_warnings(config: &KernelConfig) {
    for warning in config.validate() {
        warn!("Config: {}", warning);
    }
}

fn ensure_data_dir(config: &KernelConfig) -> KernelResult<()> {
    std::fs::create_dir_all(&config.data_dir)
        .map_err(|e| KernelError::BootFailed(format!("Failed to create data dir: {e}")))
}

fn open_boot_memory(config: &KernelConfig) -> KernelResult<Arc<MemorySubstrate>> {
    let db_path = boot_memory_db_path(config);
    MemorySubstrate::open(&db_path, config.memory.decay_rate)
        .map(Arc::new)
        .map_err(|e| KernelError::BootFailed(format!("Memory init failed: {e}")))
}

fn boot_memory_db_path(config: &KernelConfig) -> PathBuf {
    config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| config.data_dir.join("captain.db"))
}

fn build_boot_credential_resolver(
    config: &KernelConfig,
) -> KernelResult<captain_extensions::credentials::CredentialResolver> {
    let vault = unlock_boot_vault(config);
    captain_extensions::credentials::CredentialResolver::from_home(vault, &config.home_dir)
        .map_err(|error| KernelError::BootFailed(format!("Secret sources unavailable: {error}")))
}

fn unlock_boot_vault(config: &KernelConfig) -> Option<captain_extensions::vault::CredentialVault> {
    let vault_path = config.home_dir.join("vault.enc");
    if !vault_path.exists() {
        return None;
    }

    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);
    match vault.unlock() {
        Ok(()) => {
            info!("Credential vault unlocked ({} entries)", vault.len());
            Some(vault)
        }
        Err(e) => {
            warn!("Credential vault exists but could not unlock: {e} — falling back to env vars");
            None
        }
    }
}

fn build_boot_auth(config: &KernelConfig) -> AuthManager {
    let auth = AuthManager::new(&config.users);
    if auth.is_enabled() {
        info!("RBAC enabled with {} users", auth.user_count());
    }
    auth
}

fn build_boot_model_catalog(
    config: &KernelConfig,
    credentials: &captain_extensions::credentials::CredentialResolver,
) -> captain_runtime::model_catalog::ModelCatalog {
    let mut model_catalog = captain_runtime::model_catalog::ModelCatalog::new();
    if !config.provider_urls.is_empty() {
        model_catalog.apply_url_overrides(&config.provider_urls);
        info!(
            "applied {} provider URL override(s)",
            config.provider_urls.len()
        );
    }

    model_catalog.load_custom_models(&config.home_dir.join("custom_models.json"));
    model_catalog.detect_auth_with(&|key| credentials.has_credential(key));
    let available_count = model_catalog.available_models().len();
    let total_count = model_catalog.list_models().len();
    let local_count = model_catalog
        .list_providers()
        .iter()
        .filter(|provider| !provider.key_required)
        .count();
    info!(
        "Model catalog: {total_count} models, {available_count} available from configured providers ({local_count} local)"
    );
    model_catalog
}

fn build_boot_web_context(config: &KernelConfig) -> captain_runtime::web_search::WebToolsContext {
    let cache_ttl = std::time::Duration::from_secs(config.web.cache_ttl_minutes * 60);
    let web_cache = Arc::new(captain_runtime::web_cache::WebCache::new(cache_ttl));
    captain_runtime::web_search::WebToolsContext {
        search: captain_runtime::web_search::WebSearchEngine::new(
            config.web.clone(),
            web_cache.clone(),
        ),
        fetch: captain_runtime::web_fetch::WebFetchEngine::new(config.web.fetch.clone(), web_cache),
    }
}

fn build_boot_cron_scheduler(config: &KernelConfig) -> crate::cron::CronScheduler {
    let mut cron_scheduler =
        crate::cron::CronScheduler::new(&config.home_dir, config.max_cron_jobs);
    cron_scheduler.set_default_tz(&config.timezone);
    match cron_scheduler.load() {
        Ok(count) if count > 0 => info!("Loaded {count} cron job(s) from disk"),
        Ok(_) => {}
        Err(e) => warn!("Failed to load cron jobs: {e}"),
    }
    cron_scheduler
}

fn build_boot_goal_store(config: &KernelConfig) -> Arc<crate::goals::GoalStore> {
    let goal_store = Arc::new(crate::goals::GoalStore::new(&config.home_dir));
    match goal_store.load() {
        Ok(count) if count > 0 => info!("Loaded {count} autopilot goal(s) from disk"),
        Ok(_) => {}
        Err(e) => warn!("Failed to load autopilot goals: {e}"),
    }
    goal_store
}

fn build_boot_graph_memory(config: &KernelConfig) -> Arc<crate::graph_memory::GraphMemory> {
    let graph_memory = Arc::new(
        crate::graph_memory::GraphMemory::new(Some(config.home_dir.join("graph.hora")))
            .unwrap_or_else(|e| {
                warn!("Graph memory init failed ({e}), using in-memory");
                crate::graph_memory::GraphMemory::new(None).unwrap()
            }),
    );

    crate::graph_seed::seed_system_docs(&graph_memory);
    seed_tool_self_model(&graph_memory);

    let workspaces_dir = config
        .workspaces_dir
        .clone()
        .unwrap_or_else(|| config.home_dir.join("workspaces"));
    crate::graph_seed::migrate_memory_files(&graph_memory, &workspaces_dir);
    graph_memory
}

fn seed_tool_self_model(graph_memory: &crate::graph_memory::GraphMemory) {
    let defs = captain_runtime::tool_runner::builtin_tool_definitions();
    let tool_names: Vec<&str> = defs.iter().map(|tool| tool.name.as_str()).collect();
    graph_memory.seed_self_model(&tool_names, &[], &[]);
    graph_memory.seed_tool_rules();
}

fn build_boot_process_manager(
    config: &KernelConfig,
) -> Arc<captain_runtime::process_manager::ProcessManager> {
    Arc::new(
        captain_runtime::process_manager::ProcessManager::with_registry_path(
            5,
            config.data_dir.join("process_registry.json"),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_memory_db_path_defaults_under_data_dir() {
        let mut config = KernelConfig::default();
        config.data_dir = PathBuf::from("/tmp/captain-data");
        config.memory.sqlite_path = None;

        assert_eq!(
            boot_memory_db_path(&config),
            PathBuf::from("/tmp/captain-data/captain.db")
        );
    }

    #[test]
    fn boot_memory_db_path_honors_configured_sqlite_path() {
        let mut config = KernelConfig::default();
        config.data_dir = PathBuf::from("/tmp/captain-data");
        config.memory.sqlite_path = Some(PathBuf::from("/tmp/custom/captain.sqlite"));

        assert_eq!(
            boot_memory_db_path(&config),
            PathBuf::from("/tmp/custom/captain.sqlite")
        );
    }

    #[test]
    fn boot_fails_closed_when_external_secret_registry_is_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path()
                .join(captain_extensions::external_secret_sources::SECRET_SOURCES_FILENAME),
            "version = 1\n[sources.INVALID]\ntype = \"file\"\npath = \"relative\"\n",
        )
        .unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            ..Default::default()
        };

        let error = prepare_boot_config(config).unwrap_err().to_string();

        assert!(error.contains("Secret sources unavailable"), "{error}");
        assert!(!error.contains("secret-value"), "{error}");
    }

    #[test]
    fn authoritative_external_daemon_key_overrides_stale_config_literal() {
        let tmp = tempfile::tempdir().unwrap();
        let mounted = tmp.path().join("daemon-key");
        std::fs::write(&mounted, "fresh-external-key\n").unwrap();
        std::fs::write(
            tmp.path()
                .join(captain_extensions::external_secret_sources::SECRET_SOURCES_FILENAME),
            format!(
                "version = 1\n[sources.CAPTAIN_DAEMON_API_KEY]\ntype = \"file\"\npath = {:?}\n",
                mounted.display().to_string()
            ),
        )
        .unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            api_key: "stale-config-key".to_string(),
            ..Default::default()
        };

        let prepared = prepare_boot_config(config).unwrap();

        assert_eq!(prepared.api_key, "fresh-external-key");
    }

    #[test]
    fn boot_provisions_one_private_session_signing_state() {
        let temporary = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: temporary.path().to_path_buf(),
            data_dir: temporary.path().join("data"),
            ..Default::default()
        };

        let prepared = prepare_boot_config(config).unwrap();
        let persisted = std::fs::read_to_string(temporary.path().join("config.toml")).unwrap();
        let persisted: KernelConfig = toml::from_str(&persisted).unwrap();

        assert!(
            captain_types::config::decode_session_secret(&prepared.auth.session_secret).is_some()
        );
        assert_eq!(prepared.auth.session_secret, persisted.auth.session_secret);
        assert_eq!(prepared.auth.session_epoch, persisted.auth.session_epoch);
    }
}
