//! Core kernel for the Captain Agent Operating System.
//!
//! The kernel manages agent lifecycles, memory, permissions, scheduling,
//! and inter-agent communication.

pub mod approval;
mod approval_rules;
#[cfg(test)]
mod approval_suggestion_manager_tests;
mod approval_suggestions;
pub mod auth;
pub mod auto_reply;
mod automation_session;
pub mod background;
pub mod builtin_crons;
pub mod capabilities;
pub mod capability_routing;
pub mod channel_delivery_retry;
pub mod channel_routing;
pub mod chat_broadcast;
pub mod codex_model_updates;
pub mod config;
pub mod config_reload;
#[cfg(test)]
mod config_reload_tests;
pub mod config_watcher;
pub mod cron;
pub mod cron_agent_turn;
pub mod cron_delivery_queue;
pub mod delivery_reliability;
pub mod ephemeral_agents;
pub mod error;
pub mod event_bus;
pub mod fleet_autoscale;
pub mod gmail_persistence;
pub mod goals;
pub mod graph_memory;
pub mod graph_seed;
pub mod heartbeat;
pub mod hub_node_service;
pub mod hub_pairing_service;
pub mod kernel;
pub mod metering;
pub mod milestone_alerts;
pub mod model_switch;
pub mod operational_awareness;
pub mod outbound_deliveries;
pub mod pairing;
mod provider_quota_monitor;
pub mod registry;
pub mod release_updates;
pub mod scheduler;
pub mod supervisor;
pub mod tool_rag;
pub mod triggers;
pub mod whatsapp_gateway;
pub mod wizard;
pub mod workflow;

pub use kernel::default_blocked_workspace_paths;
pub use kernel::shared_memory_agent_id;
pub use kernel::BudgetConfigUpdateError;
pub use kernel::CaptainKernel;
pub use kernel::CompactionProgressSink;
pub use kernel::DeliveryTracker;
pub use kernel::ToolRunOperatorSurface;
