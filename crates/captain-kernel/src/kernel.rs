//! CaptainKernel — assembles all subsystems and provides the main API.
//!
//! Root contract:
//! - own the long-lived subsystem fields and wire them together at boot;
//! - keep cross-subsystem atomic boundaries such as agent spawn, shutdown,
//!   credential resolution, and the public message entrypoints;
//! - expose compatibility shims for public callers while domain behavior lives
//!   in `kernel_*` modules;
//! - adapt `CaptainKernel` to `KernelHandle` by delegating to
//!   `kernel_handle_*` modules whenever the behavior belongs to a domain.
//!
//! Do not add new domain logic here by default. If a behavior can be tested
//! without constructing the full kernel, it belongs in the owning module.

use crate::auth::AuthManager;
use crate::background::BackgroundExecutor;
use crate::capabilities::CapabilityManager;
use crate::config::load_config;
use crate::error::{KernelError, KernelResult};
use crate::event_bus::EventBus;
use crate::metering::MeteringEngine;
use crate::registry::AgentRegistry;
use crate::scheduler::AgentScheduler;
use crate::supervisor::Supervisor;
use crate::triggers::{FileChangeWatchGuard, TriggerEngine, TriggerId};
use crate::workflow::WorkflowEngine;

use captain_memory::MemorySubstrate;
use captain_runtime::agent_loop::{strip_provider_prefix, AgentLoopResult};
use captain_runtime::audit::AuditLog;
use captain_runtime::drivers;
use captain_runtime::kernel_handle::{self, KernelHandle};
use captain_runtime::llm_driver::LlmDriver;
use captain_runtime::sandbox::WasmSandbox;
use captain_types::agent::*;
#[cfg(test)]
use captain_types::config::AssistantConfig;
use captain_types::config::KernelConfig;
use captain_types::error::CaptainError;
use captain_types::tool::ToolDefinition;

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};
use tracing::{debug, info, warn};

#[path = "kernel_agent_api_provision.rs"]
mod kernel_agent_api_provision;
#[path = "kernel_agent_lifecycle.rs"]
mod kernel_agent_lifecycle;
#[path = "kernel_agent_runtime.rs"]
mod kernel_agent_runtime;
#[cfg(test)]
#[path = "kernel_agent_runtime_tests.rs"]
mod kernel_agent_runtime_tests;
#[path = "kernel_agent_spawn.rs"]
mod kernel_agent_spawn;
#[path = "kernel_agent_turn.rs"]
mod kernel_agent_turn;
#[path = "kernel_agent_turn_observability.rs"]
mod kernel_agent_turn_observability;
#[path = "kernel_agent_workspace.rs"]
mod kernel_agent_workspace;
#[path = "kernel_autonomy_runtime.rs"]
mod kernel_autonomy_runtime;
#[path = "kernel_background_startup.rs"]
mod kernel_background_startup;
#[path = "kernel_boot_default_agent.rs"]
mod kernel_boot_default_agent;
#[path = "kernel_boot_devices.rs"]
mod kernel_boot_devices;
#[path = "kernel_boot_embedding.rs"]
mod kernel_boot_embedding;
#[path = "kernel_boot_foundations.rs"]
mod kernel_boot_foundations;
#[path = "kernel_boot_import_tui_sessions.rs"]
mod kernel_boot_import_tui_sessions;
#[path = "kernel_boot_llm.rs"]
mod kernel_boot_llm;
#[path = "kernel_boot_registries.rs"]
mod kernel_boot_registries;
#[path = "kernel_boot_restore_agents.rs"]
mod kernel_boot_restore_agents;
#[path = "kernel_boot_restore_tool_runs.rs"]
mod kernel_boot_restore_tool_runs;
#[path = "kernel_compaction_runtime.rs"]
mod kernel_compaction_runtime;
#[path = "kernel_config_reload.rs"]
mod kernel_config_reload;
#[path = "kernel_config_support.rs"]
mod kernel_config_support;
#[path = "kernel_cron_runtime.rs"]
mod kernel_cron_runtime;
#[path = "kernel_delivery_runtime.rs"]
mod kernel_delivery_runtime;
#[path = "kernel_delivery_tracker.rs"]
mod kernel_delivery_tracker;
#[path = "kernel_driver_support.rs"]
mod kernel_driver_support;
#[path = "kernel_first_use.rs"]
mod kernel_first_use;
#[path = "kernel_first_use_text.rs"]
mod kernel_first_use_text;
#[path = "kernel_fleet_autoscale.rs"]
mod kernel_fleet_autoscale;
#[path = "kernel_graph_snapshot.rs"]
mod kernel_graph_snapshot;
#[path = "kernel_hand_runtime.rs"]
mod kernel_hand_runtime;
#[path = "kernel_handle_a2a.rs"]
mod kernel_handle_a2a;
#[path = "kernel_handle_agents.rs"]
mod kernel_handle_agents;
#[path = "kernel_handle_approval.rs"]
mod kernel_handle_approval;
#[path = "kernel_handle_automation.rs"]
mod kernel_handle_automation;
#[path = "kernel_handle_channels.rs"]
mod kernel_handle_channels;
#[path = "kernel_handle_config.rs"]
mod kernel_handle_config;
#[path = "kernel_handle_goals.rs"]
mod kernel_handle_goals;
#[path = "kernel_handle_hands.rs"]
mod kernel_handle_hands;
#[path = "kernel_handle_knowledge.rs"]
mod kernel_handle_knowledge;
#[path = "kernel_handle_mcp.rs"]
mod kernel_handle_mcp;
#[path = "kernel_handle_memory.rs"]
mod kernel_handle_memory;
#[path = "kernel_handle_projects.rs"]
mod kernel_handle_projects;
#[path = "kernel_llm_launch.rs"]
mod kernel_llm_launch;
#[path = "kernel_llm_prompt.rs"]
mod kernel_llm_prompt;
#[path = "kernel_llm_routing.rs"]
mod kernel_llm_routing;
#[path = "kernel_llm_runtime.rs"]
mod kernel_llm_runtime;
#[path = "kernel_llm_turn.rs"]
mod kernel_llm_turn;
#[path = "kernel_mcp_runtime.rs"]
mod kernel_mcp_runtime;
#[path = "kernel_memory_bridge.rs"]
mod kernel_memory_bridge;
#[path = "kernel_model_support.rs"]
mod kernel_model_support;
#[path = "kernel_model_switch_config.rs"]
mod kernel_model_switch_config;
#[path = "kernel_model_switch_core.rs"]
mod kernel_model_switch_core;
#[path = "kernel_model_switch_requests.rs"]
mod kernel_model_switch_requests;
#[cfg(test)]
#[path = "kernel_model_switch_tests.rs"]
mod kernel_model_switch_tests;
#[path = "kernel_module_runtime.rs"]
mod kernel_module_runtime;
#[path = "kernel_peer_handle.rs"]
mod kernel_peer_handle;
#[path = "kernel_peer_runtime.rs"]
mod kernel_peer_runtime;
#[path = "kernel_project_prompt.rs"]
mod kernel_project_prompt;
#[path = "kernel_prompt_context.rs"]
mod kernel_prompt_context;
#[cfg(test)]
#[path = "kernel_prompt_context_tests.rs"]
mod kernel_prompt_context_tests;
#[cfg(test)]
#[path = "kernel_prompt_tests.rs"]
mod kernel_prompt_tests;
#[path = "kernel_reflection_support.rs"]
mod kernel_reflection_support;
#[path = "kernel_running_tasks.rs"]
mod kernel_running_tasks;
#[path = "kernel_session_runtime.rs"]
mod kernel_session_runtime;
#[path = "kernel_startup_external.rs"]
mod kernel_startup_external;
#[path = "kernel_startup_runtime.rs"]
mod kernel_startup_runtime;
#[path = "kernel_streaming_runtime.rs"]
mod kernel_streaming_runtime;
#[path = "kernel_streaming_turn.rs"]
mod kernel_streaming_turn;
#[path = "kernel_task_checkpoint.rs"]
mod kernel_task_checkpoint;
#[path = "kernel_task_checkpoint_recovery.rs"]
mod kernel_task_checkpoint_recovery;
#[path = "kernel_tool_filter.rs"]
mod kernel_tool_filter;
#[path = "kernel_tool_runtime.rs"]
mod kernel_tool_runtime;
#[path = "kernel_trigger_runtime.rs"]
mod kernel_trigger_runtime;
#[path = "kernel_usage_runtime.rs"]
mod kernel_usage_runtime;
#[path = "kernel_user_facts.rs"]
mod kernel_user_facts;
#[path = "kernel_workflow_runtime.rs"]
mod kernel_workflow_runtime;
#[path = "kernel_workspace_security.rs"]
mod kernel_workspace_security;

use kernel_agent_runtime::{
    normalize_background_fallbacks_for_provider, normalize_background_model_for_provider,
};
use kernel_boot_default_agent::{ensure_default_captain, validate_boot_agent_routing};
use kernel_boot_devices::build_boot_devices;
use kernel_boot_embedding::build_boot_embedding_driver;
use kernel_boot_foundations::{build_boot_core, prepare_boot_config};
use kernel_boot_import_tui_sessions::import_legacy_tui_sessions;
use kernel_boot_llm::build_boot_llm_driver;
use kernel_boot_registries::build_boot_registries;
use kernel_boot_restore_agents::restore_persisted_agents;
use kernel_boot_restore_tool_runs::restore_persisted_tool_runs;
pub use kernel_delivery_tracker::DeliveryTracker;
#[cfg(test)]
pub(crate) use kernel_fleet_autoscale::worker_tools_for_domain;
use kernel_model_support::{build_default_routing, infer_provider_from_model};
#[cfg(test)]
use kernel_prompt_context::{append_turn_diagnostic_context, assistant_style_context};
#[cfg(test)]
use kernel_prompt_context::{asks_for_error_diagnosis, workspace_prompt_file_has_product_content};
use kernel_prompt_context::{read_identity_file, runtime_binary_fingerprint};
pub use kernel_running_tasks::RunningTaskHandle;
pub use kernel_tool_filter::filter_builtins_for_agent;
use kernel_workspace_security::PRINCIPAL_AGENT_NAME;
pub use kernel_workspace_security::{default_blocked_workspace_paths, shared_memory_agent_id};

pub struct CaptainKernel {
    /// Kernel configuration.
    pub config: KernelConfig,
    /// Agent registry.
    pub registry: AgentRegistry,
    /// Capability manager.
    pub capabilities: CapabilityManager,
    /// Event bus.
    pub event_bus: EventBus,
    /// Agent scheduler.
    pub scheduler: AgentScheduler,
    /// Memory substrate.
    pub memory: Arc<MemorySubstrate>,
    /// Process supervisor.
    pub supervisor: Supervisor,
    /// Workflow engine.
    pub workflows: WorkflowEngine,
    /// Event-driven trigger engine.
    pub triggers: TriggerEngine,
    /// Background agent executor.
    pub background: BackgroundExecutor,
    /// Merkle hash chain audit trail.
    pub audit_log: Arc<AuditLog>,
    /// Cost metering engine.
    pub metering: Arc<MeteringEngine>,
    /// Default LLM driver (from kernel config).
    default_driver: Arc<dyn LlmDriver>,
    /// Whether the daemon booted with a usable LLM driver.
    pub llm_driver_ready: bool,
    /// Last LLM driver initialization error when Captain had to boot with the stub driver.
    pub llm_driver_error: Option<String>,
    /// WASM sandbox engine (shared across all WASM agent executions).
    wasm_sandbox: WasmSandbox,
    /// RBAC authentication manager.
    pub auth: AuthManager,
    /// Model catalog registry (RwLock for auth status refresh from API).
    pub model_catalog: std::sync::RwLock<captain_runtime::model_catalog::ModelCatalog>,
    /// Serializes durable Codex model update state mutations.
    pub(crate) codex_model_update_lock: std::sync::Mutex<()>,
    /// Skill registry for plugin skills (RwLock for hot-reload on install/uninstall).
    pub skill_registry: std::sync::RwLock<captain_skills::registry::SkillRegistry>,
    /// Tracks running agent tasks for cancellation support.
    pub running_tasks: dashmap::DashMap<AgentId, RunningTaskHandle>,
    /// MCP server connections (lazily initialized at start_background_agents).
    pub mcp_connections: Arc<tokio::sync::Mutex<Vec<captain_runtime::mcp::McpConnection>>>,
    /// MCP tool definitions cache (populated after connections are established).
    pub mcp_tools: std::sync::Mutex<Vec<ToolDefinition>>,
    /// A2A task store for tracking task lifecycle.
    pub a2a_task_store: captain_runtime::a2a::A2aTaskStore,
    /// Discovered external A2A agent cards.
    pub a2a_external_agents: std::sync::Mutex<Vec<(String, captain_runtime::a2a::AgentCard)>>,
    /// Web tools context (multi-provider search + SSRF-protected fetch + caching).
    pub web_ctx: captain_runtime::web_search::WebToolsContext,
    /// Browser automation manager (native Chrome DevTools Protocol sessions).
    pub browser_ctx: captain_runtime::browser::BrowserManager,
    /// Media understanding engine (image description, audio transcription).
    pub media_engine: captain_runtime::media_understanding::MediaEngine,
    /// Text-to-speech engine.
    pub tts_engine: captain_runtime::tts::TtsEngine,
    /// Device pairing manager.
    pub pairing: crate::pairing::PairingManager,
    /// Embedding driver for vector similarity search (None = text fallback).
    pub embedding_driver:
        Option<Arc<dyn captain_runtime::embedding::EmbeddingDriver + Send + Sync>>,
    /// Hand registry — curated autonomous capability packages.
    pub hand_registry: captain_hands::registry::HandRegistry,
    /// Credential resolver — vault → dotenv → env var priority chain.
    pub credential_resolver: std::sync::Mutex<captain_extensions::credentials::CredentialResolver>,
    /// Extension/integration registry (bundled MCP templates + install state).
    pub extension_registry: std::sync::RwLock<captain_extensions::registry::IntegrationRegistry>,
    /// Integration health monitor.
    pub extension_health: captain_extensions::health::HealthMonitor,
    /// Effective MCP server list (manual config + extension-installed, merged at boot).
    pub effective_mcp_servers: std::sync::RwLock<Vec<captain_types::config::McpServerConfigEntry>>,
    /// Delivery receipt tracker (bounded LRU, max 10K entries).
    pub delivery_tracker: DeliveryTracker,
    /// Cron job scheduler.
    pub cron_scheduler: crate::cron::CronScheduler,
    /// R.2.1 — long-running goal-driven autopilot store.
    pub goal_store: Arc<crate::goals::GoalStore>,
    /// Execution approval manager.
    pub approval_manager: crate::approval::ApprovalManager,
    /// Agent bindings for multi-account routing (Mutex for runtime add/remove).
    pub bindings: std::sync::Mutex<Vec<captain_types::config::AgentBinding>>,
    /// Broadcast configuration.
    pub broadcast: captain_types::config::BroadcastConfig,
    /// Auto-reply engine.
    pub auto_reply_engine: crate::auto_reply::AutoReplyEngine,
    /// Plugin lifecycle hook registry.
    pub hooks: captain_runtime::hooks::HookRegistry,
    /// Persistent process manager for interactive sessions (REPLs, servers).
    pub process_manager: Arc<captain_runtime::process_manager::ProcessManager>,
    /// OFP peer registry — tracks connected peers (OnceLock for safe init after Arc creation).
    pub peer_registry: OnceLock<captain_wire::PeerRegistry>,
    /// OFP peer node — the local networking node (OnceLock for safe init after Arc creation).
    pub peer_node: OnceLock<Arc<captain_wire::PeerNode>>,
    /// Boot timestamp for uptime calculation.
    pub booted_at: std::time::Instant,
    /// R.1.1 — stable UUID generated once at boot. Used by mDNS peer
    /// discovery to broadcast our identity and detect our own packets.
    pub instance_id: String,
    /// WhatsApp Web gateway child process PID (for shutdown cleanup).
    pub whatsapp_gateway_pid: Arc<std::sync::Mutex<Option<u32>>>,
    /// Channel adapters registered at bridge startup (for proactive `channel_send` tool).
    pub channel_adapters:
        dashmap::DashMap<String, Arc<dyn captain_channels::types::ChannelAdapter>>,
    /// Hot-reloadable default model override (set via config hot-reload, read at agent spawn).
    pub default_model_override:
        std::sync::RwLock<Option<captain_types::config::DefaultModelConfig>>,
    /// Per-agent message locks — serializes LLM calls for the same agent to prevent
    /// session corruption when multiple messages arrive concurrently (e.g. rapid voice
    /// messages via Telegram). Different agents can still run in parallel.
    agent_msg_locks: dashmap::DashMap<AgentId, Arc<tokio::sync::Mutex<()>>>,
    /// Knowledge graph for conversation memory (BM25 recall).
    pub graph_memory: Arc<crate::graph_memory::GraphMemory>,
    /// Live guards for file-change triggers.
    file_watchers: std::sync::Mutex<std::collections::HashMap<TriggerId, FileChangeWatchGuard>>,
    /// Weak self-reference for trigger dispatch (set after Arc wrapping).
    self_handle: OnceLock<Weak<CaptainKernel>>,
    /// Flag to prevent cron cleanup during hand reactivation.
    reactivating_hand: std::sync::atomic::AtomicBool,
    /// Tracks active (in-progress) agent streams for WebSocket catch-up.
    pub active_streams: crate::chat_broadcast::ActiveStreamTracker,
    /// Tool RAG cache — precomputed tool embeddings for semantic retrieval.
    pub tool_embedding_cache: crate::tool_rag::SharedToolCache,
}

/// Captain system prompt — concis, avec les règles essentielles.
/// Le playbook détaillé est dans un fichier séparé chargé à la demande.
const CAPTAIN_SYSTEM_PROMPT: &str = "\
Tu es Captain, l'agent principal de Captain — un Agent OS local.\n\
Tu ES le système. Tu agis, tu ne décris pas tes limites.\n\
Réponds toujours dans la langue du dernier message utilisateur. Si la langue\n\
est ambiguë, utilise la préférence utilisateur configurée; sans préférence,\n\
réponds en anglais.\n\n\
RÈGLES:\n\
- Chaque action = un tool call. Pas de tool = pas d'action.\n\
- Ne dis JAMAIS \"c'est noté\" sans memory_store.\n\
- Ne dis JAMAIS \"je ne peux pas\".\n\
- Rappel → cron_create. État système → shell_exec + agent_list.\n\
- Ne traite pas PLAYBOOK.md comme une source canonique pour t'auto-auditer,\n\
  comparer Captain à un autre agent ou choisir un outil : c'est un support\n\
  legacy de workspace. Utilise l'état live, captain_docs et capability_search.\n\n\
MÉMOIRE (3 outils, 3 usages) — pattern Captain:\n\
- memory_save(subject, predicate, object, category) = ÉCRIS un fait durable.\n\
  Appelle-le SPONTANÉMENT — sans qu'on te le demande — dès que tu détectes :\n\
  (1) une info user qui ressortira plus tard (préférence, contact, contexte),\n\
  (2) un fait durable découvert pendant un workflow (endpoint, commande, convention, quirk d'outil),\n\
  (3) une leçon courte (succès ou échec à reproduire/éviter),\n\
  (4) une solution validée à un problème précis.\n\
  Écris des faits déclaratifs et compacts. Ne stocke pas la procédure complète\n\
  en mémoire : les workflows et modes opératoires vont dans les skills.\n\
  Si l'utilisateur dit 'mémorise X' / 'rappelle-toi Y' / 'note que Z' →\n\
  appelle memory_save TOI-MÊME, ne te contente pas d'acquiescer.\n\
- memory_recall(query) = CHERCHE un fait dans MemPalace. À faire AVANT de\n\
  répondre quand le user pose une question sur ses infos perso/préférences.\n\
- memory_store(key, value) = paire clé/valeur simple, pour de l'état\n\
  temporaire ou un cache de ta propre logique. Préfère memory_save pour\n\
  les faits durables.\n\
- Confidentialité mémoire : utilise les souvenirs en contexte silencieux.\n\
  Ne liste jamais des détails personnels juste pour prouver que tu te\n\
  souviens. Ne révèle un souvenir personnel que si l'utilisateur le demande\n\
  explicitement, parle de ce sujet précis, ou si la tâche l'exige vraiment.\n\
- memory_forget(subject?, predicate?, object?) = SUPPRIME un fait que\n\
  tu avais stocké à tort. Appelle-le SPONTANÉMENT quand l'utilisateur\n\
  dit 'oublie ça', 'tu te trompes', 'corrige ce que tu sais sur X',\n\
  'ce n'est plus vrai'. Au moins UN filtre requis (anti-wipe). Les\n\
  filtres acceptent les wildcards SQL LIKE (% = n'importe quoi).\n\
  EXEMPLE : memory_forget({object:'%ancienne_valeur%'}) supprime tout fait\n\
  mentionnant cette valeur. Ne demande pas confirmation : agis, le user\n\
  voit le nombre de lignes supprimées en retour. Appelle memory_forget\n\
  d'abord, puis confirme brièvement sans réexposer de détails inutiles.\n\
- Filet de sécurité : un job background (Haiku) scanne aussi les tours\n\
  et capture ce que tu as oublié. C'est un FILET, pas un substitut.\n\
- Pour LIRE la mémoire structurée : mempalace_kg_query, mempalace_search,\n\
  mempalace_diary_read.\n\n\
ACCÈS DISTANTS:\n\
- Serveur distant → ssh_exec({key_name, command}) direct si l'alias est connu ou déductible depuis les infos utilisateur.\n\
- Le runtime SSH résout l'alias exact, un raccourci non ambigu, ou la clé SSH par défaut pour les demandes génériques.\n\
- Avant de demander l'IP/host à l'user, tente d'abord session_recall / memory_recall / config_read / knowledge_query puis ssh_exec.\n\
- Ne lance pas `captain ssh list` via shell_exec pour diagnostiquer SSH : shell_exec a un environnement sandboxé. Utilise les tools SSH natifs et captain_docs(family=\"ssh\").\n\n\
CHERCHE AVANT DE DEMANDER:\n\
- N'appelle pas capability_search par réflexe pour chaque tour : ça économise moins que ça ne coûte.\n\
- Appel direct si le bon outil est déjà CORE/visible et évident (captain_docs, memory_recall, session_recall, system_time, ask_user).\n\
- Appelle capability_search dès qu'une demande actionnable nécessite une capacité de domaine absente du CORE visible (fichier, shell, SSH, web, navigateur, image, canal, config, secret, projet, scheduling, agent...), quand deux rails se ressemblent, ou avant de conclure \"je n'ai pas accès\".\n\
- Nom familier ('mon serveur', 'la machine de prod') → essaye d'abord session_recall / memory_recall / config_read / knowledge_query.\n\
- Si tu as les tools, utilise-les. Demande seulement après échec.\n\n\
ÉCONOMIE DE TOKENS (obligatoire):\n\
- Réponds court. Pas de récap, pas de listes inutiles.\n\
- 1 tool call quand possible, pas 3 séquentiels.\n\
- Ne fais PAS de memory_recall si l'info est déjà dans le contexte.\n\
- Questions simples = réponse directe, SANS tool call.\n\
- Délègue seulement quand une sous-tâche est indépendante, longue ou parallélisable avec un budget clair ; sinon fais-la toi-même pour éviter une explosion de contexte.\n\n\
ORCHESTRATION:\n\
- Tu es le chef d'orchestre. Tu délègues, surveilles, corriges.\n\
- agent_status(id) = voir ce que fait un agent.\n\
- agent_watch(id) = voir ses derniers events.\n\
- agent_delegate(id, task, max_tokens) = assigner une tâche avec budget.\n\
- agent_correct(id, message) = corriger un agent en live.\n\n\
EXTENSIBILITÉ — tu peux te rendre plus capable:\n\
- Quand tu résous un WORKFLOW RÉPÉTABLE (suite d'API, check récurrent,\n\
  procédure manuelle, debug, convention projet, commande CLI), capitalise-le\n\
  au lieu de juste répondre. Commence par skill_search/capability_search pour\n\
  éviter un doublon. Si un skill existe déjà, propose un skill_refinement_propose\n\
  avec snapshot; sinon propose ou scaffold un skill. Le skill doit contenir\n\
  conditions de déclenchement, étapes numérotées, commandes/API exactes si\n\
  connues, pièges et vérification.\n\
- N'attends pas forcément 5 répétitions : une découverte non triviale et\n\
  réutilisable (nouvel endpoint documenté, route d'outil fiable, recovery\n\
  validé) mérite au minimum mémoire déclarative + proposition de skill ou de\n\
  refinement selon le cas.\n\
- Quand une INTÉGRATION TIERCE manque (Slack, Notion, monitoring,\n\
  paiement), propose un scaffold_skill plutôt que d'abandonner.\n\
- Si une CAPABILITY peut servir à d'autres agents, capture-la — tu fais\n\
  grandir Captain à chaque skill créé.\n\
- Auto-amélioration contrôlée : après une tâche longue/tool-heavy, un\n\
  échec répété ou un `Security blocked`, appelle self_improvement_review\n\
  pour inspecter les learnings, bugs système et proposals. Si tu détectes\n\
  un défaut reproductible de Captain, enregistre-le via system_bug_report\n\
  avec une description générique sans secrets ni noms privés. Apprentissage non critique :\n\
  memory_save est autorisé et doit produire un feedback chat 🧠.\n\
  Changement critique (skill, config, goal, routing, prompt, comportement\n\
  global) : rends la proposition visible, puis attends approbation explicite\n\
  via learning_review_decide, skill_proposal_decide ou ask_user avant de\n\
  muter durablement. Quand un apprentissage change ton comportement futur,\n\
  dis explicitement à l'utilisateur ce qui change et comment tu agiras la\n\
  prochaine fois. Si la préférence est ambiguë, pose une question courte avant\n\
  de mémoriser ou d'appliquer la règle.\n\n\
  Après chaque usage réel d'un skill, fais un contrôle rapide et proactif :\n\
  si le modèle détecte une amélioration réutilisable (précondition, recovery,\n\
  routage d'outil, doc, version), crée sans attendre une proposition via\n\
  skill_refinement_propose (snapshot automatique + version actuelle/cible si\n\
  connue). Après approbation, patche le skill minimalement, teste, puis marque\n\
  le raffinement applied via skill_refinement_update.\n\n\
  Si une demande nécessite un serveur MCP non connecté, ne pars pas à l'aveugle :\n\
  appelle capability_search puis captain_docs({\"family\":\"mcp\",\"query\":\"install\"}).\n\
  Préfère les intégrations packagées; sinon configure [[mcp_servers]] avec\n\
  secrets en vault, vérifie que les tools mcp_* apparaissent, et reporte une\n\
  lacune générique via system_bug_report si le flow doit être consolidé.\n\n\
MÉMOIRE LONGUE — sessions précédentes:\n\
- Tu as accès aux conversations passées via des résumés structurés\n\
  (checkpoint.md, 5 sections : Sujets / Décisions / Erreurs / Réussites /\n\
  Infos durables) générés automatiquement par un job Haiku quand une\n\
  session devient inactive ≥ 10 min.\n\
- Quand l'utilisateur dit 'on avait dit', 'l'autre fois', 'tu m'avais\n\
  dit que', 'rappelle-moi ce qu'on a fait sur X' — appelle\n\
  session_recall({query}) AVANT de répondre. Ne lui fais jamais\n\
  répéter une info qui dort dans une session précédente.\n\
- session_recall fouille les checkpoints, pas les transcripts bruts —\n\
  c'est rapide et ciblé. Pour aller plus profond, ouvre le JSON brut\n\
  via file_read sur le path retourné.\n\n\
WORKSPACE (filesystem):\n\
- Tu as accès LIBRE à ~/.captain/ : sessions, MemPalace, config.toml,\n\
  agents/, docs et runtime state. Utilise file_read / file_list / glob /\n\
  grep / file_write directement, pas besoin de demander.\n\
- Exception volontaire : les fichiers de credentials bruts (.env,\n\
  secrets.env, secrets-backups/, vault.enc) passent par les rails typés\n\
  secret_read / secret_write / config_setup / ssh_* / intégrations, pas par\n\
  file_read ou file_write.\n\
- Pour étendre l'accès à un autre dossier (projet, repo, dossier\n\
  utilisateur), appelle workspace_add({path}). L'autorisation persiste\n\
  dans config.toml [workspace] extra_paths.\n\
- Bloqué même pour toi : ~/.ssh/, ~/.gnupg/. Tente pas, l'erreur sera\n\
  explicite. Pour les credentials, passe par secret_read / secret_write.\n\n\
RTFM — TON PROPRE MANUEL:\n\
- Avant de demander à l'utilisateur comment fonctionne un outil, ses\n\
  paramètres, ses limites ou la différence entre deux outils similaires,\n\
  consulte ta propre doc via `captain_docs(query, family?)`. Ex:\n\
  `captain_docs({\"query\":\"edit_file fallback\"})` ou\n\
  `captain_docs({\"query\":\"...\",\"family\":\"channel\"})`.\n\
- Familles disponibles : file, shell-process, network, browser, ssh,\n\
  memory, skill, channel, agent-coordination, scheduling, config-secret,\n\
  knowledge, session-workspace, meta.\n\
- L'ORDRE DE RECHERCHE quand tu hésites :\n\
  1. session_recall (ce qui a déjà été dit)\n\
  2. memory_recall (ce que tu sais)\n\
  3. capability_search (choisir outil / skill / MCP / Hand / doc si la capacité n'est pas évidente)\n\
  4. captain_docs (lire le manuel du rail choisi)\n\
  5. tool_search (schema exact d'un builtin différé)\n\
  6. ask_user (en dernier recours).\n\
- Après un échec d'outil que tu ne comprends pas : lis le message d'erreur,\n\
  appelle captain_docs sur la famille concernée, puis tente une correction\n\
  raisonnable. Ne conclus pas trop vite que c'est impossible.\n\
- Tu n'as PAS le droit d'inventer le comportement d'un outil. Si\n\
  captain_docs ne renvoie rien, c'est que l'outil n'existe pas\n\
  (corrige l'orthographe ou pivote) — pas que tu peux extrapoler.\n\n\
	DÉCOUVERTE DE CAPACITÉS — capability_search puis tool_search:\n\
	- Tu vois en permanence un CORE volontairement petit : capability_search,\n\
	  tool_search, captain_docs, ask_user, memory_save, memory_recall,\n\
	  session_recall, system_time. Les outils de domaine (file_*, shell_exec,\n\
	  ssh_*, web_*, browser_*, image_*, channel_*, agent_*, config_*, secret_*,\n\
	  model_switch_*, project_*, scheduling, knowledge_add_*, etc.) sont\n\
	  différés pour économiser le contexte, mais ils restent appelables après\n\
	  découverte.\n\
	- Au moindre doute NON TRIVIAL sur la bonne capacité, avant de dire\n\
	  \"je n'ai pas accès\", avant de demander à l'utilisateur quel outil utiliser,\n\
	  et avant d'inventer un comportement : appelle `capability_search`.\n\
	  Ne l'appelle pas pour du bavardage simple ni quand la réponse ne nécessite\n\
	  clairement aucun outil.\n\
- Workflow : (1) tu identifies le besoin (\"parler à voix haute\",\n\
  \"naviguer une page web\", \"ajouter une entité dans le graph\"),\n\
  (2) `capability_search({\"query\":\"voix haute\"})` te retourne des\n\
  candidats builtin / skill / MCP / Hand / docs avec usage recommandé,\n\
  (3) si le candidat est un builtin différé et que tu dois forcer le\n\
  schema exact, utilise `tool_search({\"query\":\"select:<nom>\"})`,\n\
  (4) tu appelles le tool ou lis la doc indiquée.\n\
- Pour récupérer un nom EXACT que tu connais déjà : prefixe la query\n\
  avec `select:` → `tool_search({\"query\":\"select:text_to_speech\"})`\n\
  (supporte `select:n1,n2` pour plusieurs noms d'un coup).\n\
	- INTERDIT : répondre \"je n'ai pas accès à X\" pour un X qui n'est pas\n\
	  dans tes CORE. Ce n'est pas vrai — appelle d'abord `capability_search`.\n\
  Captain n'a aucune restriction d'exécution (manifest sans\n\
  tool_allowlist) ; tout outil retourné par capability_search/tool_search\n\
  est appelable si sa source est disponible.\n\
- captain_docs vs capability_search vs tool_search : captain_docs lit la\n\
  prose de `docs/captain-tools/` ; capability_search choisit la bonne\n\
  capacité entre outils, skills, MCP, Hands et docs ; tool_search retourne\n\
  le SCHEMA d'un builtin absent du prompt courant.\n\n\
SÉCURITÉ — contraintes invisibles à connaître:\n\
- **env_clear sur tes subprocess** : execute_code / shell_exec / process_start /\n\
  skill_execute reçoivent un env minimal (PATH, HOME, TMPDIR/TMP/TEMP,\n\
  LANG/LC_ALL, TERM seulement ; pas tes variables de shell complètes).\n\
  Tes API keys (OPENAI_API_KEY, GROQ_API_KEY, …) ne fuient PAS dans les snippets.\n\
  Un script Python qui fait `os.environ['OPENAI_API_KEY']` retourne vide.\n\
  Ne passe jamais un secret brut a un snippet, a un script, a une commande,\n\
  a un patch ou a un message. Si un outil bloque avec `Security blocked`,\n\
  change de methode: `secret_write` pour stocker, `secret_read` seulement\n\
  pour verifier la presence masquee, puis integration native ou skill avec\n\
  `[requirements.env_inject]` pour injecter la valeur du vault au runtime.\n\
  Le code genere peut referencer `GEMINI_API_KEY` / `OPENAI_API_KEY`, jamais\n\
  coller la valeur reelle.\n\
  Pour diagnostiquer les secrets, le vault ou SSH, n'utilise pas shell_exec :\n\
  passe par les tools natifs et captain_docs, sinon tu risques un faux négatif.\n\
- **env_inject pour les skills** : un skill ne voit que les secrets listés dans\n\
  son SKILL.toml `[requirements.env_inject]`. Sans cette déclaration, secrets.env\n\
  reste invisible côté skill — le scaffold_skill doit ajouter env_inject quand\n\
  le skill a besoin d'un credential.\n\
- **tool_allowlist strict (TS.3)** : si un worker a `tool_allowlist = [...]`\n\
  dans son manifest, tout tool hors liste est refusé à l'exécution\n\
  (priorité sur capabilities.tools). Sans tool_allowlist, le fallback\n\
  est `capabilities.tools` ; sans les deux (ou avec `[\"*\"]`) → bypass\n\
  total comme toi-même Captain. Pour spawner un worker contraint :\n\
  `tool_allowlist = [\"file_read\", \"web_fetch\", \"document_extract\"]` dans son manifest.\n\
- **api_key obligatoire pour bind non-loopback** : si tu changes `listen_addr`\n\
  pour `0.0.0.0:...` ou une IP publique, le daemon refuse de démarrer sans\n\
  `CAPTAIN_DAEMON_API_KEY` ou `CAPTAIN_API_KEY` dans secrets.env/env. Ne pas\n\
  écrire de clé API directement dans config.toml. Loopback (127.0.0.1, [::1])\n\
  reste permissif.\n\
- **allowed_users sur les canaux = empty deny all** : éditer\n\
  `[channels.telegram]` sans déclarer `allowed_users` rend le bot muet pour\n\
  tout le monde. Pour ouvrir au public : `allowed_users = [\"*\"]` explicite.\n\
  Pour restreindre : liste des platform_id.\n\
- **Tes agents Chrome ont des profils isolés** : chaque agent a son propre\n\
  --user-data-dir sous ~/.captain/browser-profiles/. Cookies/logins persistent\n\
  par agent, mais pas entre agents. Le navigateur de l'utilisateur n'est jamais\n\
  touché.\n\
Pour les détails complets de chaque contrainte, appelle\n\
`captain_docs({\"family\":\"shell-process\"})`, `\"skill\"`, `\"channel\"`, etc.\n\n\
CANAUX (Telegram, Discord, Slack, Matrix…) — rotation à chaud:\n\
- Tu édites toi-même la config d'un canal dans config.toml (bot_token_env,\n\
  allowed_users, default_chat_id, intents…) via file_write.\n\
- APRÈS chaque édition, appelle channel_reconfigure({channel}) pour\n\
  re-spawner UNIQUEMENT cet adapter avec la config fraîche. Les autres\n\
  canaux restent connectés. Ne redémarre PAS le daemon entier — tu\n\
  couperais Discord/Slack pour rien.\n\
- Trigger phrases : 'change le bot Telegram', 'mets ce nouveau token',\n\
  'reconnecte Discord avec ces paramètres', 'rotation du token X'.\n\
  Quand tu les reconnais : (1) file_write config.toml, (2)\n\
  channel_reconfigure({channel}), (3) confirme à l'utilisateur.\n\
- Si le nom de canal ne correspond à aucune section [channels.*] live,\n\
  l'outil renverra l'erreur avec la liste des noms valides — relis\n\
  config.toml et corrige avant de relancer.\n\
";

impl CaptainKernel {
    fn memory_retractions_for_prompt(
        &self,
    ) -> Vec<captain_runtime::memory_retractions::MemoryRetraction> {
        captain_runtime::memory_retractions::load_retractions(
            self.memory
                .structured_get(
                    shared_memory_agent_id(),
                    captain_runtime::memory_retractions::MEMORY_RETRACTIONS_KEY,
                )
                .ok()
                .flatten(),
        )
    }

    fn runtime_update_notice(&self) -> Option<String> {
        const KEY: &str = "__captain_runtime_fingerprint_v1";
        let current = runtime_binary_fingerprint();
        let shared_id = shared_memory_agent_id();
        let previous = self
            .memory
            .structured_get(shared_id, KEY)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_owned));

        if previous.as_deref() == Some(current.as_str()) {
            return None;
        }

        let _ = self
            .memory
            .structured_set(shared_id, KEY, serde_json::Value::String(current));

        Some(
            "## Mise a jour runtime reelle\n\
             Captain vient de demarrer sur un binaire ou un jeu de capacites different du dernier fingerprint connu. \
             Tu sais seulement qu'une vraie mise a jour a eu lieu: ne pretends jamais avoir lu le changelog, les commits ou les details exacts sans verification par outil. \
             Si l'utilisateur demande ce qui a change, commence par lire captain_docs avec family=\"runtime-changelog\" puis verifie les schemas d'outils avec capability_search et les familles captain_docs concernees. \
             Ne deduis pas les changements depuis git log, d'anciens resumes, des chemins locaux, ou des hypotheses. \
             Cite uniquement les capacites confirmees par le runtime courant. \
             Si une ancienne hypothese contredit le runtime actuel, ignore l'ancienne hypothese et utilise le nouveau fonctionnement immediatement."
                .to_string(),
        )
    }

    /// Boot the kernel with configuration from the given path.
    pub fn boot(config_path: Option<&Path>) -> KernelResult<Self> {
        let config = load_config(config_path);
        Self::boot_with_config(config)
    }

    /// Boot the kernel with an explicit configuration.
    pub fn boot_with_config(config: KernelConfig) -> KernelResult<Self> {
        let mut config = prepare_boot_config(config)?;
        let boot_core = build_boot_core(&config)?;

        let boot_driver = build_boot_llm_driver(&mut config, &boot_core.credential_resolver);
        let driver = boot_driver.driver;
        let llm_driver_ready = boot_driver.ready;
        let llm_driver_error = boot_driver.error;

        let boot_registries = build_boot_registries(&config);
        let skill_registry = boot_registries.skill_registry;
        let hand_registry = boot_registries.hand_registry;
        let extension_registry = boot_registries.extension_registry;
        let extension_health = boot_registries.extension_health;
        let all_mcp_servers = boot_registries.all_mcp_servers;

        let embedding_driver = build_boot_embedding_driver(&config);

        let boot_devices = build_boot_devices(&config, &boot_core.memory);
        let browser_ctx = boot_devices.browser_ctx;
        let media_engine = boot_devices.media_engine;
        let tts_engine = boot_devices.tts_engine;
        let pairing = boot_devices.pairing;

        let kernel = Self {
            config,
            registry: AgentRegistry::new(),
            capabilities: CapabilityManager::new(),
            event_bus: EventBus::new(),
            scheduler: AgentScheduler::new(),
            memory: boot_core.memory.clone(),
            supervisor: boot_core.supervisor,
            workflows: WorkflowEngine::new(),
            triggers: boot_core.triggers,
            background: boot_core.background,
            audit_log: boot_core.audit_log,
            metering: boot_core.metering,
            default_driver: driver,
            llm_driver_ready,
            llm_driver_error,
            wasm_sandbox: boot_core.wasm_sandbox,
            auth: boot_core.auth,
            model_catalog: std::sync::RwLock::new(boot_core.model_catalog),
            codex_model_update_lock: std::sync::Mutex::new(()),
            skill_registry: std::sync::RwLock::new(skill_registry),
            running_tasks: dashmap::DashMap::new(),
            mcp_connections: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            mcp_tools: std::sync::Mutex::new(Vec::new()),
            a2a_task_store: captain_runtime::a2a::A2aTaskStore::default(),
            a2a_external_agents: std::sync::Mutex::new(Vec::new()),
            web_ctx: boot_core.web_ctx,
            browser_ctx,
            media_engine,
            tts_engine,
            pairing,
            embedding_driver,
            hand_registry,
            credential_resolver: std::sync::Mutex::new(boot_core.credential_resolver),
            extension_registry: std::sync::RwLock::new(extension_registry),
            extension_health,
            effective_mcp_servers: std::sync::RwLock::new(all_mcp_servers),
            delivery_tracker: DeliveryTracker::new(),
            cron_scheduler: boot_core.cron_scheduler,
            goal_store: boot_core.goal_store,
            approval_manager: boot_core.approval_manager,
            bindings: std::sync::Mutex::new(boot_core.bindings),
            broadcast: boot_core.broadcast,
            auto_reply_engine: boot_core.auto_reply_engine,
            hooks: captain_runtime::hooks::HookRegistry::new(),
            process_manager: boot_core.process_manager,
            peer_registry: OnceLock::new(),
            peer_node: OnceLock::new(),
            booted_at: std::time::Instant::now(),
            instance_id: uuid::Uuid::new_v4().to_string(),
            whatsapp_gateway_pid: Arc::new(std::sync::Mutex::new(None)),
            channel_adapters: dashmap::DashMap::new(),
            default_model_override: std::sync::RwLock::new(None),
            agent_msg_locks: dashmap::DashMap::new(),
            graph_memory: boot_core.graph_memory,
            file_watchers: std::sync::Mutex::new(std::collections::HashMap::new()),
            self_handle: OnceLock::new(),
            reactivating_hand: std::sync::atomic::AtomicBool::new(false),
            active_streams: crate::chat_broadcast::ActiveStreamTracker::new(),
            tool_embedding_cache: Arc::new(tokio::sync::RwLock::new(None)),
        };

        restore_persisted_agents(&kernel);
        restore_persisted_tool_runs(&kernel);
        ensure_default_captain(&kernel);
        import_legacy_tui_sessions(&kernel);
        validate_boot_agent_routing(&kernel);

        info!("Captain kernel booted successfully");
        Ok(kernel)
    }

    /// Check if the target agent can handle the input. If not, find or spawn
    /// a capable agent and return its ID + entry instead.
    /// Check if the target agent can handle the input. If not, delegate to a
    /// specialized agent in the background and inject its analysis as context.
    ///
    /// The original agent (e.g., Captain) stays the respondent — the user only
    /// talks to Captain. The specialized agent works behind the scenes.
    ///
    /// Returns: (message, content_blocks) — possibly enriched with specialist output
    /// and with capability-specific blocks (e.g., images) stripped.
    pub async fn resolve_capability_gap(
        &self,
        agent_id: AgentId,
        message: &str,
        content_blocks: Option<Vec<captain_types::message::ContentBlock>>,
    ) -> KernelResult<(String, Option<Vec<captain_types::message::ContentBlock>>)> {
        use crate::capability_routing::{decide_routing, RoutingDecision};

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let decision = {
            let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
            decide_routing(
                &self.registry,
                &catalog,
                agent_id,
                &entry.manifest.model.model,
                content_blocks.as_deref(),
            )
        };

        let target_id = match decision {
            RoutingDecision::Proceed => return Ok((message.to_string(), content_blocks)),
            RoutingDecision::NoCandidateAvailable(cap) => {
                warn!(capability = ?cap, "No capable model available, proceeding without");
                return Ok((message.to_string(), content_blocks));
            }
            RoutingDecision::DelegateTo(target_id) => {
                info!(agent_id = %agent_id, target = %target_id, "Delegating to specialist agent");
                target_id
            }
            RoutingDecision::SpawnAndDelegate {
                manifest_toml,
                capability,
            } => {
                info!(capability = ?capability, "Auto-spawning specialist agent");
                let manifest: AgentManifest = toml::from_str(&manifest_toml).map_err(|e| {
                    KernelError::Captain(CaptainError::ManifestParse(format!("Vision agent: {e}")))
                })?;
                let new_name = manifest.name.clone();
                let new_id = self.spawn_agent(manifest)?;
                info!(agent_id = %new_id, name = new_name, "Specialist agent spawned");
                new_id
            }
        };

        // Call the specialist agent with the full content (including images)
        let specialist_name = self
            .registry
            .get(target_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "specialist".to_string());
        info!(
            specialist = specialist_name,
            "Calling specialist for analysis"
        );

        let specialist_result = self
            .send_message_with_blocks(
                target_id,
                message,
                content_blocks.clone().unwrap_or_default(),
            )
            .await;

        match specialist_result {
            Ok(result) => {
                // Strip images from blocks (Captain's model can't process them)
                let cleaned_blocks = content_blocks
                    .map(|blocks| {
                        blocks
                            .into_iter()
                            .filter(|b| {
                                !matches!(b, captain_types::message::ContentBlock::Image { .. })
                            })
                            .collect::<Vec<_>>()
                    })
                    .filter(|b: &Vec<_>| !b.is_empty());

                // Enrich Captain's message with the specialist's analysis
                let enriched = format!(
                    "{message}\n\n\
                     [Note: I delegated image analysis to my specialist agent \"{specialist_name}\". \
                     Here is the raw analysis — reformulate it naturally in your response and mention \
                     briefly that you used a specialist agent for the image.]\n\
                     ---\n{}\n---",
                    result.response
                );

                info!(
                    specialist = specialist_name,
                    "Specialist analysis injected into Captain's context"
                );
                Ok((enriched, cleaned_blocks))
            }
            Err(e) => {
                warn!(specialist = specialist_name, error = %e, "Specialist agent failed, proceeding without");
                Ok((message.to_string(), content_blocks))
            }
        }
    }

    /// Verify a signed manifest envelope (Ed25519 + SHA-256).
    ///
    /// Call this before `spawn_agent` when a `SignedManifest` JSON is provided
    /// alongside the TOML. Returns the verified manifest TOML string on success.
    pub fn verify_signed_manifest(&self, signed_json: &str) -> KernelResult<String> {
        let signed: captain_types::manifest_signing::SignedManifest =
            serde_json::from_str(signed_json).map_err(|e| {
                KernelError::Captain(captain_types::error::CaptainError::Config(format!(
                    "Invalid signed manifest JSON: {e}"
                )))
            })?;
        signed.verify().map_err(|e| {
            KernelError::Captain(captain_types::error::CaptainError::Config(format!(
                "Manifest signature verification failed: {e}"
            )))
        })?;
        info!(signer = %signed.signer_id, hash = %signed.content_hash, "Signed manifest verified");
        Ok(signed.manifest)
    }

    /// Send a message to an agent and get a response.
    ///
    /// Automatically upgrades the kernel handle from `self_handle` so that
    /// agent turns triggered by cron, channels, events, or inter-agent calls
    /// have full access to kernel tools (cron_create, agent_send, etc.).
    pub async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> KernelResult<AgentLoopResult> {
        let handle: Option<Arc<dyn KernelHandle>> = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .map(|arc| arc as Arc<dyn KernelHandle>);
        self.send_message_with_handle(agent_id, message, handle, None, None)
            .await
    }

    /// Send a multimodal message (text + images) to an agent and get a response.
    ///
    /// Used by channel bridges when a user sends a photo — the image is downloaded,
    /// base64 encoded, and passed as `ContentBlock::Image` alongside any caption text.
    pub async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<captain_types::message::ContentBlock>,
    ) -> KernelResult<AgentLoopResult> {
        let handle: Option<Arc<dyn KernelHandle>> = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .map(|arc| arc as Arc<dyn KernelHandle>);
        self.send_message_with_handle_and_blocks(
            agent_id,
            message,
            handle,
            Some(blocks),
            None,
            None,
        )
        .await
    }

    /// Send a message with an optional kernel handle for inter-agent tools.
    pub async fn send_message_with_handle(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_with_handle_and_blocks(
            agent_id,
            message,
            kernel_handle,
            None,
            sender_id,
            sender_name,
        )
        .await
    }

    /// Send a message with optional content blocks and an optional kernel handle.
    ///
    /// When `content_blocks` is `Some`, the LLM agent loop receives structured
    /// multimodal content (text + images) instead of just a text string. This
    /// enables vision models to process images sent from channels like Telegram.
    ///
    /// Per-agent locking ensures that concurrent messages for the same agent
    /// are serialized (preventing session corruption), while messages for
    /// different agents run in parallel.
    pub async fn send_message_with_handle_and_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        content_blocks: Option<Vec<captain_types::message::ContentBlock>>,
        sender_id: Option<String>,
        sender_name: Option<String>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full(
            agent_id,
            message,
            kernel_handle,
            content_blocks,
            sender_id,
            sender_name,
            None,
        )
        .await
    }

    /// Resolve a module path relative to the kernel's home directory.
    ///
    /// If the path is absolute, return it as-is. Otherwise, resolve relative
    /// to `config.home_dir`.
    fn resolve_module_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.config.home_dir.join(path)
        }
    }

    /// Switch an agent's model.
    ///
    /// When `explicit_provider` is `Some`, that provider name is used as-is
    /// (respecting the user's custom configuration). When `None`, the provider
    /// is auto-detected from the model catalog or inferred from the model name,
    /// but only if the agent does NOT have a custom `base_url` configured.
    /// Agents with a custom `base_url` keep their current provider unless
    /// overridden explicitly — this prevents custom setups (e.g. Tencent,
    /// Azure, or other third-party endpoints) from being misidentified.
    pub fn set_agent_model(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
    ) -> KernelResult<()> {
        let provider = if let Some(ep) = explicit_provider {
            // User explicitly set the provider — use it as-is
            Some(ep.to_string())
        } else {
            // Check whether the agent has a custom base_url, which indicates
            // a user-configured provider endpoint. In that case, preserve the
            // current provider name instead of overriding it with auto-detection.
            let has_custom_url = self
                .registry
                .get(agent_id)
                .map(|e| e.manifest.model.base_url.is_some())
                .unwrap_or(false);
            if has_custom_url {
                // Keep the current provider — don't let auto-detection override
                // a deliberately configured custom endpoint.
                None
            } else {
                // No custom base_url: safe to auto-detect from catalog / model name
                let resolved_provider = self.model_catalog.read().ok().and_then(|catalog| {
                    catalog
                        .find_model(model)
                        .map(|entry| entry.provider.clone())
                });
                resolved_provider.or_else(|| infer_provider_from_model(model))
            }
        };

        // Strip the provider prefix from the model name (e.g. "openrouter/deepseek/deepseek-chat" → "deepseek/deepseek-chat")
        let normalized_model = if let Some(ref prov) = provider {
            strip_provider_prefix(model, prov)
        } else {
            model.to_string()
        };

        if let Some(provider) = provider {
            self.registry
                .update_model_and_provider(agent_id, normalized_model.clone(), provider.clone())
                .map_err(KernelError::Captain)?;
            info!(agent_id = %agent_id, model = %normalized_model, provider = %provider, "Agent model+provider updated");
            // Routing was generated for the old model family — regenerate for the new one
            // so we don't keep routing Qwen requests to mimo, etc.
            let new_routing = build_default_routing(&provider, &normalized_model);
            let _ = self.registry.update_routing(agent_id, new_routing);
        } else {
            self.registry
                .update_model(agent_id, normalized_model.clone())
                .map_err(KernelError::Captain)?;
            info!(agent_id = %agent_id, model = %normalized_model, "Agent model updated (provider unchanged)");
            let prov = self
                .registry
                .get(agent_id)
                .map(|e| e.manifest.model.provider.clone())
                .unwrap_or_default();
            let new_routing = build_default_routing(&prov, &normalized_model);
            let _ = self.registry.update_routing(agent_id, new_routing);
        }

        // Persist the updated entry
        if let Some(entry) = self.registry.get(agent_id) {
            let _ = self.memory.save_agent(&entry);
        }

        // Clear canonical session to prevent memory poisoning from old model's responses
        let _ = self.memory.delete_canonical_session(agent_id);
        debug!(agent_id = %agent_id, "Cleared canonical session after model switch");

        Ok(())
    }

    fn empty_agent_loop_result(response: String) -> AgentLoopResult {
        AgentLoopResult {
            response,
            total_usage: captain_types::message::TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                cached_input_tokens: 0,
                cache_creation_tokens: 0,
            },
            iterations: 0,
            cost_usd: Some(0.0),
            silent: false,
            directives: captain_types::message::ReplyDirectives {
                reply_to: None,
                current_thread: false,
                silent: false,
            },
            tool_calls: Vec::new(),
        }
    }

    /// Update an agent's skill allowlist. Empty = all skills (backward compat).
    pub fn set_agent_skills(&self, agent_id: AgentId, skills: Vec<String>) -> KernelResult<()> {
        // Validate skill names if allowlist is non-empty
        if !skills.is_empty() {
            let registry = self
                .skill_registry
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let known = registry.skill_names();
            for name in &skills {
                if !known.contains(name) {
                    return Err(KernelError::Captain(CaptainError::Internal(format!(
                        "Unknown skill: {name}"
                    ))));
                }
            }
        }

        self.registry
            .update_skills(agent_id, skills.clone())
            .map_err(KernelError::Captain)?;

        if let Some(entry) = self.registry.get(agent_id) {
            let _ = self.memory.save_agent(&entry);
        }

        info!(agent_id = %agent_id, skills = ?skills, "Agent skills updated");
        Ok(())
    }

    /// Update an agent's MCP server allowlist. Empty = all servers (backward compat).
    pub fn set_agent_mcp_servers(
        &self,
        agent_id: AgentId,
        servers: Vec<String>,
    ) -> KernelResult<()> {
        // Validate server names if allowlist is non-empty
        if !servers.is_empty() {
            if let Ok(mcp_tools) = self.mcp_tools.lock() {
                let mut known_servers: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for tool in mcp_tools.iter() {
                    if let Some(s) = captain_runtime::mcp::extract_mcp_server(&tool.name) {
                        known_servers.insert(s.to_string());
                    }
                }
                for name in &servers {
                    let normalized = captain_runtime::mcp::normalize_name(name);
                    if !known_servers.contains(&normalized) {
                        return Err(KernelError::Captain(CaptainError::Internal(format!(
                            "Unknown MCP server: {name}"
                        ))));
                    }
                }
            }
        }

        self.registry
            .update_mcp_servers(agent_id, servers.clone())
            .map_err(KernelError::Captain)?;

        if let Some(entry) = self.registry.get(agent_id) {
            let _ = self.memory.save_agent(&entry);
        }

        info!(agent_id = %agent_id, servers = ?servers, "Agent MCP servers updated");
        Ok(())
    }

    /// Update an agent's tool allowlist and/or blocklist.
    pub fn set_agent_tool_filters(
        &self,
        agent_id: AgentId,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> KernelResult<()> {
        self.registry
            .update_tool_filters(agent_id, allowlist.clone(), blocklist.clone())
            .map_err(KernelError::Captain)?;

        if let Some(entry) = self.registry.get(agent_id) {
            let _ = self.memory.save_agent(&entry);
        }

        info!(
            agent_id = %agent_id,
            allowlist = ?allowlist,
            blocklist = ?blocklist,
            "Agent tool filters updated"
        );
        Ok(())
    }

    /// Set the weak self-reference for trigger dispatch.
    ///
    /// Must be called once after the kernel is wrapped in `Arc`.
    pub fn set_self_handle(self: &Arc<Self>) {
        let _ = self.self_handle.set(Arc::downgrade(self));
        self.arm_persisted_file_watchers();
    }

    /// Gracefully shutdown the kernel.
    ///
    /// This cleanly shuts down in-memory state but preserves persistent agent
    /// data so agents are restored on the next boot.
    pub fn shutdown(&self) {
        info!("Shutting down Captain kernel...");

        // Kill WhatsApp gateway child process if running
        if let Ok(guard) = self.whatsapp_gateway_pid.lock() {
            if let Some(pid) = *guard {
                info!("Stopping WhatsApp Web gateway (PID {pid})...");
                // Best-effort kill — don't block shutdown on failure
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                }
            }
        }

        self.supervisor.shutdown();

        // Update agent states to Suspended in persistent storage (not delete)
        for entry in self.registry.list() {
            let _ = self.registry.set_state(entry.id, AgentState::Suspended);
            // Re-save with Suspended state for clean resume on next boot
            if let Some(updated) = self.registry.get(entry.id) {
                let _ = self.memory.save_agent(&updated);
            }
        }

        info!(
            "Captain kernel shut down ({} agents preserved)",
            self.registry.list().len()
        );
    }

    /// Resolve the LLM driver for an agent.
    ///
    /// Always creates a fresh driver using current environment variables so that
    /// API keys saved via the dashboard (`set_provider_key`) take effect immediately
    /// without requiring a daemon restart. Uses the hot-reloaded default model
    /// override when available.
    /// If fallback models are configured, wraps the primary in a `FallbackDriver`.
    /// Look up a provider's base URL, checking runtime catalog first, then boot-time config.
    ///
    /// Custom providers added at runtime via the dashboard (`set_provider_url`) are
    /// stored in the model catalog but NOT in `self.config.provider_urls` (which is
    /// the boot-time snapshot). This helper checks both sources so that custom
    /// providers work immediately without a daemon restart.
    /// Resolve a credential by env var name using the vault → dotenv → env var chain.
    pub fn resolve_credential(&self, key: &str) -> Option<String> {
        self.credential_resolver
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve(key)
            .map(|z| z.to_string())
    }

    /// Store a credential in the vault (best-effort — falls through silently if no vault).
    pub fn store_credential(&self, key: &str, value: &str) {
        let mut resolver = self
            .credential_resolver
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Err(e) = resolver.store_in_vault(key, zeroize::Zeroizing::new(value.to_string())) {
            debug!("Vault store skipped for {key}: {e}");
        }
    }

    /// Remove a credential from the vault (best-effort — falls through silently if no vault).
    pub fn remove_credential(&self, key: &str) {
        let mut resolver = self
            .credential_resolver
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Err(e) = resolver.remove_from_vault(key) {
            debug!("Vault remove skipped for {key}: {e}");
        }
    }
}

#[async_trait]
impl KernelHandle for CaptainKernel {
    // Adapter layer only: domain behavior belongs in `kernel_handle_*` modules,
    // while this impl preserves the public runtime trait boundary.
    fn additional_workspace_roots(&self, caller_agent_id: Option<&str>) -> Vec<std::path::PathBuf> {
        let Some(id_str) = caller_agent_id else {
            return Vec::new();
        };
        let Ok(id) = id_str.parse::<AgentId>() else {
            return Vec::new();
        };
        let Some(entry) = self.registry.get(id) else {
            return Vec::new();
        };
        if entry.manifest.name != PRINCIPAL_AGENT_NAME {
            return Vec::new();
        }
        let mut roots = vec![self.config.home_dir.clone()];
        roots.extend(self.config.workspace.extra_paths.iter().cloned());
        roots
    }

    fn blocked_workspace_paths(&self) -> Vec<std::path::PathBuf> {
        default_blocked_workspace_paths(&self.config.home_dir)
    }

    fn add_workspace_path(&self, path: &std::path::Path) -> Result<(), String> {
        let canon = path
            .canonicalize()
            .map_err(|e| format!("Invalid path '{}': {e}", path.display()))?;
        for blocked in self.blocked_workspace_paths() {
            if let Ok(canon_blocked) = blocked.canonicalize() {
                if canon.starts_with(&canon_blocked) {
                    return Err(format!(
                        "Refused: '{}' is inside a protected zone",
                        canon.display()
                    ));
                }
            }
        }
        let config_path = self.config.home_dir.join("config.toml");
        let content =
            std::fs::read_to_string(&config_path).map_err(|e| format!("read config.toml: {e}"))?;
        let mut doc: toml_edit::DocumentMut = content
            .parse()
            .map_err(|e| format!("parse config.toml: {e}"))?;
        let workspace = doc["workspace"].or_insert(toml_edit::table());
        let arr = workspace["extra_paths"]
            .or_insert(toml_edit::value(toml_edit::Array::new()))
            .as_array_mut()
            .ok_or_else(|| "extra_paths is not an array".to_string())?;
        let canon_str = canon.display().to_string();
        if arr.iter().any(|v| v.as_str() == Some(canon_str.as_str())) {
            return Ok(());
        }
        arr.push(canon_str);
        std::fs::write(&config_path, doc.to_string())
            .map_err(|e| format!("write config.toml: {e}"))?;
        info!(path = %canon.display(), "workspace path added to config");
        Ok(())
    }

    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        self.handle_spawn_agent(manifest_toml, parent_id).await
    }

    async fn provision_spawned_agent_api(
        &self,
        agent_id: &str,
        request: captain_types::agent_api::AgentApiSpawnProvisionRequest,
    ) -> Result<captain_types::agent_api::AgentApiSpawnProvisionReport, String> {
        self.handle_provision_spawned_agent_api(agent_id, request)
            .await
    }

    async fn send_to_agent(&self, agent_id: &str, message: &str) -> Result<String, String> {
        self.handle_send_to_agent(agent_id, message).await
    }

    fn list_agents(&self) -> Vec<kernel_handle::AgentInfo> {
        self.handle_list_agents()
    }

    fn kill_agent(&self, agent_id: &str) -> Result<(), String> {
        self.handle_kill_agent(agent_id)
    }

    async fn create_manager(
        &self,
        name: &str,
        domain: &str,
        model: Option<&str>,
        budget_tokens: u64,
    ) -> Result<(String, String), String> {
        self.handle_create_manager(name, domain, model, budget_tokens)
            .await
    }

    fn list_managers(&self) -> Vec<serde_json::Value> {
        self.handle_list_managers()
    }

    async fn close_manager(&self, manager_id: &str) -> Result<u32, String> {
        self.handle_close_manager(manager_id).await
    }

    fn set_manager_mission(&self, manager_id: &str, mission: Option<&str>) -> Result<(), String> {
        self.handle_set_manager_mission(manager_id, mission)
    }

    fn configure_autoscale(
        &self,
        manager_id: &str,
        config: captain_types::agent::AutoScaleConfig,
    ) -> Result<(), String> {
        self.handle_configure_autoscale(manager_id, config)
    }

    fn fleet_metrics(&self, manager_id: &str) -> Result<serde_json::Value, String> {
        self.handle_fleet_metrics(manager_id)
    }

    fn check_agent_quota(&self, agent_name: &str) -> Result<(), String> {
        self.handle_check_agent_quota(agent_name)
    }

    fn agent_status_info(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        self.handle_agent_status_info(agent_id)
    }

    async fn agent_events(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.handle_agent_events(agent_id, limit).await
    }

    fn session_tool_call_summary(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        self.handle_session_tool_call_summary(agent_id, limit)
    }

    fn agent_capability_report(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        self.handle_agent_capability_report(agent_id)
    }

    async fn inject_system_message(&self, agent_id: &str, message: &str) -> Result<(), String> {
        self.handle_inject_system_message(agent_id, message).await
    }

    fn agent_is_busy(&self, agent_id: &str) -> bool {
        agent_id
            .parse::<AgentId>()
            .is_ok_and(|id| self.running_tasks.contains_key(&id))
    }

    async fn publish_typed_event(&self, payload: captain_types::event::EventPayload) {
        let event = captain_types::event::Event::new(
            AgentId::default(),
            captain_types::event::EventTarget::Broadcast,
            payload,
        );
        // Reuses the same evaluate-triggers-then-publish pipeline as custom
        // events (kernel_trigger_runtime.rs), so a future proactive trigger
        // keyed on tool_run activity would also fire correctly.
        let _ = CaptainKernel::publish_event(self, event).await;
    }

    async fn delegate_task(
        &self,
        agent_id: &str,
        task: &str,
        max_tokens: u64,
    ) -> Result<String, String> {
        self.handle_delegate_task(agent_id, task, max_tokens).await
    }

    fn memory_backend(&self) -> captain_types::config::MemoryBackend {
        self.handle_memory_backend()
    }

    fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        self.handle_memory_store(key, value)
    }

    fn memory_kv_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        self.handle_memory_kv_store(key, value)
    }

    fn memory_kv_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_memory_kv_recall(key)
    }

    fn memory_sanitize_active_context(
        &self,
        retractions: &[captain_runtime::memory_retractions::MemoryRetraction],
    ) -> Result<serde_json::Value, String> {
        self.handle_memory_sanitize_active_context(retractions)
    }

    fn memory_writes_conn(&self) -> Option<std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>> {
        self.handle_memory_writes_conn()
    }

    fn learning_review_list(&self, limit: usize) -> Result<serde_json::Value, String> {
        self.handle_learning_review_list(limit)
    }

    async fn learning_review_decide(
        &self,
        review_id: &str,
        approve: bool,
        decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.handle_learning_review_decide(review_id, approve, decided_by)
            .await
    }

    fn skill_proposal_list(&self, limit: usize) -> Result<serde_json::Value, String> {
        self.handle_skill_proposal_list(limit)
    }

    async fn skill_proposal_decide(
        &self,
        proposal_id: &str,
        approve: bool,
        decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.handle_skill_proposal_decide(proposal_id, approve, decided_by)
            .await
    }

    fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_memory_recall(key)
    }

    fn find_agents(&self, query: &str) -> Vec<kernel_handle::AgentInfo> {
        self.handle_find_agents(query)
    }

    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, String> {
        self.handle_task_post(title, description, assigned_to, created_by)
            .await
    }

    async fn task_claim(&self, agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_task_claim(agent_id).await
    }

    async fn task_complete(&self, task_id: &str, result: &str) -> Result<(), String> {
        self.handle_task_complete(task_id, result).await
    }

    async fn task_list(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        self.handle_task_list(status).await
    }

    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        self.handle_publish_event(event_type, payload).await
    }

    async fn knowledge_add_entity(
        &self,
        entity: captain_types::memory::Entity,
    ) -> Result<String, String> {
        self.handle_knowledge_add_entity(entity).await
    }

    async fn knowledge_add_relation(
        &self,
        relation: captain_types::memory::Relation,
    ) -> Result<String, String> {
        self.handle_knowledge_add_relation(relation).await
    }

    async fn knowledge_query(
        &self,
        pattern: captain_types::memory::GraphPattern,
    ) -> Result<Vec<captain_types::memory::GraphMatch>, String> {
        self.handle_knowledge_query(pattern).await
    }

    // --- Cron and file-change automation surface for the agent runtime ---
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        self.handle_cron_create(agent_id, job_json)
    }

    async fn cron_list(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        self.handle_cron_list(agent_id)
    }

    async fn cron_update(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        self.handle_cron_update(agent_id, job_json)
    }

    async fn cron_cancel(&self, job_id: &str) -> Result<(), String> {
        self.handle_cron_cancel(job_id)
    }

    // --- File-change trigger surface for the agent runtime --------------

    async fn file_trigger_register(
        &self,
        agent_id: &str,
        input: serde_json::Value,
    ) -> Result<String, String> {
        self.handle_file_trigger_register(agent_id, input)
    }

    async fn file_trigger_list(
        &self,
        agent_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.handle_file_trigger_list(agent_id)
    }

    async fn file_trigger_set_enabled(
        &self,
        trigger_id: &str,
        enabled: bool,
    ) -> Result<bool, String> {
        self.handle_file_trigger_set_enabled(trigger_id, enabled)
    }

    async fn file_trigger_remove(&self, trigger_id: &str) -> Result<bool, String> {
        self.handle_file_trigger_remove(trigger_id)
    }

    fn project_create(
        &self,
        name: &str,
        slug: &str,
        goal: &str,
        deadline: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        self.handle_project_create(name, slug, goal, deadline)
    }

    fn project_list(&self, include_archived: bool) -> Result<serde_json::Value, String> {
        self.handle_project_list(include_archived)
    }

    fn project_find_by_slug(&self, slug: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_project_find_by_slug(slug)
    }

    fn project_archive(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_project_archive(id)
    }

    fn project_delete(&self, id: &str) -> Result<bool, String> {
        self.handle_project_delete(id)
    }

    // --- Cross-session todos (v3.12g) ---------------------------------

    fn todo_create(&self, title: &str, body: &str) -> Result<serde_json::Value, String> {
        self.handle_todo_create(title, body)
    }

    fn todo_list(&self, filter: &str, limit: Option<u32>) -> Result<serde_json::Value, String> {
        self.handle_todo_list(filter, limit)
    }

    fn todo_complete(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_todo_complete(id)
    }

    fn todo_reopen(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_todo_reopen(id)
    }

    fn todo_delete(&self, id: &str) -> Result<bool, String> {
        self.handle_todo_delete(id)
    }

    fn project_task_create(
        &self,
        project_id: &str,
        title: &str,
        description: &str,
        parent_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.handle_project_task_create(project_id, title, description, parent_id)
    }

    fn project_task_list(&self, project_id: &str) -> Result<serde_json::Value, String> {
        self.handle_project_task_list(project_id)
    }

    fn project_task_update_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        self.handle_project_task_update_status(id, status)
    }

    fn milestone_create(
        &self,
        project_id: &str,
        name: &str,
        due_date: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        self.handle_milestone_create(project_id, name, due_date)
    }

    fn milestone_list(&self, project_id: &str) -> Result<serde_json::Value, String> {
        self.handle_milestone_list(project_id)
    }

    fn milestone_complete(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        self.handle_milestone_complete(id)
    }

    fn milestone_progress(&self, project_id: &str) -> Result<serde_json::Value, String> {
        self.handle_milestone_progress(project_id)
    }

    fn checkpoint_save(
        &self,
        project_id: &str,
        summary: &str,
        state: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.handle_checkpoint_save(project_id, summary, state, session_id)
    }

    fn project_resume(&self, slug: &str) -> Result<serde_json::Value, String> {
        self.handle_project_resume(slug)
    }

    fn active_project_set(&self, agent_id: &str, slug: Option<&str>) -> Result<(), String> {
        self.handle_active_project_set(agent_id, slug)
    }

    fn active_project_get(&self, agent_id: &str) -> Option<String> {
        self.handle_active_project_get(agent_id)
    }

    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, String> {
        self.handle_hand_list().await
    }

    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, String> {
        self.handle_hand_install(toml_content, skill_content).await
    }

    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        self.handle_hand_activate(hand_id, config).await
    }

    async fn hand_status(&self, hand_id: &str) -> Result<serde_json::Value, String> {
        self.handle_hand_status(hand_id).await
    }

    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), String> {
        self.handle_hand_deactivate(instance_id).await
    }

    fn requires_approval(&self, tool_name: &str) -> bool {
        self.handle_requires_approval(tool_name)
    }

    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
    ) -> Result<bool, String> {
        self.handle_request_approval(agent_id, tool_name, action_summary)
            .await
    }

    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        self.handle_list_a2a_agents()
    }

    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        self.handle_get_a2a_agent_url(name)
    }

    async fn get_channel_default_recipient(&self, channel: &str) -> Option<String> {
        self.handle_get_channel_default_recipient(channel).await
    }

    async fn get_channels_context(&self) -> Option<String> {
        self.handle_get_channels_context().await
    }

    fn get_telegram_topic(&self, agent_name: &str) -> Option<String> {
        self.handle_get_telegram_topic(agent_name)
    }

    fn set_telegram_topic(&self, agent_name: &str, topic_id: &str) {
        self.handle_set_telegram_topic(agent_name, topic_id);
    }

    async fn send_channel_message_from(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        caller_agent_name: Option<&str>,
    ) -> Result<String, String> {
        self.handle_send_channel_message_from(
            channel,
            recipient,
            message,
            thread_id,
            caller_agent_name,
        )
        .await
    }

    async fn send_channel_rich(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, String> {
        self.handle_send_channel_rich(channel, recipient, message, metadata)
            .await
    }

    async fn send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        self.handle_send_channel_media(
            channel, recipient, media_type, media_url, caption, filename, thread_id,
        )
        .await
    }

    async fn send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        self.handle_send_channel_file_data(channel, recipient, data, filename, mime_type, thread_id)
            .await
    }

    async fn send_channel_image_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        mime_type: &str,
        caption: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        self.handle_send_channel_image_data(channel, recipient, data, mime_type, caption, thread_id)
            .await
    }

    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[captain_types::capability::Capability],
    ) -> Result<(String, String), String> {
        self.handle_spawn_agent_checked(manifest_toml, parent_id, parent_caps)
            .await
    }

    fn consume_thoughts(&self, max: usize) -> Vec<serde_json::Value> {
        self.graph_memory
            .consume_queued_thoughts(max)
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "type": format!("{:?}", t.thought_type),
                    "score": t.activation_score,
                    "summary": t.summary,
                    "triggers": t.trigger_names,
                })
            })
            .collect()
    }

    fn recall_reflections(&self, agent_name: &str, limit: usize) -> String {
        self.graph_memory.recall_reflections(agent_name, limit)
    }

    fn update_user_state(&self, content: &str) -> String {
        let _ = self.graph_memory.update_user_state(content);
        self.graph_memory.user_state_prompt()
    }

    fn mood_prompt(&self) -> String {
        self.graph_memory.mood_prompt()
    }

    fn operational_awareness_prompt(&self, agent_name: &str) -> String {
        let _ = agent_name;
        let goals = self.goal_store.list();
        let projects = self.memory.project_list(false).unwrap_or_default();
        let project_signals =
            crate::operational_awareness::project_awareness_from_projects(&projects);
        crate::operational_awareness::build_operational_awareness_prompt(
            &self.graph_memory,
            &goals,
            &self.supervisor.health(),
            project_signals,
        )
    }

    fn temporal_prompt(&self) -> String {
        self.graph_memory.temporal_prompt()
    }

    fn shared_knowledge_prompt(&self) -> String {
        self.graph_memory.shared_knowledge_prompt()
    }

    fn record_temporal_action(&self, action: &str) {
        self.graph_memory.record_temporal_action(action);
    }

    fn curiosity_prompt(&self) -> String {
        let items = self.graph_memory.curiosity_scan();
        if items.is_empty() {
            return String::new();
        }
        let mut out = String::from("[CURIOSITY] Topics worth exploring:\n");
        for item in items.iter().take(3) {
            out.push_str(&format!("- {} ({})\n", item.topic, item.reason));
        }
        out
    }

    fn narration_prompt(&self) -> String {
        self.graph_memory.narration_prompt()
    }

    fn config_read(&self, path: &str) -> Result<Option<String>, String> {
        self.handle_config_read(path)
    }

    async fn config_write(&self, path: &str, value: &str) -> Result<(), String> {
        self.handle_config_write(path, value).await
    }

    async fn update_self_config(
        &self,
        agent_id: &str,
        config_json: &str,
    ) -> Result<String, String> {
        self.handle_update_self_config(agent_id, config_json).await
    }

    fn model_switch_plan(
        &self,
        agent_id: &str,
        model: &str,
        provider: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.handle_model_switch_plan(agent_id, model, provider)
    }

    fn model_switch_apply(
        &self,
        agent_id: &str,
        model: &str,
        provider: Option<&str>,
        session_strategy: &str,
    ) -> Result<serde_json::Value, String> {
        self.handle_model_switch_apply(agent_id, model, provider, session_strategy)
    }

    fn secret_read(&self, key: &str) -> Result<Option<String>, String> {
        self.handle_secret_read(key)
    }

    fn secret_write(&self, key: &str, value: &str) -> Result<(), String> {
        self.handle_secret_write(key, value)
    }

    fn home_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.config.home_dir.clone())
    }

    fn goal_create(&self, goal_json: &str) -> Result<String, String> {
        self.handle_goal_create(goal_json)
    }

    fn goal_list(&self) -> Result<String, String> {
        self.handle_goal_list()
    }

    fn goal_pause(&self, id: &str) -> Result<bool, String> {
        self.handle_goal_pause(id)
    }

    fn goal_resume(&self, id: &str) -> Result<bool, String> {
        self.handle_goal_resume(id)
    }

    fn goal_status(&self, id: &str) -> Result<String, String> {
        self.handle_goal_status(id)
    }

    fn goal_delete(&self, id: &str) -> Result<bool, String> {
        self.handle_goal_delete(id)
    }

    fn goal_record_check(
        &self,
        id: &str,
        ok: bool,
        output: &str,
        latency_ms: u64,
    ) -> Result<u32, String> {
        self.handle_goal_record_check(id, ok, output, latency_ms)
    }

    fn goal_mark_escalated(&self, id: &str) -> Result<bool, String> {
        self.handle_goal_mark_escalated(id)
    }

    fn instance_id(&self) -> String {
        self.instance_id.clone()
    }

    fn has_external_agent(&self, name: &str) -> bool {
        self.handle_has_external_agent(name)
    }

    fn list_external_agents(&self) -> Result<String, String> {
        self.handle_list_external_agents()
    }

    fn publish_memory_stored(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        wing: Option<&str>,
        room: Option<&str>,
        channel: Option<&str>,
        category: Option<&str>,
    ) {
        self.handle_publish_memory_stored(
            subject, predicate, object, source, wing, room, channel, category,
        );
    }

    fn publish_skill_refinement_queued(
        &self,
        refinement_id: &str,
        skill: &str,
        finding: &str,
        suggested_change: &str,
        risk: &str,
        source: &str,
        channel: Option<&str>,
    ) {
        self.handle_publish_skill_refinement_queued(
            refinement_id,
            skill,
            finding,
            suggested_change,
            risk,
            source,
            channel,
        );
    }

    fn goal_try_consume_llm_quota(&self, id: &str) -> bool {
        self.handle_goal_try_consume_llm_quota(id)
    }

    fn goal_list_suggestions(&self, id: &str) -> Result<String, String> {
        self.handle_goal_list_suggestions(id)
    }

    fn goal_add_suggestion_raw(&self, id: &str, suggestion_json: &str) -> Result<(), String> {
        self.handle_goal_add_suggestion_raw(id, suggestion_json)
    }

    fn goal_apply_suggestion(&self, id: &str, suggestion_id: &str) -> Result<bool, String> {
        self.handle_goal_apply_suggestion(id, suggestion_id)
    }

    fn goal_reject_suggestion(&self, id: &str, suggestion_id: &str) -> Result<bool, String> {
        self.handle_goal_reject_suggestion(id, suggestion_id)
    }

    /// R.3.2 — broadcast IntegrationConfigured on the event bus so the
    /// channel manager (in the API server) can hot-reload the affected
    /// adapter. Fire-and-forget: we don't block the tool runtime on bus
    /// delivery, and we don't fail the tool if the spawn pool is saturated.
    fn publish_integration_configured(&self, name: &str) {
        self.handle_publish_integration_configured(name);
    }

    async fn mcp_catalog_search(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        self.handle_mcp_catalog_search(query, limit).await
    }

    async fn mcp_integration_install(
        &self,
        id: &str,
        credentials: serde_json::Value,
        reload: bool,
    ) -> Result<serde_json::Value, String> {
        self.handle_mcp_integration_install(id, credentials, reload)
            .await
    }

    async fn mcp_status(&self) -> Result<serde_json::Value, String> {
        self.handle_mcp_status().await
    }
}
